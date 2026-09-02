//! **R40** — the production [`ScanStore`], over plan 04's index.
//!
//! Every scan write goes through the trait, and for three plans the only implementation anywhere
//! was the in-memory double. That is this project's most repeated defect — a seam introduced for
//! testability gets its fake first and the real one is assumed to be somebody's next step; it
//! compiles, the tests pass against the fake, and nothing fails until assembly. R40 assigns the
//! real one to the plan that declares the trait.
//!
//! **The index is owned behind a `Mutex`, shared as an `Arc`.** `rusqlite::Connection` is `Send`
//! but not `Sync` (R39), so `Arc<Index>` is not `Send` and the run's worker thread could not hold
//! one. R39 leaves the choice between an owning `Mutex<Index>` and a channel back to the loop
//! thread to whoever resolves it; this takes the mutex, and takes it as a *shared* `Arc` so the
//! command loop keeps the same handle rather than the scanner claiming the only connection.
//!
//! **No method opens an explicit transaction, so none needs a `TxGuard`.** Each is one statement
//! in autocommit. A presence sweep wrapped in one transaction would hold the write lock — and
//! block every protocol frame, since `FrameSink::send` refuses while a guard is live — for the
//! length of a full library pass, and a sweep interrupted halfway is self-healing: the next run
//! recomputes every row from its generation.

use std::sync::{Arc, Mutex, PoisonError};

use crate::index::Index;
use crate::scan::presence::{
    LocationPresenceRow, Presence, ScanRootRow, ScanRunFinish, ScanRunRow, ScanRunStart, ScanStore,
    ScanStoreError,
};
use crate::scan::ScanProblem;

/// The scanner's view of the real index.
#[derive(Debug)]
pub struct SqliteScanStore {
    index: Arc<Mutex<Index>>,
}

impl SqliteScanStore {
    #[must_use]
    pub fn new(index: Arc<Mutex<Index>>) -> Self {
        Self { index }
    }

    /// A poisoned lock is recovered rather than propagated: the index itself is a SQLite
    /// connection whose state is intact after a panic elsewhere, and refusing every subsequent
    /// scan write would turn one unrelated panic into a permanently broken library.
    fn with_conn<T>(
        &self,
        body: impl FnOnce(&rusqlite::Connection) -> rusqlite::Result<T>,
    ) -> Result<T, ScanStoreError> {
        let guard = self.index.lock().unwrap_or_else(PoisonError::into_inner);
        body(guard.conn()).map_err(|e| ScanStoreError::new(e.to_string()))
    }
}

/// One `location` row as SQLite hands it over, before `presence` is parsed. Aliased because a
/// seven-field tuple trips `clippy::type_complexity`, which this crate denies.
type RawLocationRow = (i64, i64, Vec<u8>, Vec<u8>, String, i64, String);

/// `u64` counters into the `INTEGER … CHECK (>= 0)` columns. Saturating rather than wrapping: a
/// negative walk count would violate the constraint and abort the run's own completion row.
fn clamp(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

impl ScanStore for SqliteScanStore {
    /// One above the highest generation any run or any location has ever carried.
    ///
    /// Both tables are consulted, not just `scan_run`: a generation already stamped on a
    /// `location` row must never be reissued, or the second run to use it marks the first run's
    /// locations `missing` while they are present (§4.6).
    fn next_generation(&self) -> Result<i64, ScanStoreError> {
        self.with_conn(|conn| {
            let highest: i64 = conn.query_row(
                "SELECT MAX(g) FROM (
                   SELECT COALESCE(MAX(generation), 0) AS g FROM scan_run
                   UNION ALL
                   SELECT COALESCE(MAX(scan_generation), 0) FROM location
                 )",
                [],
                |row| row.get(0),
            )?;
            Ok(highest.saturating_add(1))
        })
    }

    fn scan_roots(&self) -> Result<Vec<ScanRootRow>, ScanStoreError> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, kind, distro, path_bytes, path_key, enabled, descend_into_repos
                   FROM scan_root ORDER BY id",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok(ScanRootRow {
                    root_id: row.get(0)?,
                    kind: row.get(1)?,
                    distro: row.get(2)?,
                    path_bytes: row.get(3)?,
                    path_key: row.get(4)?,
                    enabled: row.get::<_, i64>(5)? != 0,
                    descend_into_repos: row.get::<_, i64>(6)? != 0,
                })
            })?;
            rows.collect()
        })
    }

    fn indexed_project_count(&self) -> Result<u64, ScanStoreError> {
        self.with_conn(|conn| {
            let n: i64 = conn.query_row("SELECT COUNT(*) FROM project", [], |row| row.get(0))?;
            Ok(u64::try_from(n).unwrap_or(0))
        })
    }

    fn locations_for_presence(&self) -> Result<Vec<LocationPresenceRow>, ScanStoreError> {
        let rows: Vec<RawLocationRow> = self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, project_id, path_bytes, path_key, store_key, scan_generation,
                            presence
                       FROM location ORDER BY id",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                ))
            })?;
            rows.collect()
        })?;

        rows.into_iter()
            .map(
                |(location_id, project_id, path_bytes, path_key, store_key, generation, raw)| {
                    // An unreadable presence is a corrupt row, not a reason to guess: defaulting
                    // to `present` here would claim currency the app does not have (§6).
                    let presence = Presence::parse(&raw).ok_or_else(|| {
                        ScanStoreError::new(format!(
                            "location {location_id} has presence '{raw}', which is not one of the four"
                        ))
                    })?;
                    Ok(LocationPresenceRow {
                        location_id,
                        project_id,
                        path_bytes,
                        path_key,
                        store_key,
                        scan_generation: generation,
                        presence,
                    })
                },
            )
            .collect()
    }

    fn set_presence(&self, location_id: i64, presence: Presence) -> Result<(), ScanStoreError> {
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE location SET presence = ?1 WHERE id = ?2",
                rusqlite::params![presence.as_str(), location_id],
            )?;
            Ok(())
        })
    }

    fn begin_scan_run(&self, start: &ScanRunStart) -> Result<i64, ScanStoreError> {
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO scan_run (generation, started_at, mode, roots_json)
                 VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![
                    start.generation,
                    start.started_at,
                    start.mode,
                    start.roots_json
                ],
            )?;
            Ok(conn.last_insert_rowid())
        })
    }

    fn finish_scan_run(
        &self,
        scan_run_id: i64,
        finish: &ScanRunFinish,
    ) -> Result<(), ScanStoreError> {
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE scan_run
                    SET ended_at = ?1, walked_dirs = ?2, found_repos = ?3, cancelled = ?4
                  WHERE id = ?5",
                rusqlite::params![
                    finish.ended_at,
                    clamp(finish.walked_dirs),
                    clamp(finish.found_repos),
                    i64::from(finish.cancelled),
                    scan_run_id
                ],
            )?;
            Ok(())
        })
    }

    fn record_problem(
        &self,
        scan_run_id: i64,
        problem: &ScanProblem,
    ) -> Result<(), ScanStoreError> {
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO scan_problem (scan_run_id, kind, path_display, detail, count)
                 VALUES (?1, ?2, ?3, ?4, 1)",
                rusqlite::params![
                    scan_run_id,
                    problem.kind.as_str(),
                    problem.path_display,
                    problem.detail
                ],
            )?;
            Ok(())
        })
    }

    fn latest_scan_run(&self) -> Result<Option<ScanRunRow>, ScanStoreError> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, generation, started_at, ended_at, mode, walked_dirs, found_repos,
                        cancelled
                   FROM scan_run ORDER BY id DESC LIMIT 1",
            )?;
            let mut rows = stmt.query([])?;
            match rows.next()? {
                None => Ok(None),
                Some(row) => Ok(Some(ScanRunRow {
                    id: row.get(0)?,
                    generation: row.get(1)?,
                    started_at: row.get(2)?,
                    ended_at: row.get(3)?,
                    mode: row.get(4)?,
                    walked_dirs: row.get(5)?,
                    found_repos: row.get(6)?,
                    cancelled: row.get::<_, i64>(7)? != 0,
                })),
            }
        })
    }

    fn problem_count(&self, scan_run_id: i64) -> Result<u64, ScanStoreError> {
        self.with_conn(|conn| {
            let n: i64 = conn.query_row(
                "SELECT COUNT(*) FROM scan_problem WHERE scan_run_id = ?1",
                [scan_run_id],
                |row| row.get(0),
            )?;
            Ok(u64::try_from(n).unwrap_or(0))
        })
    }

    fn ambiguous_lineage_count(&self) -> Result<u64, ScanStoreError> {
        self.with_conn(|conn| {
            let n: i64 = conn.query_row(
                "SELECT COUNT(*) FROM project WHERE ambiguous_lineage = 1",
                [],
                |row| row.get(0),
            )?;
            Ok(u64::try_from(n).unwrap_or(0))
        })
    }
}
