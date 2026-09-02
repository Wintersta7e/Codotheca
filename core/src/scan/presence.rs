//! §4.6 — presence, and the generation rule v1 omitted.
//!
//! Without it nothing ever left `present`, `last_seen_at` was written and never read, and
//! criterion 4 could not pass. Note what is absent from [`ScanStore`]: there is no method that
//! removes anything. §17 admits no destructive operation in phase 1, and a project whose every
//! location is offline is offline — **never deleted**.
//!
//! | Condition | Result |
//! |---|---|
//! | not under any **enabled** root, or the exclusion list now covers it | `unscanned` |
//! | `location.scan_generation` equals this run's generation | `present` |
//! | otherwise, and its **store was present** this generation | `missing` |
//! | otherwise — its store was **absent** | `offline`; the condition freezes (§5.4) |

use std::collections::{BTreeMap, BTreeSet};

use crate::paths::{is_under, path_from_bytes};
use crate::scan::skiplist::SkipList;

/// R31: `Presence` is declared in `protocol/schema/protocol.json` and generated into
/// `crate::protocol`; a second hand-written copy compiles and then drifts from the wire form.
/// Re-exported so `scan::presence::Presence` still names it and every caller is unchanged.
///
/// `as_str` and `parse` stay here as an **inherent impl on the generated type** — legal because
/// both modules are in this crate. The generated enum carries serde attributes and no methods,
/// and these two are how a presence reaches and returns from SQLite (plans 08, 10b and 11c call
/// them). Deleting them with the enum would have been a silent loss.
pub use crate::protocol::Presence;

impl Presence {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Present => "present",
            Self::Offline => "offline",
            Self::Missing => "missing",
            Self::Unscanned => "unscanned",
        }
    }

    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "present" => Some(Self::Present),
            "offline" => Some(Self::Offline),
            "missing" => Some(Self::Missing),
            "unscanned" => Some(Self::Unscanned),
            _ => None,
        }
    }
}

/// One `scan_root` row (§1.9).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanRootRow {
    pub root_id: i64,
    pub kind: String,
    pub distro: String,
    pub path_bytes: Vec<u8>,
    pub path_key: Vec<u8>,
    pub enabled: bool,
    pub descend_into_repos: bool,
}

/// The columns of `location` (§1.3) presence classification reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocationPresenceRow {
    pub location_id: i64,
    pub project_id: i64,
    pub path_bytes: Vec<u8>,
    pub path_key: Vec<u8>,
    pub store_key: String,
    pub scan_generation: i64,
    pub presence: Presence,
}

#[derive(Debug)]
pub struct PresenceContext<'a> {
    pub generation: i64,
    pub roots: &'a [ScanRootRow],
    pub skip: &'a SkipList,
    /// Every store at least one enabled root was reachable on this generation.
    pub present_stores: &'a BTreeSet<String>,
}

/// §4.6's four-way rule. Pure — the caller decides whether to persist the answer.
#[must_use]
pub fn classify_presence(row: &LocationPresenceRow, ctx: &PresenceContext<'_>) -> Presence {
    let Some(root) = ctx
        .roots
        .iter()
        .find(|root| root.enabled && is_under(&row.path_key, &root.path_key))
    else {
        return Presence::Unscanned;
    };
    // `covers`, not `skips`: this row names the repository *inside* an excluded directory, and
    // asking whether its own final component is on the list answers the wrong question — the
    // location would come back `missing`, claiming a repository is gone when the walk simply
    // never descended to it.
    if ctx.skip.covers(
        &path_from_bytes(&row.path_bytes),
        &path_from_bytes(&root.path_bytes),
    ) {
        return Presence::Unscanned;
    }
    if row.scan_generation == ctx.generation {
        return Presence::Present;
    }
    if ctx.present_stores.contains(&row.store_key) {
        Presence::Missing
    } else {
        Presence::Offline
    }
}

/// The project rollup. `project` has no presence column: this is derived at read time, which is
/// why the function is pure and takes a slice.
#[must_use]
pub fn project_presence(locations: &[Presence]) -> Presence {
    if locations.is_empty() {
        return Presence::Unscanned;
    }
    if locations.iter().all(|p| *p == Presence::Offline) {
        return Presence::Offline;
    }
    if locations.contains(&Presence::Present) {
        return Presence::Present;
    }
    if locations.contains(&Presence::Missing) {
        return Presence::Missing;
    }
    Presence::Unscanned
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PresenceSummary {
    pub present: u64,
    pub missing: u64,
    pub offline: u64,
    pub unscanned: u64,
    /// How many rows the pass actually rewrote.
    pub changed: u64,
    pub offline_projects: u64,
}

#[derive(Debug)]
pub struct ScanStoreError {
    pub message: String,
}

impl ScanStoreError {
    #[must_use]
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for ScanStoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ScanStoreError {}

/// One `scan_run` row at insert time (§1.9). `mode` is `"full"` or `"incremental"`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanRunStart {
    pub generation: i64,
    pub started_at: i64,
    pub mode: &'static str,
    pub roots_json: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScanRunFinish {
    pub ended_at: i64,
    pub walked_dirs: u64,
    pub found_repos: u64,
    pub cancelled: bool,
}

/// One `scan_run` row as it reads back (§1.9). `mode` is the stored text; `scan.status` turns it
/// into the generated `ScanMode` through serde rather than through a second table of two strings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanRunRow {
    pub id: i64,
    pub generation: i64,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub mode: String,
    pub walked_dirs: i64,
    pub found_repos: i64,
    pub cancelled: bool,
}

/// The scanner's whole view of the database, and its **only** one.
///
/// **Nothing here deletes**, and §17 is what says there must not be one.
///
/// **R35(a), second option: there is no `upsert_location` here either.** R1 put a delegating
/// method on this trait; the delegation is not expressible, because the `location` row must be
/// written inside the transaction `resolve_identity` decided the project in, and a trait method
/// taking a `&Transaction` leaks the identity module's transaction into the scanner's seam.
/// The writer is `identity::store::upsert_location`, reached through
/// `assembly::handoff::hand_off_discovered`.
///
/// `Send + Sync` because the run executes on a worker thread while the protocol loop answers
/// `scan.status` — `rusqlite::Connection` is `Send` but not `Sync` (R39), so the production
/// implementation owns it behind a mutex rather than handing one out.
pub trait ScanStore: Send + Sync {
    fn next_generation(&self) -> Result<i64, ScanStoreError>;
    fn scan_roots(&self) -> Result<Vec<ScanRootRow>, ScanStoreError>;
    /// Projects already indexed, for §4.8's resumed counter.
    fn indexed_project_count(&self) -> Result<u64, ScanStoreError>;
    fn locations_for_presence(&self) -> Result<Vec<LocationPresenceRow>, ScanStoreError>;
    /// Writes `location.presence` and nothing else — `scan_generation` belongs to whoever saw
    /// the location, and `last_seen_at` to whoever upserted it.
    fn set_presence(&self, location_id: i64, presence: Presence) -> Result<(), ScanStoreError>;
    fn begin_scan_run(&self, start: &ScanRunStart) -> Result<i64, ScanStoreError>;
    fn finish_scan_run(
        &self,
        scan_run_id: i64,
        finish: &ScanRunFinish,
    ) -> Result<(), ScanStoreError>;
    fn record_problem(
        &self,
        scan_run_id: i64,
        problem: &crate::scan::ScanProblem,
    ) -> Result<(), ScanStoreError>;
    /// The newest `scan_run` row, or `None` when no scan has ever run.
    ///
    /// `None` is the fact that separates *no scan has ever run* from *a scan ran and found
    /// nothing*; plan 16's first-run gate is exactly that distinction, so confusing the two
    /// either replays the whole reveal on a configured machine or withholds it on an empty one.
    fn latest_scan_run(&self) -> Result<Option<ScanRunRow>, ScanStoreError>;
    /// How many `scan_problem` rows one run recorded. §11.1's header figure, computed once here
    /// rather than in both the launcher's `ScanFinished` and plan 17's `problems.list`.
    fn problem_count(&self, scan_run_id: i64) -> Result<u64, ScanStoreError>;
    /// Projects whose lineage could not be decided (§1.1). A live query over `project`, not a
    /// `scan_problem` kind — §11.1's eighth group reads the column, not this table.
    fn ambiguous_lineage_count(&self) -> Result<u64, ScanStoreError>;
}

/// Run §4.6 over every location. Called **only after a run completes** — a cancelled run did not
/// visit everything, so applying this to one would mark half the library `missing` (§4.8).
pub fn apply_presence(
    store: &dyn ScanStore,
    ctx: &PresenceContext<'_>,
) -> Result<PresenceSummary, ScanStoreError> {
    let rows = store.locations_for_presence()?;
    let mut summary = PresenceSummary::default();
    let mut by_project: BTreeMap<i64, Vec<Presence>> = BTreeMap::new();

    for row in &rows {
        let next = classify_presence(row, ctx);
        match next {
            Presence::Present => summary.present += 1,
            Presence::Missing => summary.missing += 1,
            Presence::Offline => summary.offline += 1,
            Presence::Unscanned => summary.unscanned += 1,
        }
        by_project.entry(row.project_id).or_default().push(next);
        if next != row.presence {
            store.set_presence(row.location_id, next)?;
            summary.changed += 1;
        }
    }

    summary.offline_projects = by_project
        .values()
        .filter(|places| project_presence(places) == Presence::Offline)
        .count()
        .try_into()
        .unwrap_or(u64::MAX);
    Ok(summary)
}

/// §4.6: disabling a root marks its locations `unscanned` — neither gone nor verified — at the
/// moment `roots.setEnabled` runs, without waiting for the next scan.
pub fn mark_root_unscanned(
    store: &dyn ScanStore,
    root: &ScanRootRow,
) -> Result<u64, ScanStoreError> {
    let mut changed = 0;
    for row in store.locations_for_presence()? {
        if is_under(&row.path_key, &root.path_key) && row.presence != Presence::Unscanned {
            store.set_presence(row.location_id, Presence::Unscanned)?;
            changed += 1;
        }
    }
    Ok(changed)
}
