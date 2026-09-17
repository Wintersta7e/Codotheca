//! **R54**: §24.4's stage state lives in the core, so an off-screen virtualized tile does not
//! render and scrolling back shows the *correct* stage rather than a replay.
//!
//! A stream of four events cannot deliver that to a tile that was unmounted while they fired.
//! `install.snapshot` answers on subscribe, on the `projects`/`core` precedent the schema already
//! has — and the snapshot is what a renderer opening mid-clone builds its first frame from.
//!
//! **`InstallState` is two lists joined on `runId`, not one.** `InstallStage` carries no
//! `projectId` — its field set is pinned by p2-24r's criterion so no aggregate can be smuggled in
//! — so `runs` alone cannot tell a tile which run is its own, which is the entire case the
//! snapshot exists for. `started` carries the project id and the composed display form, which a
//! tile mounting mid-clone needs anyway.

use std::collections::BTreeMap;
use std::sync::Mutex;

use crate::protocol::{
    InstallRunId, InstallStage, InstallStageKind, InstallStarted, InstallState, ProjectId,
};

/// One run as the snapshot remembers it.
#[derive(Debug, Clone)]
struct Remembered {
    started: InstallStarted,
    stage: InstallStage,
    /// A settled or failed run is kept briefly so a tile scrolled back into view after the end
    /// still learns how it ended, rather than finding nothing and rendering *not cloned*.
    ended: bool,
}

/// Every running and recently-settled install.
#[derive(Debug, Default)]
pub struct InstallStateStore {
    runs: Mutex<BTreeMap<i64, Remembered>>,
}

/// How many ended runs are kept. Small on purpose: this is a courtesy for a tile that was
/// unmounted across the ending, not a history — `install_run` is the durable record.
const KEEP_ENDED: usize = 8;

impl InstallStateStore {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record that a run has begun, with the facts a tile needs to recognise it as its own.
    pub fn begin(&self, run: InstallRunId, project: ProjectId, destination_display: String) {
        let mut runs = self
            .runs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        runs.insert(
            run.0,
            Remembered {
                started: InstallStarted {
                    run_id: run,
                    project_id: project,
                    destination_display,
                },
                stage: InstallStage {
                    run_id: run,
                    stage: InstallStageKind::Plans,
                    done: None,
                    total: None,
                    bytes: None,
                },
                ended: false,
            },
        );
    }

    /// Record the stage a run has reached. Unknown runs are ignored rather than invented.
    pub fn record(&self, stage: InstallStage) {
        let mut runs = self
            .runs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(entry) = runs.get_mut(&stage.run_id.0) {
            entry.ended = stage.stage == InstallStageKind::Settled;
            entry.stage = stage;
        }
    }

    /// Mark a run finished without a settled stage — a failure or a cancellation.
    pub fn end(&self, run: InstallRunId) {
        let mut runs = self
            .runs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(entry) = runs.get_mut(&run.0) {
            entry.ended = true;
        }
        Self::trim(&mut runs);
    }

    /// The frame a subscriber builds from.
    #[must_use]
    pub fn snapshot(&self) -> InstallState {
        let runs = self
            .runs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        InstallState {
            runs: runs.values().map(|r| r.stage.clone()).collect(),
            started: runs.values().map(|r| r.started.clone()).collect(),
        }
    }

    /// Whether anything is known at all. `None` from the assembly's snapshot arm means *not
    /// computed*; an empty `InstallState` means *nothing is installing*, and the two are
    /// different claims.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.runs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_empty()
    }

    fn trim(runs: &mut BTreeMap<i64, Remembered>) {
        let ended: Vec<i64> = runs
            .iter()
            .filter(|(_, r)| r.ended)
            .map(|(id, _)| *id)
            .collect();
        if ended.len() <= KEEP_ENDED {
            return;
        }
        for id in ended.iter().take(ended.len() - KEEP_ENDED) {
            runs.remove(id);
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use super::InstallStateStore;
    use crate::protocol::{InstallRunId, InstallStage, InstallStageKind, ProjectId};

    #[test]
    fn a_resubscribe_sees_the_current_stage_and_not_a_replay() {
        let store = InstallStateStore::new();
        store.begin(InstallRunId(1), ProjectId(7), "<root>/widget".to_owned());
        for kind in [
            InstallStageKind::Enumerating,
            InstallStageKind::Receiving,
            InstallStageKind::Assembling,
        ] {
            store.record(InstallStage {
                run_id: InstallRunId(1),
                stage: kind,
                done: Some(3),
                total: Some(9),
                bytes: None,
            });
        }
        let snapshot = store.snapshot();
        assert_eq!(
            snapshot.runs.len(),
            1,
            "one entry per run, never one per event"
        );
        assert_eq!(
            snapshot.runs[0].stage,
            InstallStageKind::Assembling,
            "the current stage, not the first or the whole stream"
        );
        assert_eq!(
            snapshot.started[0].project_id,
            ProjectId(7),
            "a tile learns which run is its own from `started`, because InstallStage has no project id"
        );
    }

    #[test]
    fn an_unknown_run_is_ignored_rather_than_invented() {
        let store = InstallStateStore::new();
        store.record(InstallStage {
            run_id: InstallRunId(99),
            stage: InstallStageKind::Receiving,
            done: None,
            total: None,
            bytes: None,
        });
        assert!(store.is_empty(), "a stage for no known run creates no run");
    }

    #[test]
    fn nothing_known_and_nothing_installing_are_different_claims() {
        let store = InstallStateStore::new();
        assert!(store.is_empty());
        store.begin(InstallRunId(1), ProjectId(1), "<root>/a".to_owned());
        assert!(!store.is_empty());
    }
}
