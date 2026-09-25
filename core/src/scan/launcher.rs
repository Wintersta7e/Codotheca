//! **R40's second half** — the production [`ScanLauncher`].
//!
//! Nothing in the plan set produced one; only the fake existed, which is what made plan 21's
//! Task 7 uncompletable. This is also the place R8 puts the `WalkEvent` → `scan` topic mapping:
//! the walk's own enum never reaches the wire, and the generated payloads are what does.
//!
//! **Three `WalkEvent`s are deliberately not published**, because their payloads need values only
//! later plans produce, and inventing them would put a claim on the wire the app cannot support:
//!
//! * `Discovered` → `repo_found` needs `projectId` and a `LocationRef`. The ids now exist —
//!   `ScanRunner` writes the rows before it emits the event — but building the payload here
//!   would put the hand-off's result on the wire from a second place; `scan/job_done` already
//!   says a location was indexed.
//! * `SubmoduleEdge` → no schema event carries one; plan 08 writes the edge.
//! * `WslBridge` → §13's dispatch is plan 18's, and no `scan` event describes a refused bridge.
//!
//! `run_started` is **not** published here either: only `scan.start` knows whether it started a
//! run or coalesced onto a live one, and one run started is one event.

use std::sync::{Arc, Mutex};
use std::thread;

use crate::cancel::CancelToken;
use crate::clock::Clock;
use crate::git::GitBackend;
use crate::index::Index;
use crate::jobs::JobSink;
use crate::mount::MountResolver;
use crate::proto::EventSink;
use crate::protocol::{
    ProblemFound, ProblemKind, RootId, ScanCancelled, ScanFinished, ScanMode, ScanProgress,
    ScanRunId,
};
use crate::scan::presence::{ScanStore, ScanStoreError};
use crate::scan::run::{ScanRunOutcome, ScanRunner};
use crate::scan::skiplist::SkipList;
use crate::scan::{LaunchedScan, ScanLauncher, ScanProblem, ScanProgressCell, WalkEvent};

/// Runs one scan on a worker thread and publishes it onto the `scan` topic.
///
/// Every collaborator is an `Arc` rather than a borrow because the walk outlives the `launch`
/// call that started it. `rusqlite::Connection` is `Send` but not `Sync` (R39), so the index
/// crosses to the walk thread behind a mutex — the same one `store` holds, never a second
/// connection.
#[derive(Clone)]
pub struct ThreadScanLauncher {
    store: Arc<dyn ScanStore>,
    index: Arc<Mutex<Index>>,
    git: Arc<dyn GitBackend>,
    mounts: Arc<dyn MountResolver>,
    clock: Arc<dyn Clock>,
    skip: Arc<SkipList>,
    /// §13's dispatcher, or `None` on a host with no WSL and in a build that staged no worker.
    wsl: Option<Arc<crate::wsl::dispatch::WslDispatcher>>,
    /// §4.1a's queue. **Not `NullJobSink`**: that is the absence of a scheduler, not a fake of
    /// one, and installing it here is what left every discovered repository uncomputed.
    jobs: Arc<dyn JobSink>,
    events: Arc<dyn EventSink>,
}

impl std::fmt::Debug for ThreadScanLauncher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ThreadScanLauncher").finish_non_exhaustive()
    }
}

/// What one launcher is built from.
///
/// A struct rather than eight positional arguments: two of them are `Arc<dyn …>` over traits
/// with no relation to each other, and a swapped pair would compile.
pub struct ScanLauncherDeps {
    /// The scanner's seam onto its own rows, over the same connection as `index`.
    pub store: Arc<dyn ScanStore>,
    /// The one index connection, which the hand-off writes `location` rows through (R39).
    pub index: Arc<Mutex<Index>>,
    /// Git, for discovery's probe and the hand-off's reads.
    pub git: Arc<dyn GitBackend>,
    /// Resolves a path's device identities (§4.7).
    pub mounts: Arc<dyn MountResolver>,
    /// The unix-seconds clock that runs and their events are stamped with.
    pub clock: Arc<dyn Clock>,
    /// The exclusion list (§4.3) every run applies.
    pub skip: Arc<SkipList>,
    /// §13's dispatcher. `None` on a host with no WSL, or in a build that staged no worker.
    pub wsl: Option<Arc<crate::wsl::dispatch::WslDispatcher>>,
    /// §4.1a's queue for each newly indexed location.
    pub jobs: Arc<dyn JobSink>,
    /// Where the `scan` topic's events are published.
    pub events: Arc<dyn EventSink>,
}

impl std::fmt::Debug for ScanLauncherDeps {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScanLauncherDeps").finish_non_exhaustive()
    }
}

impl ThreadScanLauncher {
    /// A launcher holding `deps` for every run it starts.
    #[must_use]
    pub fn new(deps: ScanLauncherDeps) -> Self {
        Self {
            store: deps.store,
            index: deps.index,
            git: deps.git,
            mounts: deps.mounts,
            clock: deps.clock,
            skip: deps.skip,
            wsl: deps.wsl,
            jobs: deps.jobs,
            events: deps.events,
        }
    }

    fn runner<'a>(&'a self, cancel: &'a CancelToken) -> ScanRunner<'a> {
        ScanRunner {
            store: self.store.as_ref(),
            git: self.git.as_ref(),
            mounts: Arc::clone(&self.mounts),
            clock: self.clock.as_ref(),
            skip: self.skip.as_ref(),
            cancel,
            index: self.index.as_ref(),
            wsl: self.wsl.as_deref(),
            jobs: Arc::clone(&self.jobs),
        }
    }

    /// The run's last word. A cancelled run says `cancelled`, never `finished`: saying it
    /// finished would claim the library was fully visited when half of it was not.
    fn publish_end(&self, run_id: ScanRunId, outcome: &ScanRunOutcome) {
        let ended_at = self.clock.now_unix();
        let indexed = self.store.indexed_project_count().unwrap_or(0);
        if outcome.cancelled {
            self.emit(
                "cancelled",
                &ScanCancelled {
                    run_id,
                    ended_at,
                    indexed_projects: clamp32(indexed),
                },
            );
            return;
        }
        // Both figures are measured, not assumed: `problem_count` counts this run's rows and
        // `ambiguous_lineage_count` reads `project.ambiguous_lineage`. A failed read reports the
        // run without them rather than publishing a zero it did not measure.
        let (Ok(problems), Ok(ambiguous)) = (
            self.store.problem_count(outcome.scan_run_id),
            self.store.ambiguous_lineage_count(),
        ) else {
            eprintln!("scan: run {} finished but its problem figures could not be read; no `finished` event published", outcome.scan_run_id);
            return;
        };
        self.emit(
            "finished",
            &ScanFinished {
                run_id,
                ended_at,
                walked_dirs: clamp64(outcome.walked_dirs),
                found_repos: clamp32(outcome.found_repos),
                problem_count: clamp32(problems),
                ambiguous_lineage_count: clamp32(ambiguous),
            },
        );
    }

    fn emit<T: serde::Serialize>(&self, event: &str, payload: &T) {
        match serde_json::to_value(payload) {
            Ok(value) => self.events.emit("scan", event, value),
            Err(err) => eprintln!("scan: could not encode `{event}`: {err}"),
        }
    }

    /// `WalkEvent` → the `scan` topic. See the module note for the three that are not mapped.
    fn publish_walk_event(
        &self,
        run_id: ScanRunId,
        indexed_projects: u32,
        progress: &ScanProgressCell,
        event: WalkEvent,
    ) {
        match event {
            WalkEvent::Progress {
                walked_dirs,
                found_repos,
            } => {
                progress.observe(walked_dirs, found_repos);
                self.emit(
                    "progress",
                    &ScanProgress {
                        run_id,
                        walked_dirs: clamp64(walked_dirs),
                        found_repos: clamp32(found_repos),
                        indexed_projects,
                    },
                );
            }
            WalkEvent::Problem(problem) => self.publish_problem(run_id, &problem),
            _ => {}
        }
    }

    /// `ProblemFound` carries the path and the diagnostic, not §11.1's running totals — those are
    /// the summary header's and plan 17 reads them once. The `detail` is diagnostic and the shell
    /// owns the prose the user sees.
    fn publish_problem(&self, run_id: ScanRunId, problem: &ScanProblem) {
        let Some(kind) = problem_kind(problem) else {
            eprintln!(
                "scan: problem kind `{}` has no wire spelling",
                problem.kind.as_str()
            );
            return;
        };
        self.emit(
            "problem",
            &ProblemFound {
                run_id,
                kind,
                path_display: problem.path_display.clone(),
                detail: Some(problem.detail.clone()),
            },
        );
    }
}

impl ScanLauncher for ThreadScanLauncher {
    /// Writes the `scan_run` row, then hands the walk to a thread and returns.
    ///
    /// Returning only once the walk finished would block the protocol loop for tens of seconds,
    /// which is why `ScanRunner` splits `begin` from `finish` at all.
    fn launch(
        &self,
        mode: ScanMode,
        _now: i64,
        cancel: CancelToken,
        progress: Arc<ScanProgressCell>,
    ) -> Result<LaunchedScan, ScanStoreError> {
        let begun = self.runner(&cancel).begin(mode)?;
        let launched = LaunchedScan {
            scan_run_id: ScanRunId(begun.scan_run_id),
            generation: begun.generation,
            roots: begun.enabled_root_ids().into_iter().map(RootId).collect(),
        };

        let run_id = launched.scan_run_id;
        let scan_run_id = begun.scan_run_id;
        let indexed_projects = clamp32(begun.resumed_from);
        let worker = self.clone();
        // One clone for the worker's own use, one kept here so a spawn failure can still release
        // the supervisor's slot.
        let worker_progress = Arc::clone(&progress);

        let spawned = thread::Builder::new()
            .name("codotheca-scan".to_owned())
            .spawn(move || {
                let outcome = worker.runner(&cancel).finish(begun, &|event| {
                    worker.publish_walk_event(run_id, indexed_projects, &worker_progress, event);
                });
                match outcome {
                    Ok(outcome) => worker.publish_end(run_id, &outcome),
                    Err(err) => eprintln!("scan: run {scan_run_id} failed: {err}"),
                }
                // Last, and unconditional: this is what releases the supervisor's slot, so a
                // failed run must not leave the library unable to start another.
                worker_progress.finish();
            });

        if let Err(err) = spawned {
            // The row exists and nothing will ever finish it. Say so rather than reporting a
            // launch that did not happen.
            progress.finish();
            return Err(ScanStoreError::new(format!(
                "could not spawn the scan worker for run {scan_run_id}: {err}"
            )));
        }
        Ok(launched)
    }
}

/// `ScanProblemKind` → the wire's `ProblemKind`. The two enums share their serialised spellings
/// (R26), so serde is the mapping rather than a second `match` that can drift from it. The wire
/// enum is wider: `deferred_slow` and `ambiguous_lineage` are §11.1 groups that read other
/// tables, and no `ScanProblemKind` produces them.
fn problem_kind(problem: &ScanProblem) -> Option<ProblemKind> {
    serde_json::from_value(serde_json::Value::String(problem.kind.as_str().to_owned())).ok()
}

fn clamp32(value: u64) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

fn clamp64(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}
