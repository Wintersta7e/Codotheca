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
use super::{run_one, Job, JobDeps, JobKind, JobOrigin, JobOutcome, JobSink, JobState, Priority};
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
        JobKind::J15Authorship => {
            let mut next = vec![
                (JobKind::J2Status, band),
                (JobKind::J3Inventory, band),
                (JobKind::J4History, Priority::Deferred),
            ];
            // §29.7 predicate 1, and **the skip goes here** (R131/F14): this arm is the first
            // point at which `is_reference` is known, and this function otherwise only lowers
            // the band — so a J7 pushed unconditionally would be enqueued for a Reference
            // project, run, self-gate, and still leave a `project_job_state` row behind through
            // `settle`. **`None` is *not computed* and is not Reference.**
            //
            // This is what removes the measured worst case — a one-commit clone of someone
            // else's 14,000-file project — from the workload rather than budgeting for it.
            if is_reference == Some(false) {
                next.push((JobKind::J7Markers, Priority::Deferred));
            }
            next
        }
        JobKind::J3Inventory => vec![
            (JobKind::J6Content, Priority::Deferred),
            (JobKind::J5Art, Priority::Deferred),
        ],
        // J7 is a leaf: nothing chains off a content scan. Where it is *enqueued* is §29.7's
        // three sites and none of them is here — see the `J15Authorship` arm above.
        JobKind::J2Status
        | JobKind::J4History
        | JobKind::J5Art
        | JobKind::J6Content
        | JobKind::J7Markers => Vec::new(),
    }
}

/// §28.2's singleton evaluator — **R145's first of two call sites, both §28's** — with §34's
/// `health_delta` producer around it, in the settle's own transaction.
///
/// Every job-fed arm answers here: the three `missing_*` and `unpushed_commits`. The producer's
/// first snapshot is taken before the debt write — reading only after it would leave no `before`
/// to recover — and the row commits with the debt set that moved it or not at all. The provenance
/// is the chain's origin, never the priority. The event it returns is announced after the commit.
fn settle_debt(
    tx: &rusqlite::Transaction<'_>,
    job: &Job,
    now: i64,
    tz_offset_min: i32,
) -> Result<Option<crate::protocol::ProjectHealthDelta>, crate::index::IndexError> {
    let layers_before = crate::restoration::LayerValues::read(tx, job.project_id)?;
    let effect = crate::debt::singletons::settle_singletons(
        tx,
        job.project_id,
        now,
        tz_offset_min,
        &crate::debt::store::SqliteDebtStore,
    )
    .map_err(debt_to_index)?;
    crate::restoration::record_after_write(
        tx,
        job.project_id,
        &layers_before,
        &effect.closed,
        crate::restoration::detected_in_for(job.origin),
        now,
    )
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
        let announce = std::cell::RefCell::new(Vec::new());
        let result = run_one(self.index.as_ref(), &self.deps, job, &announce);
        // [p3] §34: J7 writes its deltas inside its own transaction, which has committed by the
        // time `run_one` returns — so they are announced here, never from inside it.
        for delta in announce.into_inner() {
            crate::restoration::emit_health_delta(self.events.as_ref(), &delta);
        }
        match result {
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

        // [p3] R121's first conjunct: the projected row **as it stood before** the write
        // transaction. Whole-row equality, never a hand-maintained list of *"fields a settle can
        // move"* — a list like that is a count a human maintains and goes stale on the first plan
        // that adds a field. It is only read on an interactive chain, so a walk pays nothing.
        let before = (job.origin == JobOrigin::Interactive)
            .then(|| {
                self.with_index(|conn| Ok(crate::detail::upserted_payload(conn, job.project_id)))
                    .ok()
                    .flatten()
            })
            .flatten();

        // The job row and the recompute land in one transaction: a settled job whose derived
        // values were not rewritten is a project the shelf sections into the wrong era.
        let (row, requeue_at) = apply_outcome(&previous, outcome, now);
        let tz_offset_min = self.deps.tz_offset_min;
        let recomputed = self.write_index(|tx| {
            put(tx, job.project_id, &row)?;
            // §28.2's singleton evaluator, and §34's producer around it. It runs **before** §31's
            // completion evaluator, or every Group-A check answers from the previous settle; do
            // not reorder them.
            let announce = settle_debt(tx, job, now, tz_offset_min)?;
            let recomputed = crate::derive::persist::recompute(tx, job.project_id, now)?;
            // [p3] §31.5's evaluator — **hook site 1 of exactly two** (R123). None of §31.5's
            // three triggers is observable: `coverage_for` returns two booleans and
            // `next_jobs_after` returns empty for four job kinds, so *the last input job* is a
            // thing nothing can report. A settle hook is countable, and `completion_hooks.rs`
            // counts it.
            //
            // **After `settle_singletons` above, and the order is load-bearing**: six of §31's
            // ten checks read what §28 wrote in this same transaction (R124), so reversing the
            // two would make every Group-A check answer from the PREVIOUS settle — a one-settle
            // lag no test of either plan alone would catch, because §28's tests see correct rows
            // written and §31's see rows that are merely stale.
            //
            // **It is not folded into `recompute`**: that has other callers, and a hook firing
            // from an unbounded set is a hook nobody can count.
            crate::completion::evaluate_and_write(tx, job.project_id, now)?;
            Ok((recomputed, announce))
        });
        if let Ok((r, _)) = &recomputed {
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

        self.publish_row_change(job, before);

        // [p3] §34.4: after the commit, beside the state's own transport and never instead of it.
        if let Ok((_, Some(delta))) = &recomputed {
            crate::restoration::emit_health_delta(self.events.as_ref(), delta);
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
            // [p3] R121: `clone` carries `origin`, so a backoff requeue keeps the chain's owner.
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
                // [p3] R121: copied, never re-derived. A chain a user started stays the user's
                // to its last job, which is what makes the payoff path publish.
                origin: job.origin,
            });
        }
    }

    /// [p3] R121, **both conjuncts**: the row actually changed, *and* the chain came from a user
    /// looking at something. Either alone is wrong — an ungated emit is the firehose phase 1
    /// refused, and a change gate alone re-creates it on a first scan, where the row genuinely
    /// changes on almost every settle.
    ///
    /// `before` is `None` on a walk chain, which is the second conjunct: the walk pays nothing
    /// for a comparison it would never publish.
    ///
    /// This is the path §29.7 put there — `projects.get` → `on_visible` → J7 at `Standard` → this
    /// settle. Closing a TODO, reopening the page and watching the card change is that path end
    /// to end.
    fn publish_row_change(&self, job: &Job, before: Option<serde_json::Value>) {
        let Some(before) = before else { return };
        let after = self
            .with_index(|conn| Ok(crate::detail::upserted_payload(conn, job.project_id)))
            .ok()
            .flatten();
        let Some(after) = after else { return };
        if after != before {
            self.events.emit("projects", "upserted", after);
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

    /// `None` when authorship has not been computed.
    fn read_is_reference(&self, project: ProjectId) -> Option<bool> {
        self.with_index(|conn| is_reference(conn, project))
            .ok()
            .flatten()
    }
}

/// Whether authorship resolved this project to Reference, or `None` when it has not been
/// computed.
///
/// **`None` is *not computed* and is not Reference.** The `authored_by_user IS NOT NULL` clause is
/// what makes the query answer that rather than the column's `DEFAULT 0` — and it is why §29.7's
/// first predicate is `Some(false)` rather than a falsy check.
///
/// One owner, because §29.7's gate and the scheduler's band both ask it (R12).
///
/// # Errors
/// Fails when SQLite refuses the read.
pub fn is_reference(
    conn: &rusqlite::Connection,
    project: ProjectId,
) -> Result<Option<bool>, crate::index::IndexError> {
    conn.query_row(
        "SELECT is_reference FROM project
          WHERE id = ?1 AND authored_by_user IS NOT NULL",
        [project.0],
        |r| r.get::<_, i64>(0),
    )
    .map(|v| Some(v != 0))
    .or_else(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => Ok(None),
        other => Err(crate::index::IndexError::Sqlite(other)),
    })
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
            // [p3] R121: the walk publishes nothing. `scan/finished` covers the bulk case.
            origin: JobOrigin::Walk,
        });
    }

    fn on_visible(
        &self,
        project: ProjectId,
        location: LocationId,
        store_key: &str,
        store_kind: StoreClass,
        needs_art: bool,
        wants_content: bool,
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
            origin: JobOrigin::Interactive,
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
                origin: JobOrigin::Interactive,
            });
        }

        // §29.7's second site. `Standard`, not `Interactive`: no surface waits on it, and the
        // opened page is the only visibility site that asks — Peek is the triage surface over the
        // unsorted backlog, where health is suppressed until a verdict, so doing the read there
        // performs precisely the work whose output is suppressed.
        if wants_content {
            self.enqueue(Job {
                kind: JobKind::J7Markers,
                project_id: project,
                location_id: location,
                store_key: store_key.to_owned(),
                store_kind,
                priority: Priority::Standard,
                not_before: 0,
                origin: JobOrigin::Interactive,
            });
        }
    }
}

/// §28's error, in the vocabulary `write_index` takes.
fn debt_to_index(error: crate::debt::DebtError) -> crate::index::IndexError {
    match error {
        crate::debt::DebtError::Index(inner) => inner,
        crate::debt::DebtError::Codec(detail) => {
            crate::index::IndexError::Sqlite(rusqlite::Error::InvalidParameterName(detail))
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
