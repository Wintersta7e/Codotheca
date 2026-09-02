//! Building, starting and stopping the job pump.
//!
//! `JobRunner` was constructed in exactly one place before this — `core/tests/jobs_scheduler.rs`,
//! a test — so J1–J6 never ran in the product: no ref state, no status, no inventory, no
//! history, no card art. The walk wrote rows and nothing computed a fact about them.
//!
//! **This is construction, not a second scheduler.** The runner owns its own worker pool and
//! `run_one` releases the index lock around every git invocation (R39), which is what makes a
//! pool safe beside a protocol loop holding the one `rusqlite::Connection`.

use std::sync::{Arc, Mutex};

use crate::cancel::CancelToken;
use crate::clock::Clock;
use crate::git::GitBackend;
use crate::index::Index;
use crate::jobs::queue::global_cap;
use crate::jobs::scheduler::JobRunner;
use crate::jobs::{JobDeps, JobSink};
use crate::proto::EventSink;

/// How many worker threads the pump runs.
///
/// **Deviation from the plan, which declares `JOB_WORKERS: usize` as a `const`.** §3.4's number
/// already has an owner — `jobs::queue::global_cap()`, `min(16, cores)` — and it is also what
/// `SlotState` admits on, so a const here would be a second copy of one rule and a larger pool
/// would only park threads in `take_next`.
#[must_use]
pub fn job_workers() -> usize {
    global_cap()
}

/// Assemble a runner. Nothing is started until [`JobRunner::start`].
///
/// Takes `Arc<Mutex<Index>>` because `rusqlite::Connection` is `Send` but not `Sync` (R39): the
/// mutex is what lets one connection be reached from the loop thread and from a worker without a
/// second one existing.
#[must_use]
pub fn build_job_runner(
    index: Arc<Mutex<Index>>,
    deps: JobDeps,
    events: Arc<dyn EventSink>,
) -> Arc<JobRunner> {
    JobRunner::new(index, deps, events)
}

/// The running pump: the worker pool, and the token that stops the git it has in flight.
///
/// The two travel together because stopping is two acts in one order. `request_stop` only asks a
/// worker to take no *further* job; a worker inside a twenty-second history read would keep the
/// process alive until git returned, and the process is already exiting with its window closed.
/// Cancelling first takes down the git process tree, so the join is bounded.
#[derive(Clone)]
pub struct JobPump {
    runner: Arc<JobRunner>,
    cancel: CancelToken,
}

impl std::fmt::Debug for JobPump {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JobPump").finish_non_exhaustive()
    }
}

impl JobPump {
    /// Build the runner and spawn its pool.
    #[must_use]
    pub fn start(
        index: Arc<Mutex<Index>>,
        git: Arc<dyn GitBackend>,
        clock: Arc<dyn Clock>,
        events: Arc<dyn EventSink>,
    ) -> JobPump {
        let cancel = CancelToken::new();
        let runner = build_job_runner(
            index,
            JobDeps {
                git,
                clock,
                cancel: cancel.clone(),
            },
            events,
        );
        runner.start(job_workers());
        JobPump { runner, cancel }
    }

    /// The pump seen as the scanner's hand-off point.
    #[must_use]
    pub fn sink(&self) -> Arc<dyn JobSink> {
        Arc::clone(&self.runner) as Arc<dyn JobSink>
    }

    /// The same seam borrowed rather than cloned, for a command context that lives one call.
    #[must_use]
    pub fn sink_ref(&self) -> &dyn JobSink {
        self.runner.as_ref()
    }

    /// Stop every worker and wait for it.
    ///
    /// Idempotent, and it must be: it runs from `CoreHandler::shutdown`, which the loop calls
    /// once, and a second call has to be harmless rather than a second join on taken handles.
    pub fn stop(&self) {
        self.cancel.cancel();
        self.runner.request_stop();
        self.runner.join();
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
mod tests {
    use super::job_workers;

    /// §3.4's number has one owner. A const here would be a second copy of it, and a copy of a
    /// rule is a copy that drifts.
    #[test]
    fn the_pool_is_sized_from_the_one_owner_of_the_concurrency_rule() {
        assert_eq!(job_workers(), crate::jobs::queue::global_cap());
        assert!(job_workers() >= 1, "a pool of zero drains nothing");
    }
}
