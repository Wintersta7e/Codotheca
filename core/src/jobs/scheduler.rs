//! The scheduler (§4.1a).
//!
//! The ordering here is measured, not intuited: the most expensive repositories are shallow
//! clones of large public projects, those are exactly the ones that resolve to Reference, and
//! gating on a cheap authorship probe removed 61% of scan work from the critical path at 0.8%
//! of the cost.
//!
//! **R39.** `JobRunner::new(index: Arc<Index>, …)` cannot work: `Index` holds a
//! `rusqlite::Connection`, which is `Send` but not `Sync`, so `Arc<Index>` is not `Send` and
//! `start` cannot spawn a thread holding it. This takes an owning `Arc<Mutex<Index>>` — the
//! first of the two options the ruling names. The reason for that choice over a command channel
//! back to the loop thread is that **the loop thread cannot service one**: plan 03's `run_loop`
//! blocks reading stdin, so a channel needs a *third* thread owning the connection, and plan 21's
//! `CommandHandler` — which borrows the connection directly — would have to be rewritten to go
//! through it too. That is a change to a plan that has not run, made from this one.
//!
//! What makes the mutex safe is structural, not a note: `run_one` takes the lock to read its
//! inputs, releases it, runs git, and takes it again to write. No job holds it across a git
//! invocation, so a twenty-second J4 blocks no command.

use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;

use super::queue::{global_cap, store_cap_for, JobQueue, SlotState};
use super::state::{apply_outcome, put, JobStateRow};
use super::{run_one, Job, JobDeps, JobKind, JobOutcome, JobSink, JobState, Priority};
use crate::index::Index;
use crate::mount::StoreClass;
use crate::proto::EventSink;
use crate::protocol::{LocationId, ProjectId};

/// What to queue once `done` has finished.
///
/// `is_reference` is `None` when authorship is not computed, and **that is not Reference**:
/// deprioritising on a NULL would silently push a repository nothing has managed to read yet to
/// the bottom of every queue.
#[must_use]
pub fn next_jobs_after(done: JobKind, is_reference: Option<bool>) -> Vec<(JobKind, Priority)> {
    let band = if is_reference == Some(true) {
        Priority::Reference
    } else {
        Priority::Standard
    };
    match done {
        JobKind::J1Refstate => vec![(JobKind::J15Authorship, Priority::Authorship)],
        JobKind::J15Authorship => vec![
            (JobKind::J2Status, band),
            (JobKind::J3Inventory, band),
            (JobKind::J4History, Priority::Deferred),
        ],
        JobKind::J3Inventory => vec![
            (JobKind::J6Content, Priority::Deferred),
            (JobKind::J5Art, Priority::Deferred),
        ],
        JobKind::J2Status | JobKind::J4History | JobKind::J5Art | JobKind::J6Content => Vec::new(),
    }
}

#[derive(Debug)]
struct Shared {
    queue: JobQueue,
    slots: SlotState,
    stopping: bool,
}

/// The worker pool and the queue behind it.
pub struct JobRunner {
    index: Arc<Mutex<Index>>,
    deps: JobDeps,
    events: Arc<dyn EventSink>,
    shared: Mutex<Shared>,
    wake: Condvar,
    workers: Mutex<Vec<JoinHandle<()>>>,
}

impl std::fmt::Debug for JobRunner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JobRunner").finish_non_exhaustive()
    }
}

impl JobRunner {
    /// Assemble a runner. No thread is started until [`JobRunner::start`].
    #[must_use]
    pub fn new(index: Arc<Mutex<Index>>, deps: JobDeps, events: Arc<dyn EventSink>) -> Arc<Self> {
        Arc::new(JobRunner {
            index,
            deps,
            events,
            shared: Mutex::new(Shared {
                queue: JobQueue::new(),
                slots: SlotState::new(global_cap()),
                stopping: false,
            }),
            wake: Condvar::new(),
            workers: Mutex::new(Vec::new()),
        })
    }

    /// Queue one job. Returns false when an equal-or-better entry is already queued.
    pub fn enqueue(&self, job: Job) -> bool {
        let Ok(mut shared) = self.shared.lock() else {
            return false;
        };
        shared
            .slots
            .set_store_cap(&job.store_key, store_cap_for(job.store_kind));
        let pushed = shared.queue.push(job);
        drop(shared);
        self.wake.notify_one();
        pushed
    }

    /// Ask every worker to finish its current job and stop.
    pub fn request_stop(&self) {
        if let Ok(mut shared) = self.shared.lock() {
            shared.stopping = true;
        }
        self.wake.notify_all();
    }

    /// Wait for every worker to exit. Call [`JobRunner::request_stop`] first or this blocks.
    pub fn join(&self) {
        let handles: Vec<JoinHandle<()>> = match self.workers.lock() {
            Ok(mut w) => std::mem::take(&mut *w),
            Err(_) => Vec::new(),
        };
        for h in handles {
            drop(h.join());
        }
    }

    /// Spawn the pool.
    pub fn start(self: &Arc<Self>, workers: usize) {
        let mut spawned = Vec::new();
        for _ in 0..workers.max(1) {
            let me = Arc::clone(self);
            if let Ok(h) = std::thread::Builder::new()
                .name("codotheca-jobs".to_owned())
                .spawn(move || me.worker_loop())
            {
                spawned.push(h);
            }
        }
        if let Ok(mut w) = self.workers.lock() {
            w.extend(spawned);
        }
    }

    fn take_next(&self) -> Option<Job> {
        let mut shared = self.shared.lock().ok()?;
        loop {
            if shared.stopping {
                return None;
            }
            let now = self.deps.clock.now_unix();
            // Split the borrow: `pop_ready` needs the queue and the slot table at once, and
            // they live in one guarded struct.
            let Shared { queue, slots, .. } = &mut *shared;
            if let Some(job) = queue.pop_ready(now, slots) {
                return Some(job);
            }
            let (guard, _) = self
                .wake
                .wait_timeout(shared, std::time::Duration::from_millis(250))
                .ok()?;
            shared = guard;
        }
    }

    fn worker_loop(self: Arc<Self>) {
        while let Some(job) = self.take_next() {
            let outcome = self.execute(&job);
            self.settle(&job, &outcome);
            if let Ok(mut shared) = self.shared.lock() {
                shared.slots.release(&job);
            }
            self.wake.notify_all();
        }
    }

    /// Dispatch, and turn a failure into the outcome vocabulary the retry rules read.
    ///
    /// This function owns only the hand-off, so a job's behaviour is reviewable without reading
    /// the scheduler.
    fn execute(&self, job: &Job) -> JobOutcome {
        match run_one(self.index.as_ref(), &self.deps, job) {
            Ok(outcome) => outcome,
            Err(super::JobError::RepositoryBusy) => JobOutcome::TransientFail {
                reason: "repository_busy".to_owned(),
            },
            Err(super::JobError::TornRead) => JobOutcome::TransientFail {
                reason: "torn_read".to_owned(),
            },
            Err(super::JobError::BudgetExceeded) => JobOutcome::TransientFail {
                reason: "budget_exceeded".to_owned(),
            },
            Err(super::JobError::Git(e)) => {
                let detail = format!("{e:?}");
                if e.is_deferral() {
                    JobOutcome::TransientFail { reason: detail }
                } else {
                    JobOutcome::HardFail {
                        error_kind: git_error_kind(&e),
                        detail,
                    }
                }
            }
            Err(super::JobError::Index(e)) => JobOutcome::HardFail {
                error_kind: "INTERNAL",
                detail: format!("{e:?}"),
            },
            Err(super::JobError::Io(detail)) => JobOutcome::HardFail {
                error_kind: "REPO_UNREADABLE",
                detail,
            },
        }
    }

    fn settle(&self, job: &Job, outcome: &JobOutcome) {
        let now = self.deps.clock.now_unix();
        let previous = self
            .with_index(|index| super::state::load(index, job.project_id))
            .unwrap_or_default()
            .into_iter()
            .find(|r| r.job == job.kind)
            .unwrap_or_else(|| JobStateRow::fresh(job.kind, JobState::Running, now));

        // The job row and the recompute land in one transaction: a settled job whose derived
        // values were not rewritten is a project the shelf sections into the wrong era.
        let (row, requeue_at) = apply_outcome(&previous, outcome, now);
        let recomputed = self.write_index(|tx| {
            put(tx, job.project_id, &row)?;
            crate::derive::persist::recompute(tx, job.project_id, now)
        });
        if let Ok(r) = &recomputed {
            if r.changed_condition {
                self.events.emit(
                    "projects",
                    "condition_changed",
                    serde_json::json!({
                        "projectId": job.project_id.0,
                        "conditionSignal": r
                            .condition_signal
                            .map(crate::derive::condition::ConditionSignal::slug),
                    }),
                );
            }
        }

        self.events.emit(
            "scan",
            "job_done",
            serde_json::json!({
                "projectId": job.project_id.0,
                "locationId": job.location_id.0,
                "job": job.kind.slug(),
                "state": row.state.slug(),
            }),
        );

        if let Some(when) = requeue_at {
            let mut again = job.clone();
            again.not_before = when;
            self.enqueue(again);
            return;
        }
        if row.state != JobState::Done {
            return;
        }
        // Ruling 12: the art event is emitted after the commit, never inside it — `put_scene`
        // has already returned by the time `settle` runs.
        if job.kind == JobKind::J5Art {
            let payload = {
                let Ok(guard) = self.index.lock() else { return };
                crate::art::job::art_ready_payload(
                    &guard,
                    job.project_id.0,
                    crate::protocol::Rendition::Card,
                )
            };
            if let Some(data) = payload {
                self.events.emit("projects", "art_ready", data);
            }
        }
        let is_reference = self.read_is_reference(job.project_id);
        for (kind, priority) in next_jobs_after(job.kind, is_reference) {
            // Ruling 6: art takes a slot keyed apart from the repository's own, so a card does
            // not spend a git slot on a store it never touches.
            let store_key = if kind == JobKind::J5Art {
                crate::art::art_store_key(&job.store_key)
            } else {
                job.store_key.clone()
            };
            self.enqueue(Job {
                kind,
                project_id: job.project_id,
                location_id: job.location_id,
                store_key,
                store_kind: job.store_kind,
                priority,
                not_before: 0,
            });
        }
    }

    fn with_index<T>(
        &self,
        f: impl FnOnce(&rusqlite::Connection) -> Result<T, crate::index::IndexError>,
    ) -> Result<T, crate::index::IndexError> {
        let guard = self
            .index
            .lock()
            .map_err(|_| crate::index::IndexError::Corrupt {
                detail: "index lock is poisoned".to_owned(),
            })?;
        let out = guard.read(f);
        drop(guard);
        out
    }

    fn write_index<T>(
        &self,
        f: impl FnOnce(&rusqlite::Transaction<'_>) -> Result<T, crate::index::IndexError>,
    ) -> Result<T, crate::index::IndexError> {
        let mut guard = self
            .index
            .lock()
            .map_err(|_| crate::index::IndexError::Corrupt {
                detail: "index lock is poisoned".to_owned(),
            })?;
        let out = guard.with_tx(f);
        drop(guard);
        out
    }

    /// `None` when authorship has not been computed. The `authored_by_user IS NOT NULL` clause
    /// is what makes the query answer "not computed" rather than the column's `DEFAULT 0`.
    fn read_is_reference(&self, project: ProjectId) -> Option<bool> {
        self.with_index(|conn| {
            conn.query_row(
                "SELECT is_reference FROM project
                  WHERE id = ?1 AND authored_by_user IS NOT NULL",
                [project.0],
                |r| r.get::<_, i64>(0),
            )
            .map(|v| v != 0)
            .map_err(crate::index::IndexError::Sqlite)
        })
        .ok()
    }
}

fn git_error_kind(e: &crate::git::GitError) -> &'static str {
    use crate::git::GitError;
    match e {
        GitError::Untrusted { .. } => "UNTRUSTED_REPO",
        GitError::PermissionDenied { .. } => "PERMISSION_DENIED",
        GitError::Missing => "GIT_MISSING",
        GitError::TooOld { .. } => "GIT_TOO_OLD",
        GitError::StoreOffline { .. } => "STORE_OFFLINE",
        GitError::PathGone { .. } => "PATH_GONE",
        GitError::Budget { .. } => "BUDGET_EXCEEDED",
        GitError::Unreadable { .. }
        | GitError::Stale { .. }
        | GitError::Busy { .. }
        | GitError::TornRead
        | GitError::Cancelled
        | GitError::Internal { .. } => "REPO_UNREADABLE",
    }
}

impl JobSink for JobRunner {
    fn on_location_indexed(
        &self,
        project: ProjectId,
        location: LocationId,
        store_key: &str,
        store_kind: StoreClass,
    ) {
        self.enqueue(Job {
            kind: JobKind::J1Refstate,
            project_id: project,
            location_id: location,
            store_key: store_key.to_owned(),
            store_kind,
            priority: Priority::RefState,
            not_before: 0,
        });
    }

    fn on_visible(
        &self,
        project: ProjectId,
        location: LocationId,
        store_key: &str,
        store_kind: StoreClass,
        needs_art: bool,
    ) {
        // §6: visible tiles re-observe on scroll-idle and on window focus, and an opened page
        // re-observes the location it is showing. Worktree state is never cacheable, so this
        // always runs — it is not gated on a fingerprint.
        self.enqueue(Job {
            kind: JobKind::J2Status,
            project_id: project,
            location_id: location,
            store_key: store_key.to_owned(),
            store_kind,
            priority: Priority::Interactive,
            not_before: 0,
        });

        // §7.5: a stale, failed, missing or absent card is redrawn when its tile comes into
        // view, at the priority of the thing the user is looking at. This is what makes a
        // schema bump repaint shelf-visible first rather than four hundred cards at once on
        // the launch after an update.
        //
        // **The answer arrives as an argument and this method takes no lock.** It is called under
        // the one index guard on every real path, and `std::sync::Mutex` is not reentrant.
        if needs_art {
            self.enqueue(Job {
                kind: JobKind::J5Art,
                project_id: project,
                location_id: location,
                store_key: crate::art::art_store_key(store_key),
                store_kind,
                priority: Priority::Interactive,
                not_before: 0,
            });
        }
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
    use super::*;

    #[test]
    fn authorship_is_chained_immediately_after_refstate() {
        // §4.1a: J1.5 runs immediately after J1 and BEFORE J2 and J3. v2 had authorship as
        // the lowest-priority job of all, which is exactly backwards.
        let next = next_jobs_after(JobKind::J1Refstate, None);
        assert_eq!(next, vec![(JobKind::J15Authorship, Priority::Authorship)]);
    }

    #[test]
    fn a_reference_repository_drops_to_the_bottom_for_status_and_inventory() {
        let next = next_jobs_after(JobKind::J15Authorship, Some(true));
        assert!(next.contains(&(JobKind::J2Status, Priority::Reference)));
        assert!(next.contains(&(JobKind::J3Inventory, Priority::Reference)));
    }

    #[test]
    fn a_repository_with_the_users_commits_keeps_standard_priority() {
        let next = next_jobs_after(JobKind::J15Authorship, Some(false));
        assert!(next.contains(&(JobKind::J2Status, Priority::Standard)));
        assert!(next.contains(&(JobKind::J3Inventory, Priority::Standard)));
    }

    #[test]
    fn uncomputed_authorship_is_not_treated_as_reference() {
        // authored_by_user NULL means "not computed" (§1.2). Deprioritising on it would
        // silently exclude a repository nothing has managed to read yet.
        let next = next_jobs_after(JobKind::J15Authorship, None);
        assert!(next.contains(&(JobKind::J2Status, Priority::Standard)));
    }

    #[test]
    fn history_is_queued_after_authorship_and_never_before_it() {
        let next = next_jobs_after(JobKind::J15Authorship, Some(false));
        assert!(next.contains(&(JobKind::J4History, Priority::Deferred)));
        assert!(!next_jobs_after(JobKind::J1Refstate, None)
            .iter()
            .any(|(k, _)| *k == JobKind::J4History));
    }

    #[test]
    fn content_waits_for_the_inventory_that_names_its_language() {
        // J6's synthesised description reads primary_language and archetype, which J3 writes.
        let next = next_jobs_after(JobKind::J3Inventory, Some(false));
        assert!(next.contains(&(JobKind::J6Content, Priority::Deferred)));
    }

    #[test]
    fn art_is_queued_after_the_inventory_that_gives_it_its_size_bucket() {
        // §7.6: the card rendition is rendered "during the scan, at J3", and §7.2's seed inputs
        // are exactly what J3 writes.
        let next = next_jobs_after(JobKind::J3Inventory, Some(false));
        assert!(next.contains(&(JobKind::J5Art, Priority::Deferred)));
        // Art chains from nothing else, and chains nothing further.
        assert!(next_jobs_after(JobKind::J5Art, Some(false)).is_empty());
        assert!(!next_jobs_after(JobKind::J1Refstate, None)
            .iter()
            .any(|(k, _)| *k == JobKind::J5Art));
    }

    #[test]
    fn an_art_job_does_not_spend_the_repositorys_own_slot() {
        // Ruling 6 / §4.1: J5 is a CPU job off the git path.
        assert_eq!(crate::art::art_store_key("vol-a"), "art:vol-a");
        assert_ne!(crate::art::art_store_key("vol-a"), "vol-a");
    }

    #[test]
    fn a_finished_job_chains_nothing_further_from_status_or_content() {
        assert!(next_jobs_after(JobKind::J2Status, Some(false)).is_empty());
        assert!(next_jobs_after(JobKind::J6Content, Some(false)).is_empty());
        assert!(next_jobs_after(JobKind::J4History, Some(false)).is_empty());
    }

    /// **R39, held mechanically.** The whole ruling is that `Arc<Index>` is not `Send`, so a
    /// worker thread cannot hold it. This asserts the type that replaced it really is.
    #[test]
    fn the_runner_can_cross_a_thread_boundary() {
        const fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Arc<JobRunner>>();
        assert_send_sync::<Arc<Mutex<Index>>>();
    }
}
