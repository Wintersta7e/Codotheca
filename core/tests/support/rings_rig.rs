//! The fixture builder the ring tests share, over a fully migrated `TempIndex`. Every row it plants
//! is an input to a reader under test; no ring test builds a `RingsState` by hand.

use std::cell::Cell;

use codotheca_core::protocol::{LocationId, ProjectId};
use codotheca_core::testing::zone::FixedZone;
use codotheca_core::testing::TempIndex;
use rusqlite::Connection;

/// The zone most ring tests read in: UTC, for ever.
pub(crate) const UTC: FixedZone = FixedZone(0);

/// A project's flags as a ring reads them; the default is a live, deep, non-Reference project.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Flags {
    pub(crate) shallow: bool,
    pub(crate) reference: bool,
    pub(crate) archived: bool,
    pub(crate) merged_into: Option<ProjectId>,
    pub(crate) removed_at: Option<i64>,
}

/// A library the ring tests plant rows into.
pub(crate) struct RingsRig {
    pub(crate) tmp: TempIndex,
    next: Cell<i64>,
}

impl RingsRig {
    pub(crate) fn new() -> Self {
        Self {
            tmp: TempIndex::new(),
            next: Cell::new(1),
        }
    }

    /// A library whose index has finished a scan, which the rhythm settlement waits for (§39.7).
    pub(crate) fn scanned() -> Self {
        let rig = Self::new();
        scan_run(rig.conn(), false);
        rig
    }

    pub(crate) const fn conn(&self) -> &Connection {
        self.tmp.index().conn()
    }

    /// A number no earlier call answered, for names and keys that must not collide.
    fn fresh(&self) -> i64 {
        let n = self.next.get();
        self.next.set(n + 1);
        n
    }

    pub(crate) fn project(&self, flags: Flags) -> ProjectId {
        let n = self.fresh();
        self.conn()
            .execute(
                "INSERT INTO project (name, seed_basename, lineage_key, is_shallow, is_reference,
                                      is_archived, merged_into, removed_at, created_at,
                                      updated_at)
                 VALUES (?1, ?1, ?1, ?2, ?3, ?4, ?5, ?6, 1, 1)",
                rusqlite::params![
                    format!("p{n}"),
                    flags.shallow,
                    flags.reference,
                    flags.archived,
                    flags.merged_into.map(|p| p.0),
                    flags.removed_at,
                ],
            )
            .unwrap();
        ProjectId(self.conn().last_insert_rowid())
    }

    pub(crate) fn location(
        &self,
        project: ProjectId,
        presence: &str,
        removed_at: Option<i64>,
    ) -> LocationId {
        let path = format!("/rig/copy-{}", self.fresh());
        self.conn()
            .execute(
                "INSERT INTO location (project_id, kind, path_bytes, path_key, path_display,
                                       store_key, presence, repo_kind, removed_at)
                 VALUES (?1, 'linux', ?2, ?2, ?3, 'store', ?4, 'worktree', ?5)",
                rusqlite::params![project.0, path.as_bytes(), path, presence, removed_at],
            )
            .unwrap();
        LocationId(self.conn().last_insert_rowid())
    }

    /// A `commit_day` row for `day`, stamped at the day's midnight as J4 stamps one.
    pub(crate) fn commit_day(&self, project: ProjectId, day: i64) {
        self.conn()
            .execute(
                "INSERT INTO xp_events (ts, project_id, subject_key, kind, dedupe_key, track, meta)
                 VALUES (?1, ?2, ?3, 'commit_day', ?4, 'git', '{}')",
                rusqlite::params![
                    day * 86_400,
                    project.0,
                    format!("subject-{}", project.0),
                    format!("commit_day:{}:{day}", project.0),
                ],
            )
            .unwrap();
    }

    /// A `debt_day` row at `ts` in `offset_min`, whose receipt names `sources`.
    pub(crate) fn debt_day(&self, project: ProjectId, ts: i64, offset_min: i32, sources: &[&str]) {
        let meta = serde_json::json!({ "sources": sources }).to_string();
        self.conn()
            .execute(
                "INSERT INTO xp_events
                    (ts, tz_offset_min, project_id, subject_key, kind, dedupe_key, track, meta)
                 VALUES (?1, ?2, ?3, ?4, 'debt_day', ?5, 'session', ?6)",
                rusqlite::params![
                    ts,
                    offset_min,
                    project.0,
                    format!("subject-{}", project.0),
                    format!("debt_day:{}:{ts}", project.0),
                    meta,
                ],
            )
            .unwrap();
    }

    /// A closed segment of `credited` seconds from `start`, alone in its own session; `signalled`
    /// is the stored flag, `None` a row written before the column.
    pub(crate) fn segment(
        &self,
        project: ProjectId,
        start: i64,
        credited: i64,
        signalled: Option<bool>,
    ) -> i64 {
        let conn = self.conn();
        conn.execute(
            "INSERT INTO session (project_id, started_at, ended_at, credited_seconds, close_reason)
             VALUES (?1, ?2, ?3, ?4, 'stop')",
            rusqlite::params![project.0, start, start + credited, credited],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO session_segment
                (session_id, started_at, ended_at, credited_seconds, closed_by, signalled)
             VALUES (?1, ?2, ?3, ?4, 'session_end', ?5)",
            rusqlite::params![
                conn.last_insert_rowid(),
                start,
                start + credited,
                credited,
                signalled
            ],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    /// Marks `project` enrolled (§30.5): the user has acknowledged it.
    pub(crate) fn enrol(&self, project: ProjectId) {
        self.conn()
            .execute(
                "UPDATE project SET acknowledged_at = 1 WHERE id = ?1",
                [project.0],
            )
            .unwrap();
    }

    /// `project`'s `debt_sweep` row for `source`, with `outcome`; a `complete` or `partial` row
    /// carries a zero item count, as the column's `CHECK` requires.
    pub(crate) fn sweep(&self, project: ProjectId, source: &str, outcome: &str) {
        let count = matches!(outcome, "complete" | "partial").then_some(0_i64);
        self.conn()
            .execute(
                "INSERT INTO debt_sweep (project_id, source, outcome, item_count, observed_at)
                 VALUES (?1, ?2, ?3, ?4, 1)
                 ON CONFLICT (project_id, source) DO UPDATE
                   SET outcome = excluded.outcome, item_count = excluded.item_count",
                rusqlite::params![project.0, source, outcome, count],
            )
            .unwrap();
    }

    /// One `debt_item` of `project` from `source`, in `state` and `scoring`.
    pub(crate) fn item(&self, project: ProjectId, source: &str, state: &str, scoring: &str) {
        self.conn()
            .execute(
                "INSERT INTO debt_item (project_id, subject_key, source, fingerprint, state,
                                        scoring, first_seen_at, last_seen_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, 1)",
                rusqlite::params![
                    project.0,
                    format!("subject-{}", project.0),
                    source,
                    format!("f{}", self.fresh()),
                    state,
                    scoring,
                ],
            )
            .unwrap();
    }

    pub(crate) fn meta(&self, k: &str, v: &str) {
        self.conn()
            .execute(
                "INSERT INTO app_meta (k, v) VALUES (?1, ?2)
                 ON CONFLICT (k) DO UPDATE SET v = excluded.v",
                [k, v],
            )
            .unwrap();
    }
}

/// A `scan_run` row that has ended, `cancelled` or not: the one shape the settlement reads to know
/// the index has read the disk since it was built.
pub(crate) fn scan_run(conn: &Connection, cancelled: bool) {
    conn.execute(
        "INSERT INTO scan_run (generation, started_at, ended_at, mode, roots_json, cancelled)
         VALUES (1, 1, 2, 'full', '[]', ?1)",
        [cancelled],
    )
    .unwrap();
}
