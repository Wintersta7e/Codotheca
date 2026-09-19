//! Reads and writes for §32's nine tables.

use rusqlite::{Connection, Transaction};

use crate::advisories::lockfiles::{LockfileHit, LockfileWalk};
use crate::advisories::{eco_slug, read_state_slug};
use crate::index::IndexError;
use crate::protocol::{DependencyReadState, Ecosystem, ProjectId};
use crate::provider::PackageVersion;

/// Drop everything one project's last lockfile read wrote.
///
/// A re-read **replaces**: a lockfile that has been deleted must not leave its triples behind,
/// which would be a dependency this project no longer resolves, dated as if it did.
///
/// # Errors
/// Fails when SQLite refuses the delete.
pub fn clear_project_read(tx: &Transaction<'_>, project: ProjectId) -> Result<(), IndexError> {
    for table in [
        "project_dependency",
        "project_lockfile",
        "project_dependency_scan",
    ] {
        tx.execute(
            &format!("DELETE FROM {table} WHERE project_id = ?1"),
            [project.0],
        )?;
    }
    Ok(())
}

/// One `project_lockfile` row: **the file, and whether it could be read.**
///
/// # Errors
/// Fails when SQLite refuses the write.
pub fn write_lockfile_row(
    tx: &Transaction<'_>,
    project: ProjectId,
    hit: &LockfileHit,
    state: DependencyReadState,
    now: i64,
) -> Result<(), IndexError> {
    let ecosystem = eco_slug(hit.ecosystem);
    let read_state = read_state_slug(state);
    tx.execute(
        "INSERT INTO project_lockfile
           (project_id, source_path, ecosystem, read_state, size_bytes, observed_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(project_id, source_path) DO UPDATE SET
           ecosystem = excluded.ecosystem, read_state = excluded.read_state,
           size_bytes = excluded.size_bytes, observed_at = excluded.observed_at",
        rusqlite::params![
            project.0,
            hit.source_path,
            ecosystem,
            read_state,
            i64::try_from(hit.size_bytes).unwrap_or(i64::MAX),
            now
        ],
    )?;
    Ok(())
}

/// The record that **the walk ran** — the row whose *absence* is *the scan has not run*.
///
/// §32.8's first verdict row is lit only because *ran and found no lockfile* is distinguishable
/// from *has not run*; collapsing them makes every unscanned project claim to have no dependencies.
///
/// # Errors
/// Fails when SQLite refuses the write.
pub fn write_scan_row(
    tx: &Transaction<'_>,
    project: ProjectId,
    walk: &LockfileWalk,
    now: i64,
) -> Result<(), IndexError> {
    tx.execute(
        "INSERT INTO project_dependency_scan
           (project_id, observed_at, files_matched, dirs_entered, unresolved_manifests, complete)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(project_id) DO UPDATE SET
           observed_at = excluded.observed_at, files_matched = excluded.files_matched,
           dirs_entered = excluded.dirs_entered,
           unresolved_manifests = excluded.unresolved_manifests,
           complete = excluded.complete",
        rusqlite::params![
            project.0,
            now,
            i64::try_from(walk.files.len()).unwrap_or(i64::MAX),
            i64::try_from(walk.dirs_entered).unwrap_or(i64::MAX),
            i64::try_from(walk.unresolved_manifests).unwrap_or(i64::MAX),
            i64::from(walk.complete)
        ],
    )?;
    Ok(())
}

/// Write one file's triples. Returns the **number of rows written**, which is not the length of
/// `pairs`: one lockfile routinely resolves a package at a version another file already wrote.
///
/// # Errors
/// Fails when SQLite refuses the write.
pub fn write_triples(
    tx: &Transaction<'_>,
    project: ProjectId,
    source_path: &str,
    eco: Ecosystem,
    pairs: &[PackageVersion],
    now: i64,
) -> Result<usize, IndexError> {
    let ecosystem = eco_slug(eco);
    let mut written = 0usize;
    for item in pairs {
        // `DO NOTHING`, not `DO UPDATE`: the first file to resolve a triple is the one whose path
        // is recorded, and `source_path` is diagnostic — rewriting it on every duplicate would
        // make the column report whichever file the walk happened to reach last.
        written += tx.execute(
            "INSERT INTO project_dependency
                   (project_id, ecosystem, package_name, version, source_path, observed_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(project_id, ecosystem, package_name, version) DO NOTHING",
            rusqlite::params![
                project.0,
                ecosystem,
                item.name,
                item.version,
                source_path,
                now
            ],
        )?;
    }
    Ok(written)
}

/// The `x-ratelimit-resource` the sweep's own last **settled** response named, if one has.
///
/// **This is what the pre-issue budget read is keyed by**, and the reason is §32.3's second
/// defect. `DEFAULT_RESOURCE` is a process-wide constant read for every task before issuing. It is
/// right for the six authenticated REST reads phase 2 ships, but for this task it is an assumption:
/// nothing in this tree recorded which pool the advisories endpoint answers from.
///
/// If the endpoint names a resource other than `core`, [`crate::sync::budget::mirror`] writes
/// `(NULL, '<that name>')` while the pre-issue read looks up `(NULL, 'core')` — a pool the sweep
/// never writes. `may_spend` then answers `Unknown` for ever, and **`Unknown` spends**, so every
/// request issues with no brake at all until the source refuses. For an on-demand task that costs
/// one request and self-corrects; for a scheduled sweep it is the behaviour that gets the IP
/// rate-limited for every other unauthenticated call the app makes.
///
/// `None` until one response has named one, where the verdict is `Unknown`, which spends — one
/// request, self-correcting. **A response carrying no `x-ratelimit-resource` at all is not
/// mirrored and is not given a synthetic key here either**: with no resource header there is no
/// pool and no brake, and that is a property of the endpoint rather than a bug to work around.
///
/// # Errors
/// Fails when SQLite cannot be read.
pub fn last_settled_resource(conn: &Connection) -> Result<Option<String>, IndexError> {
    let found = conn
        .query_row(
            "SELECT resource FROM advisory_sweep
              WHERE settled_at IS NOT NULL AND resource IS NOT NULL
              ORDER BY settled_at DESC, id DESC
              LIMIT 1",
            [],
            |row| row.get::<_, String>(0),
        )
        .ok();
    Ok(found)
}

/// How many candidate triples one batch query reads before the byte budget trims it.
///
/// A ceiling on the *query*, not on the request: the request's own bounds are
/// `ADVISORY_BATCH_CAP` and `ADVISORY_AFFECTS_BYTE_CAP`, and this is simply how many rows are
/// fetched for those two to choose from. Generous, because a batch that trims to nothing because
/// every candidate shared one package name would stall the sweep.
pub const ADVISORY_SWEEP_BATCH: usize = 1024;

/// The open sweep's id, opening one if none is in flight.
///
/// **At most one sweep is open at a time**, found by its own `settled_at IS NULL`, so a restart
/// resumes the sweep it interrupted rather than starting a second one beside it.
///
/// # Errors
/// Fails when SQLite refuses.
pub fn open_sweep(tx: &Transaction<'_>, now: i64) -> Result<i64, IndexError> {
    if let Ok(id) = tx.query_row(
        "SELECT id FROM advisory_sweep WHERE settled_at IS NULL ORDER BY id DESC LIMIT 1",
        [],
        |row| row.get::<_, i64>(0),
    ) {
        return Ok(id);
    }
    tx.execute(
        "INSERT INTO advisory_sweep (started_at, complete) VALUES (?1, 0)",
        [now],
    )?;
    Ok(tx.last_insert_rowid())
}

/// Settle the sweep row. `complete` is what §32.8's verdict and §31's `deps` predicate read.
///
/// # Errors
/// Fails when SQLite refuses.
pub fn close_sweep(
    tx: &Transaction<'_>,
    sweep_id: i64,
    now: i64,
    outcome: &str,
    complete: bool,
) -> Result<(), IndexError> {
    tx.execute(
        "UPDATE advisory_sweep SET settled_at = ?2, outcome = ?3, complete = ?4 WHERE id = ?1",
        rusqlite::params![sweep_id, now, outcome, i64::from(complete)],
    )?;
    Ok(())
}

/// Record the `x-ratelimit-resource` this sweep's response named.
///
/// **Only when one was named.** A response carrying no resource header is not mirrored and is not
/// given a synthetic key here either: with no resource header there is no pool and no brake, and
/// that is a property of the endpoint rather than a bug to work around.
///
/// # Errors
/// Fails when SQLite refuses.
pub fn note_sweep_resource(
    tx: &Transaction<'_>,
    sweep_id: i64,
    resource: &str,
) -> Result<(), IndexError> {
    tx.execute(
        "UPDATE advisory_sweep SET resource = ?2 WHERE id = ?1",
        rusqlite::params![sweep_id, resource],
    )?;
    Ok(())
}

/// The distinct triples of one ecosystem that **this sweep has not asked about yet**.
///
/// Deduplicated library-wide, which is what makes the sweep's cost independent of project count:
/// a package every project depends on is one request key, not one per project.
///
/// # Errors
/// Fails when SQLite cannot be read.
pub fn unanswered_triples(
    tx: &Transaction<'_>,
    eco: Ecosystem,
    sweep_id: i64,
    limit: usize,
) -> Result<Vec<PackageVersion>, IndexError> {
    let ecosystem = eco_slug(eco);
    let mut stmt = tx.prepare(
        "SELECT DISTINCT d.package_name, d.version
           FROM project_dependency d
          WHERE d.ecosystem = ?1
            AND NOT EXISTS (
                  SELECT 1 FROM advisory_triple t
                   WHERE t.ecosystem = d.ecosystem
                     AND t.package_name = d.package_name
                     AND t.version = d.version
                     AND t.sweep_id = ?2)
          ORDER BY d.package_name, d.version
          LIMIT ?3",
    )?;
    let rows = stmt
        .query_map(
            rusqlite::params![
                ecosystem,
                sweep_id,
                i64::try_from(limit).unwrap_or(i64::MAX)
            ],
            |row| {
                Ok(PackageVersion {
                    name: row.get(0)?,
                    version: row.get(1)?,
                })
            },
        )?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// The distinct triples the whole library holds, for a diagnostic count.
///
/// # Errors
/// Fails when SQLite cannot be read.
pub fn distinct_triples(conn: &Connection) -> Result<Vec<(Ecosystem, PackageVersion)>, IndexError> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT ecosystem, package_name, version FROM project_dependency
          ORDER BY ecosystem, package_name, version",
    )?;
    let rows = stmt
        .query_map([], |row| {
            let raw: String = row.get(0)?;
            Ok((
                raw,
                PackageVersion {
                    name: row.get(1)?,
                    version: row.get(2)?,
                },
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows
        .into_iter()
        .filter_map(|(raw, item)| {
            Ecosystem::ALL
                .into_iter()
                .find(|e| eco_slug(*e) == raw)
                .map(|eco| (eco, item))
        })
        .collect())
}

/// Fold one answered page into the cache. Returns the **number of match rows written**.
///
/// One `advisory_triple` row per **asked** triple, answered or not — and the row is what tells
/// *this triple has no advisory* from *nobody has asked about this triple*. Both are zero match
/// rows, and only a row here tells them apart: **NEVER RENDER UNKNOWN AS ZERO, at the storage
/// layer.**
///
/// `complete` is false while the answer is still paginating, and the asked triples are then
/// written **unanswered** — half an answer is not an answer.
///
/// # Errors
/// Fails when SQLite refuses a write.
pub fn fold_response(
    tx: &Transaction<'_>,
    sweep_id: i64,
    eco: Ecosystem,
    asked: &[PackageVersion],
    page: &[crate::provider::AdvisoryPayload],
    complete: bool,
    now: i64,
) -> Result<usize, IndexError> {
    let ecosystem = eco_slug(eco);
    for item in asked {
        tx.execute(
            "INSERT INTO advisory_triple
               (ecosystem, package_name, version, sweep_id, observed_at, answered)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(ecosystem, package_name, version) DO UPDATE SET
               sweep_id = excluded.sweep_id, observed_at = excluded.observed_at,
               answered = excluded.answered",
            rusqlite::params![
                ecosystem,
                item.name,
                item.version,
                sweep_id,
                now,
                i64::from(complete)
            ],
        )?;
    }

    let mut matched = 0usize;
    for advisory in page {
        tx.execute(
            "INSERT INTO advisory (advisory_id, severity, withdrawn_at, summary, url, observed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(advisory_id) DO UPDATE SET
               severity = excluded.severity, withdrawn_at = excluded.withdrawn_at,
               summary = excluded.summary, url = excluded.url,
               observed_at = excluded.observed_at",
            rusqlite::params![
                advisory.advisory_id,
                advisory.severity,
                advisory.withdrawn_at,
                advisory.summary,
                advisory.url,
                now
            ],
        )?;
        for cve in &advisory.cve_ids {
            tx.execute(
                "INSERT INTO advisory_cve (advisory_id, cve_id) VALUES (?1, ?2)
                 ON CONFLICT(advisory_id, cve_id) DO NOTHING",
                rusqlite::params![advisory.advisory_id, cve],
            )?;
        }
        for affected in &advisory.affects {
            // The version that matched is the one **this request asked about**: the response names
            // the affected package and the vulnerable range, never the pair. A name the batch did
            // not ask about is an advisory the endpoint volunteered and is not this project's.
            let Some(item) = asked.iter().find(|a| a.name == affected.name) else {
                continue;
            };
            matched += tx.execute(
                "INSERT INTO advisory_match
                       (ecosystem, package_name, version, advisory_id, fix_available, fixed_version)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                     ON CONFLICT(ecosystem, package_name, version, advisory_id) DO UPDATE SET
                       fix_available = excluded.fix_available,
                       fixed_version = excluded.fixed_version",
                rusqlite::params![
                    ecosystem,
                    item.name,
                    item.version,
                    advisory.advisory_id,
                    i64::from(affected.fix_available),
                    affected.fixed_version
                ],
            )?;
        }
    }
    Ok(matched)
}
