//! Scanner doubles. Public and behind `testkit`, not `#[cfg(test)]`.
//!
//! **R40's extension is the reason for the visibility.** Plan 07 drafted these as a `pub(crate)`
//! `core::scan::fakes` module, which plan 21's integration fixture cannot reach at all; the
//! ruling says they belong behind the `testkit` feature like plan 06's other fakes. They also
//! replace plan 07's own `ScanMountFake`, `ScanGitFake` and `FixedClock`, which duplicate
//! [`super::FakeMountResolver`], [`super::FakeGitBackend`] and [`super::FakeClock`] (R29).

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use crate::cancel::CancelToken;
use crate::derive::LocationKind;
use crate::identity::store::LocationInput;
use crate::protocol::{RootId, ScanMode, ScanRunId};
use crate::scan::presence::{
    LocationPresenceRow, Presence, ScanRootRow, ScanRunFinish, ScanRunRow, ScanRunStart, ScanStore,
    ScanStoreError,
};
use crate::scan::ScanProblem;
use crate::scan::{LaunchedScan, ScanLauncher, ScanProgressCell};

/// An in-memory [`ScanStore`]. Like the real one it has no way to remove a row, which is how the
/// "deletes nothing" assertions are able to mean anything.
#[derive(Debug, Default)]
pub struct MemScanStore {
    inner: Mutex<MemInner>,
}

#[derive(Debug, Default)]
struct MemInner {
    generation: i64,
    roots: Vec<ScanRootRow>,
    locations: BTreeMap<i64, LocationPresenceRow>,
    /// R1: `(kind, distro, path_key) -> location_id`, the uniqueness `upsert_location` is
    /// idempotent on. `push_location` does not populate it — it fabricates rows directly for the
    /// presence tests and never goes through the writer.
    location_identity: Vec<(LocationKind, Option<String>, Vec<u8>, i64)>,
    indexed_projects: u64,
    runs: Vec<(ScanRunStart, Option<ScanRunFinish>)>,
    problems: Vec<(i64, ScanProblem)>,
}

impl MemScanStore {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push_root(&self, root: ScanRootRow) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.roots.push(root);
        }
    }

    pub fn push_location(&self, row: LocationPresenceRow) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.locations.insert(row.location_id, row);
            inner.indexed_projects = u64::try_from(inner.locations.len()).unwrap_or(u64::MAX);
        }
    }

    pub fn set_generation_of(&self, location_id: i64, generation: i64) {
        if let Ok(mut inner) = self.inner.lock() {
            if let Some(row) = inner.locations.get_mut(&location_id) {
                row.scan_generation = generation;
            }
        }
    }

    #[must_use]
    pub fn presence_of(&self, location_id: i64) -> Option<Presence> {
        let inner = self.inner.lock().ok()?;
        inner.locations.get(&location_id).map(|row| row.presence)
    }

    #[must_use]
    pub fn location_count(&self) -> usize {
        self.inner.lock().map_or(0, |inner| inner.locations.len())
    }

    /// Named `recorded_problems`, not `problem_count`: the trait now has a `problem_count` that
    /// takes a run id, and an inherent method with the same name and a different arity shadows it
    /// for every caller holding a concrete `MemScanStore`.
    #[must_use]
    pub fn recorded_problems(&self) -> usize {
        self.inner.lock().map_or(0, |inner| inner.problems.len())
    }

    #[must_use]
    pub fn problems(&self) -> Vec<ScanProblem> {
        self.inner.lock().map_or_else(
            |_| Vec::new(),
            |inner| inner.problems.iter().map(|(_, p)| p.clone()).collect(),
        )
    }

    #[must_use]
    pub fn finished_runs(&self) -> Vec<ScanRunFinish> {
        self.inner.lock().map_or_else(
            |_| Vec::new(),
            |inner| inner.runs.iter().filter_map(|(_, fin)| *fin).collect(),
        )
    }

    fn err() -> ScanStoreError {
        ScanStoreError::new("poisoned in-memory scan store")
    }
}

impl ScanStore for MemScanStore {
    fn next_generation(&self) -> Result<i64, ScanStoreError> {
        let mut inner = self.inner.lock().map_err(|_| Self::err())?;
        inner.generation += 1;
        Ok(inner.generation)
    }

    fn scan_roots(&self) -> Result<Vec<ScanRootRow>, ScanStoreError> {
        Ok(self.inner.lock().map_err(|_| Self::err())?.roots.clone())
    }

    fn indexed_project_count(&self) -> Result<u64, ScanStoreError> {
        Ok(self.inner.lock().map_err(|_| Self::err())?.indexed_projects)
    }

    fn locations_for_presence(&self) -> Result<Vec<LocationPresenceRow>, ScanStoreError> {
        Ok(self
            .inner
            .lock()
            .map_err(|_| Self::err())?
            .locations
            .values()
            .cloned()
            .collect())
    }

    fn set_presence(&self, location_id: i64, presence: Presence) -> Result<(), ScanStoreError> {
        let mut inner = self.inner.lock().map_err(|_| Self::err())?;
        if let Some(row) = inner.locations.get_mut(&location_id) {
            row.presence = presence;
        }
        Ok(())
    }

    fn begin_scan_run(&self, start: &ScanRunStart) -> Result<i64, ScanStoreError> {
        let mut inner = self.inner.lock().map_err(|_| Self::err())?;
        inner.runs.push((start.clone(), None));
        i64::try_from(inner.runs.len()).map_err(|_| Self::err())
    }

    fn finish_scan_run(
        &self,
        scan_run_id: i64,
        finish: &ScanRunFinish,
    ) -> Result<(), ScanStoreError> {
        let mut inner = self.inner.lock().map_err(|_| Self::err())?;
        let idx = usize::try_from(scan_run_id.saturating_sub(1)).map_err(|_| Self::err())?;
        if let Some(slot) = inner.runs.get_mut(idx) {
            slot.1 = Some(*finish);
        }
        Ok(())
    }

    fn record_problem(
        &self,
        scan_run_id: i64,
        problem: &ScanProblem,
    ) -> Result<(), ScanStoreError> {
        let mut inner = self.inner.lock().map_err(|_| Self::err())?;
        inner.problems.push((scan_run_id, problem.clone()));
        Ok(())
    }

    fn latest_scan_run(&self) -> Result<Option<ScanRunRow>, ScanStoreError> {
        let inner = self.inner.lock().map_err(|_| Self::err())?;
        let Some((index, (start, finish))) = inner.runs.iter().enumerate().next_back() else {
            return Ok(None);
        };
        let id = i64::try_from(index + 1).map_err(|_| Self::err())?;
        Ok(Some(ScanRunRow {
            id,
            generation: start.generation,
            started_at: start.started_at,
            ended_at: finish.map(|f| f.ended_at),
            mode: start.mode.to_owned(),
            walked_dirs: finish.map_or(0, |f| i64::try_from(f.walked_dirs).unwrap_or(i64::MAX)),
            found_repos: finish.map_or(0, |f| i64::try_from(f.found_repos).unwrap_or(i64::MAX)),
            cancelled: finish.is_some_and(|f| f.cancelled),
        }))
    }

    fn problem_count(&self, scan_run_id: i64) -> Result<u64, ScanStoreError> {
        let inner = self.inner.lock().map_err(|_| Self::err())?;
        let n = inner
            .problems
            .iter()
            .filter(|(run, _)| *run == scan_run_id)
            .count();
        Ok(u64::try_from(n).unwrap_or(0))
    }

    /// The double holds no `project` rows, so this is the only figure it cannot measure. Zero is
    /// what an index with no resolved lineage genuinely reports; a test that needs a non-zero one
    /// uses the real store.
    fn ambiguous_lineage_count(&self) -> Result<u64, ScanStoreError> {
        Ok(0)
    }

    /// R1. Idempotent on `(kind, distro, path_key)`, exactly as plan 08's real writer must be: a
    /// second scan of the same repository updates the row it already has rather than adding
    /// another. A fake that appended would let a duplicate-location bug pass every test here.
    fn upsert_location(&self, project_id: i64, loc: &LocationInput) -> Result<i64, ScanStoreError> {
        let mut inner = self.inner.lock().map_err(|_| Self::err())?;
        // A linear scan, not a map: `LocationKind` derives `PartialEq` and neither `Ord` nor
        // `Hash`, and R21 makes it plan 09's to change. A fake holding a shelf's worth of rows
        // does not need the index, and inventing derives on another plan's type would.
        let existing = inner
            .location_identity
            .iter()
            .find(|(kind, distro, key, _)| {
                *kind == loc.kind && distro == &loc.distro && key.as_slice() == loc.path.key()
            });
        if let Some((_, _, _, id)) = existing {
            let id = *id;
            if let Some(row) = inner.locations.get_mut(&id) {
                row.project_id = project_id;
                row.scan_generation = loc.generation;
                row.presence = Presence::Present;
            }
            return Ok(id);
        }
        let location_id = i64::try_from(inner.locations.len() + 1).map_err(|_| Self::err())?;
        inner.locations.insert(
            location_id,
            LocationPresenceRow {
                location_id,
                project_id,
                path_bytes: loc.path.bytes().to_vec(),
                path_key: loc.path.key().to_vec(),
                store_key: loc.store_key.clone(),
                scan_generation: loc.generation,
                presence: loc.presence,
            },
        );
        inner.location_identity.push((
            loc.kind,
            loc.distro.clone(),
            loc.path.key().to_vec(),
            location_id,
        ));
        inner.indexed_projects = u64::try_from(inner.locations.len()).unwrap_or(u64::MAX);
        Ok(location_id)
    }
}

/// Hands back a `scan_run` id without walking anything, and counts how often it was asked. The
/// count is the assertion that a second `scan.start` starts nothing.
#[derive(Debug, Default)]
pub struct ScanLauncherFake {
    launches: std::sync::atomic::AtomicU64,
    next_id: std::sync::atomic::AtomicI64,
}

impl ScanLauncherFake {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn launches(&self) -> u64 {
        self.launches.load(std::sync::atomic::Ordering::Relaxed)
    }
}

impl ScanLauncher for ScanLauncherFake {
    fn launch(
        &self,
        _mode: ScanMode,
        _now: i64,
        _cancel: CancelToken,
        _progress: Arc<ScanProgressCell>,
    ) -> Result<LaunchedScan, ScanStoreError> {
        self.launches
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let n = self
            .next_id
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            + 1;
        Ok(LaunchedScan {
            scan_run_id: ScanRunId(n),
            generation: n,
            roots: vec![RootId(1)],
        })
    }
}

/// R16: plan 03's `EventSink`, recorded rather than published.
#[derive(Debug, Default)]
pub struct ScanEventFake {
    emitted: Mutex<Vec<(String, String, serde_json::Value)>>,
}

impl ScanEventFake {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Payloads for one `(topic, event)`, in the order they were emitted.
    #[must_use]
    pub fn named(&self, topic: &str, event: &str) -> Vec<serde_json::Value> {
        self.emitted
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .filter(|(t, e, _)| t == topic && e == event)
            .map(|(_, _, payload)| payload.clone())
            .collect()
    }

    /// Every emission, so a test can assert that something was *not* published.
    #[must_use]
    pub fn all(&self) -> Vec<(String, String)> {
        self.emitted
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .map(|(t, e, _)| (t.clone(), e.clone()))
            .collect()
    }
}

impl crate::proto::EventSink for ScanEventFake {
    fn emit(&self, topic: &str, event: &str, payload: serde_json::Value) {
        self.emitted
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((topic.to_owned(), event.to_owned(), payload));
    }
}
