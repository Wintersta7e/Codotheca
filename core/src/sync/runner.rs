//! The single-threaded sync runner, and the pump that owns its thread.
//!
//! One dedicated blocking thread. It wakes, promotes every `parked` row whose clock has come,
//! picks the single highest-priority runnable row — **on-demand before scheduled**, then
//! `not_before` ascending — moves it to `running`, executes it, drains the observing transport,
//! and settles it. **At most one row is `running` in the process**, and at most one HTTP request
//! is in flight.
//!
//! **Cancel then join, in that order**, copying `JobPump::stop`
//! (`core/src/assembly/jobs.rs:99-108`) exactly: `request_stop` alone only asks the thread to take
//! no *further* task.
//!
//! **What cancelling does here, stated because it differs from `JobPump`'s.** A job's cancel takes
//! down a git process tree, so a worker parked in a twenty-second history read returns at once.
//! There is no process to kill for an HTTP request: the bound is the per-request budget
//! `crate::http::ACCOUNT_LIMITS.total_secs` — **30 seconds** — and cancelling stops the loop
//! taking further work rather than aborting the request in flight. So `stop()` is bounded, not
//! immediate, and the bound is a number `core/tests/http_transport.rs` already pins.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::Duration;

use crate::index::{Index, IndexError};
use crate::proto::EventSink;
use crate::protocol::{
    AccountId, ProjectId, SyncNotice, SyncTaskKind, SyncTaskStarted, SyncTaskState,
};
use crate::sync::budget::{may_spend, read_budget, BudgetVerdict};
use crate::sync::events::{
    emit_budget, emit_notice, emit_progress, emit_settled, emit_started, settled_of, LastSettle,
    SyncLive,
};
use crate::sync::outcome::{SyncOutcome, UnauthorizedReason};
use crate::sync::progress::ListingProgress;
use crate::sync::state::{apply_outcome, SyncTaskStateRow};
use crate::sync::store::{load, load_all, put, requeue_running};
use crate::sync::task::SyncTask;
use crate::sync::tasks::{remote, rename, repos};
use crate::sync::{is_on_demand, SyncDeps, SyncError};

/// How long the loop sleeps **while work is outstanding** and nothing is runnable yet.
///
/// Short, because the wake that matters is a `parked` row's clock coming round and no event
/// announces that (§21.4: *"a parked row returns to queued by the clock alone"*).
const IDLE_POLL: Duration = Duration::from_millis(50);

/// How long the loop sleeps when the table holds **nothing** — no queued row, no park, no
/// interrupted row.
///
/// **An idle runner must not touch the index at all.** There is one `rusqlite::Connection` behind
/// one mutex, and a loop that re-read the table twenty times a second would contend with every
/// command for it — for a table it already knows is empty. So the loop tracks whether anything is
/// outstanding and, when nothing is, waits on the condvar without taking the guard; `enqueue` and
/// `request_stop` are what wake it, and this ceiling is only the floor under a missed signal.
///
/// This was found by a p2-20 test, not by design: `a_network_account_command_holds_no_index_lock_
/// while_it_is_in_flight` probes whether the mutex is free at the moment a request reaches the
/// transport, and an idle sync runner made it free only between polls.
const IDLE_WAIT: Duration = Duration::from_secs(1);

/// How often the loop asks whether an account has come due for a listing (§21.5).
///
/// **This is not the cadence.** The cadence is six hours and `due_listings` owns it. This is how
/// long the *other* three triggers §21.5 names — on connect, on scope upgrade, on an org opt-in
/// change — wait to be noticed, and it is a poll rather than a signal for a reason worth stating:
/// each of those three completes on a p2-20 thread that has no handle on this runner. A connect
/// finishes inside `crate::accounts::pump`'s worker, not in the command arm that started it, so
/// there is no call site in this plan's reach that could signal instead. **Recorded as a
/// deviation**: the honest shape is an account-lifecycle signal, and it needs a plan that owns
/// that seam.
///
/// A minute is the delay a user sees between connecting an account and its listing starting, and
/// it is one small `SELECT` per minute against the process's one index mutex — against the twenty
/// per second [`IDLE_WAIT`] exists to prevent.
const SCHEDULE_POLL_SECS: i64 = 60;

/// The pool §21.6 keys a budget by when the response named no resource of its own.
///
/// Reading the budget before issuing needs a resource name, and the only one this process can know
/// in advance is the one it last saw. `core` is the forge's own name for the ordinary REST pool
/// and is what every response this runner makes carries; an unobserved pool answers `Unknown`,
/// which spends, so a wrong guess here costs one request and corrects itself.
const DEFAULT_RESOURCE: &str = "core";

/// The seam the two on-demand enqueue sites call.
///
/// A trait rather than a concrete type so `projects.get` and `projects.peek` can be driven with a
/// recording sink, and **so the composition root is the only thing that decides whether the real
/// runner is behind it** — the shape `JobSink` already has.
pub trait SyncSink: Send + Sync + std::fmt::Debug {
    /// A project page or a Peek opened. §21.5: at the priority of the thing the user is looking
    /// at, and from **exactly** two call sites.
    fn on_project_visible(&self, project: ProjectId);
}

/// The sink a build with no runner holds. It records nothing and queues nothing, which is what a
/// core with no accounts should do.
#[derive(Debug, Clone, Copy)]
pub struct NullSyncSink;

impl SyncSink for NullSyncSink {
    fn on_project_visible(&self, _project: ProjectId) {}
}

#[derive(Debug, Default)]
struct Waiters {
    /// Set by `enqueue` and by `request_stop`, cleared by the loop. A plain flag rather than a
    /// count: the loop re-reads the table on every wake, so one wake settles any number of
    /// enqueues.
    signalled: bool,
}

/// The runner.
pub struct SyncRunner {
    index: Arc<Mutex<Index>>,
    deps: SyncDeps,
    events: Arc<dyn EventSink>,
    stopping: AtomicBool,
    /// Whether the table holds anything the loop could still act on. See [`IDLE_WAIT`].
    outstanding: AtomicBool,
    /// The epoch second at which the loop next consults `due_listings`. See
    /// [`SCHEDULE_POLL_SECS`]. Atomic rather than behind the `waiters` mutex so the common case —
    /// *not yet* — costs one load and never a lock.
    schedule_due_at: AtomicI64,
    waiters: Mutex<Waiters>,
    wake: Condvar,
    handle: Mutex<Option<std::thread::JoinHandle<()>>>,
    /// The process-lifetime half of `SyncStatus`. There is no column for any of it, and there must
    /// not be: a listing's progress is meaningless across a restart and a notice restored from a
    /// table would claim a failure nothing has re-observed.
    live: Mutex<SyncLive>,
    /// One in-flight listing's running tally, keyed by account. It accumulates across the pages of
    /// one listing and is emitted **once**, at settle, with the rest of the summary.
    listings: Mutex<HashMap<i64, (ListingProgress, repos::ListingSummary)>>,
    /// Tasks asked for but not yet written to the table.
    ///
    /// **This exists so `enqueue` takes no index lock** (R94's first side). `on_project_visible`
    /// is reached from `projects.get` and `projects.peek`, and `Assembly` dispatches both while
    /// holding the process's one `Arc<Mutex<Index>>` guard — `std::sync::Mutex` is not reentrant,
    /// so a sink that locked it again would wedge the guard for the life of the process and the
    /// project page would simply never arrive. That is `f182452`'s defect, which `JobSink` already
    /// paid for; the e2e suite caught this lane repeating it, and the unit tests did not, because
    /// they drive a recording sink rather than the runner.
    ///
    /// The **loop** drains this inside the transaction it already holds, which is R94's second
    /// side: a worker thread must lock, because nothing above it does.
    inbox: Mutex<VecDeque<SyncTask>>,
}

impl std::fmt::Debug for SyncRunner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SyncRunner").finish_non_exhaustive()
    }
}

impl SyncRunner {
    #[must_use]
    pub fn new(index: Arc<Mutex<Index>>, deps: SyncDeps, events: Arc<dyn EventSink>) -> Arc<Self> {
        Arc::new(Self {
            index,
            deps,
            events,
            stopping: AtomicBool::new(false),
            outstanding: AtomicBool::new(false),
            // `start` sweeps the schedule itself, so the loop's first poll is one interval later.
            schedule_due_at: AtomicI64::new(i64::MIN),
            waiters: Mutex::new(Waiters::default()),
            wake: Condvar::new(),
            handle: Mutex::new(None),
            live: Mutex::new(SyncLive::default()),
            listings: Mutex::new(HashMap::new()),
            inbox: Mutex::new(VecDeque::new()),
        })
    }

    /// Sweep the crash leftovers and spawn the one thread.
    ///
    /// **AC-P2-21-12.** The sweep runs **once**, here, inside one transaction, and moves every
    /// `running` row to `queued` with `not_before = 0`. Every sync task is a GET, so it is
    /// idempotent — unlike the operations §2.2 forbids replaying — and an interrupted one is
    /// re-runnable rather than a failure to report.
    ///
    /// **§21.5's start-up trigger is the same transaction.** *"At start-up when the last settled
    /// run is older than the interval"* is a question about the table, and asking it here rather
    /// than on the loop's first turn means a process that starts with nothing due still never
    /// takes the guard twice.
    pub fn start(self: &Arc<Self>) {
        let now = self.deps.clock.now_unix();
        {
            let mut guard = self.index.lock().unwrap_or_else(PoisonError::into_inner);
            // The sweep, the schedule and the first *is anything outstanding* read are one
            // transaction, so a loop that starts against an empty table never takes the guard at
            // all.
            if let Ok((moved, outstanding)) = guard.with_tx(|tx| {
                let moved = requeue_running(tx, now)?;
                sweep_schedule(tx, now)?;
                Ok((moved, any_outstanding(tx)?))
            }) {
                if moved > 0 {
                    // Diagnostic only, and on stderr: stdout carries protocol frames and nothing
                    // else.
                    eprintln!("sync: re-queued {moved} task(s) interrupted by a restart");
                }
                // **Or the inbox**: `enqueue` may have been called before the pump started,
                // and a flag set from the table alone would clear it and leave the loop
                // asleep over work it had already been handed.
                self.outstanding
                    .store(outstanding || !self.inbox_is_empty(), Ordering::SeqCst);
            }
        }

        self.schedule_due_at
            .store(now.saturating_add(SCHEDULE_POLL_SECS), Ordering::SeqCst);

        let me = Arc::clone(self);
        let handle = std::thread::Builder::new()
            .name("codotheca-sync".to_owned())
            .spawn(move || me.run_loop())
            .ok();
        *self.handle.lock().unwrap_or_else(PoisonError::into_inner) = handle;
    }

    /// Ask the loop to take no further task.
    pub fn request_stop(&self) {
        self.stopping.store(true, Ordering::SeqCst);
        self.signal();
    }

    /// Wait for the thread. Idempotent: a second call finds the handle already taken.
    pub fn join(&self) {
        let handle = self
            .handle
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(handle) = handle {
            let _ = handle.join();
        }
    }

    /// Queue one task.
    ///
    /// **It takes no index lock, and that is load-bearing** — see [`SyncRunner::inbox`]. The task
    /// is recorded in memory and the loop writes the row, so a caller already under the index
    /// guard (which both `on_project_visible` sites are) cannot deadlock the process.
    ///
    /// **No priority parameter**, which deviates from this plan's task table. §21.5's priority is
    /// `crate::sync::is_on_demand`, a property of the task's own kind; a `Priority` argument would
    /// let a caller contradict the kind, and no caller needs to.
    pub fn enqueue(&self, task: SyncTask) {
        self.inbox
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(task);
        self.outstanding.store(true, Ordering::SeqCst);
        self.signal();
    }

    /// Write every task the inbox holds, applying §21.4's re-queue rule.
    ///
    /// A task already `queued`, `running` or `parked` is left alone — re-queueing a parked row
    /// would discard the instant the server named. `blocked` is left alone too: it is left only
    /// through an account state change or an explicit user action, never by something asking
    /// again.
    fn drain_inbox(&self, tx: &rusqlite::Transaction<'_>, now: i64) -> Result<(), IndexError> {
        let pending: Vec<SyncTask> = {
            let mut inbox = self.inbox.lock().unwrap_or_else(PoisonError::into_inner);
            inbox.drain(..).collect()
        };
        for task in pending {
            let kind = task.kind();
            let key = task.key();
            let existing = load(tx, kind, key)?;
            if matches!(
                existing.as_ref().map(|row| row.state),
                Some(
                    SyncTaskState::Queued
                        | SyncTaskState::Running
                        | SyncTaskState::Parked
                        | SyncTaskState::Blocked
                )
            ) {
                continue;
            }
            put(tx, &SyncTaskStateRow::queued(kind, key, now))?;
        }
        Ok(())
    }

    /// A snapshot of what only this process knows.
    #[must_use]
    pub fn live(&self) -> SyncLive {
        self.live
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn inbox_is_empty(&self) -> bool {
        self.inbox
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_empty()
    }

    fn signal(&self) {
        let mut waiters = self.waiters.lock().unwrap_or_else(PoisonError::into_inner);
        waiters.signalled = true;
        drop(waiters);
        self.wake.notify_all();
    }

    fn should_stop(&self) -> bool {
        self.stopping.load(Ordering::SeqCst) || self.deps.cancel.is_cancelled()
    }

    fn run_loop(self: &Arc<Self>) {
        while !self.should_stop() {
            let now = self.deps.clock.now_unix();
            if now >= self.schedule_due_at.load(Ordering::SeqCst) {
                self.poll_schedule(now);
            }
            // **No lock on an empty table.** `enqueue` and `request_stop` both signal, so a wake
            // that matters still arrives at once; the ceiling is only the floor under a missed
            // one.
            if !self.outstanding.load(Ordering::SeqCst) {
                self.idle(IDLE_WAIT);
                continue;
            }
            match self.take_next() {
                Some(task) => self.run_one(task),
                None => self.idle(IDLE_POLL),
            }
        }
    }

    fn idle(&self, ceiling: Duration) {
        let waiters = self.waiters.lock().unwrap_or_else(PoisonError::into_inner);
        let (mut waiters, _) = self
            .wake
            .wait_timeout(waiters, ceiling)
            .unwrap_or_else(PoisonError::into_inner);
        waiters.signalled = false;
    }

    /// Ask the table which accounts are due a listing, and queue one row for each.
    ///
    /// **This is what makes §21.5's cadence real in the product.** Without it `due_listings` has
    /// no production caller, no `account_repos` row is ever written, and the whole of §21 is
    /// reachable only from a test — R90 on this plan's own primary deliverable.
    ///
    /// The horizon moves whether or not the read succeeded: a failing read must not turn this
    /// into a per-turn query against the one index mutex.
    fn poll_schedule(&self, now: i64) {
        let swept = {
            let mut guard = self.index.lock().unwrap_or_else(PoisonError::into_inner);
            guard.with_tx(|tx| {
                let queued = sweep_schedule(tx, now)?;
                Ok((queued, any_outstanding(tx)?))
            })
        };
        self.schedule_due_at
            .store(now.saturating_add(SCHEDULE_POLL_SECS), Ordering::SeqCst);
        if let Ok((queued, outstanding)) = swept {
            if queued > 0 {
                self.outstanding.store(true, Ordering::SeqCst);
            } else {
                self.outstanding
                    .store(outstanding || !self.inbox_is_empty(), Ordering::SeqCst);
            }
        }
    }

    /// Promote the parks whose clock has come, then claim the one task to run.
    ///
    /// Promotion and the claim are **one transaction**, so no second reader can see a row this
    /// thread has promoted but not yet claimed. There is only one such thread today; the
    /// transaction is what keeps that a property of the code rather than of the thread count.
    fn take_next(&self) -> Option<SyncTask> {
        let now = self.deps.clock.now_unix();
        let mut guard = self.index.lock().unwrap_or_else(PoisonError::into_inner);
        guard
            .with_tx(|tx| {
                // Everything asked for since the last wake, written here where the lock is
                // already held rather than by the caller that asked.
                self.drain_inbox(tx, now)?;
                for row in load_all(tx)? {
                    if row.is_due_park(now) {
                        let mut promoted = row.clone();
                        promoted.state = SyncTaskState::Queued;
                        promoted.at = now;
                        // No event: a parked row that becomes runnable has not *settled*.
                        put(tx, &promoted)?;
                    }
                }

                let mut runnable: Vec<(SyncTask, SyncTaskStateRow)> = load_all(tx)?
                    .into_iter()
                    .filter(|row| row.is_runnable(now))
                    .filter_map(|row| task_of(&row).map(|task| (task, row)))
                    .collect();
                // **On-demand first, then the oldest deadline.** The predicate is
                // `crate::sync::is_on_demand` and not a second expression of it: the ordering and
                // the reserve have to agree about which tasks are on demand, or the reserve would
                // hold an allowance for work this loop then deprioritises. `sort_by_key` is
                // stable, so equal keys keep `load_all`'s `(not_before, id)` order.
                runnable.sort_by_key(|(task, row)| (u8::from(!is_on_demand(task)), row.not_before));
                let picked = runnable.into_iter().next();
                let Some((_, row)) = picked else {
                    // Nothing runnable now; a park whose clock has not come still counts as
                    // outstanding, and a table with neither stops the polling entirely.
                    return Ok((None, any_outstanding(tx)?));
                };
                let mut running = row.clone();
                running.state = SyncTaskState::Running;
                running.at = now;
                put(tx, &running)?;
                Ok((task_of(&row), true))
            })
            .map_or(None, |(task, outstanding)| {
                self.outstanding
                    .store(outstanding || !self.inbox_is_empty(), Ordering::SeqCst);
                task
            })
    }

    fn run_one(self: &Arc<Self>, task: SyncTask) {
        let kind = task.kind();
        let key = task.key();
        emit_started(self.events.as_ref(), &SyncTaskStarted { kind, key });

        let verdict = self.budget_verdict(&task);
        let outcome = match verdict {
            // Both park to the instant the budget named, and **neither is secondary**: the
            // server did not name a `Retry-After`, this process declined to spend. What separates
            // them is the `reason` the settle writes, which `reserved` below carries.
            BudgetVerdict::ParkUntil(until) | BudgetVerdict::Reserved(until) => {
                Ok(SyncOutcome::Throttled {
                    until,
                    secondary: false,
                })
            }
            BudgetVerdict::Spend | BudgetVerdict::Unknown => self.execute(&task),
        };
        let reserved = matches!(verdict, BudgetVerdict::Reserved(_));
        self.settle(&task, outcome, reserved);
    }

    /// §21.6's *before issuing* rule, and §21.5's reserve.
    fn budget_verdict(&self, task: &SyncTask) -> BudgetVerdict {
        let now = self.deps.clock.now_unix();
        let guard = self.index.lock().unwrap_or_else(PoisonError::into_inner);
        let account = match task {
            SyncTask::AccountRepos { account_id } | SyncTask::RenameProbe { account_id } => {
                Some(*account_id)
            }
            // **The pool the read would actually spend from**, resolved by the one function that
            // decides which account reads a project. Checking the per-IP pool instead would guard
            // an allowance the request never touches, which is a reserve that reserves nothing.
            SyncTask::ProjectRemote { project_id } => {
                remote::account_for_project(guard.conn(), *project_id)
            }
            // [p3] §32.1: the sweep is unauthenticated by ruling and its provider method has
            // nowhere to put a token, so the pool it spends from is the **NULL-account per-IP**
            // one, always — never whichever account happens to be connected.
            SyncTask::Advisories => None,
        };
        let resource = resource_for(guard.conn(), task);
        let row = read_budget(guard.conn(), account, &resource).ok().flatten();
        may_spend(row.as_ref(), is_on_demand(task), now)
    }

    fn execute(&self, task: &SyncTask) -> Result<SyncOutcome, SyncError> {
        match task {
            SyncTask::AccountRepos { account_id } => {
                let cursor = {
                    let guard = self.index.lock().unwrap_or_else(PoisonError::into_inner);
                    load(guard.conn(), SyncTaskKind::AccountRepos, Some(account_id.0))
                        .ok()
                        .flatten()
                        .and_then(|row| row.cursor)
                };
                let (outcome, page) = repos::run_account_repos(
                    &self.deps,
                    self.index.as_ref(),
                    *account_id,
                    cursor.as_deref(),
                )?;
                self.accumulate(*account_id, &page);
                Ok(outcome)
            }
            SyncTask::ProjectRemote { project_id } => {
                remote::run_project_remote(&self.deps, self.index.as_ref(), *project_id)
            }
            SyncTask::RenameProbe { account_id } => {
                rename::run_rename_probe(&self.deps, &self.index, *account_id)
            }
            // [p3] §32.2's sweep. The cursor is this task's own: a page link while an answer is
            // still paginating, and a sentinel when the next batch is due. Each batch settles
            // `NextPage`, so the **next** pick re-reads the budget before issuing again — which is
            // what lets a library too large for one hour's allowance sweep across reset windows
            // instead of spending it all at once.
            SyncTask::Advisories => {
                let cursor = {
                    let guard = self.index.lock().unwrap_or_else(PoisonError::into_inner);
                    load(guard.conn(), SyncTaskKind::Advisories, None)
                        .ok()
                        .flatten()
                        .and_then(|row| row.cursor)
                };
                let outcome = crate::advisories::sweep::run_advisory_sweep(
                    &self.deps,
                    self.index.as_ref(),
                    cursor.as_deref(),
                );
                // §32.10: a closed sweep has asked about every triple, so each scanned project's
                // items are computed now — here, before `settle`, whose completion evaluator reads
                // them.
                if matches!(outcome, Ok(SyncOutcome::Done)) {
                    self.settle_advisory_debt();
                }
                outcome
            }
        }
    }

    /// §32.10's items for every scanned project, each wrapped by §34's `health_delta` producer,
    /// in one transaction, and the deltas announced after the commit — `settle_project_debt`'s
    /// shape for the sweep.
    ///
    /// A transaction of its own, so an item fault cannot keep the sweep open for ever: it is
    /// logged, and the next sweep computes the items again.
    fn settle_advisory_debt(&self) {
        let now = self.deps.clock.now_unix();
        let tz_offset_min = self.deps.tz_offset_min;
        let mut guard = self.index.lock().unwrap_or_else(PoisonError::into_inner);
        let deltas = guard.with_tx(|tx| {
            crate::advisories::items::settle_advisory_items(tx, now, tz_offset_min).map_err(|e| {
                match e {
                    crate::advisories::AdvisoryError::Index(inner) => inner,
                    other => IndexError::Corrupt {
                        detail: other.to_string(),
                    },
                }
            })
        });
        drop(guard);
        match deltas {
            Ok(deltas) => {
                for delta in &deltas {
                    crate::restoration::emit_health_delta(self.events.as_ref(), delta);
                }
            }
            Err(e) => eprintln!("sync: advisory items were not computed: {e}"),
        }
    }

    /// Fold one page into the listing's running tally and publish the count.
    ///
    /// §21.11's counter is monotone **by construction** — `ListingProgress::add` saturates upward
    /// and there is no setter — so a page that reports fewer entries cannot lower the figure.
    fn accumulate(&self, account: AccountId, page: &repos::ListingSummary) {
        let mut listings = self.listings.lock().unwrap_or_else(PoisonError::into_inner);
        let entry = listings.entry(account.0).or_insert_with(|| {
            (
                // `None`: no forge listing in phase 2 supplies a total, and a denominator inferred
                // from page numbers is a guess.
                ListingProgress::new(account, None),
                repos::ListingSummary::default(),
            )
        });
        entry.0.add(page.listed);
        entry.1.merge(page);
        let payload = entry.0.payload();
        drop(listings);

        self.live
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .listing = Some(payload.clone());
        emit_progress(self.events.as_ref(), &payload);
    }

    /// §28.2's singleton evaluator for one project, with §34's `health_delta` producer around it,
    /// in one transaction, and the event announced after the commit.
    ///
    /// A sync is a scheduled sweep, so whatever it observes is `background`. A snapshot that
    /// cannot be read records no delta and costs the singleton write nothing — the write was
    /// never gated on it before the producer existed.
    fn settle_project_debt(&self, project_id: ProjectId, now: i64) {
        let tz_offset_min = self.deps.tz_offset_min;
        let mut guard = self.index.lock().unwrap_or_else(PoisonError::into_inner);
        let announce = guard
            .with_tx(|tx| {
                let layers_before = crate::restoration::LayerValues::read(tx, project_id).ok();
                let Ok(effect) = crate::debt::singletons::settle_singletons(
                    tx,
                    project_id,
                    now,
                    tz_offset_min,
                    &crate::debt::store::SqliteDebtStore,
                ) else {
                    return Ok(None);
                };
                let Some(layers_before) = layers_before else {
                    return Ok(None);
                };
                crate::restoration::record_after_write(
                    tx,
                    project_id,
                    &layers_before,
                    &effect.closed,
                    crate::protocol::HealthDetectedIn::Background,
                    now,
                )
            })
            .ok()
            .flatten();
        drop(guard);
        if let Some(delta) = announce {
            crate::restoration::emit_health_delta(self.events.as_ref(), &delta);
        }
    }

    fn settle(&self, task: &SyncTask, outcome: Result<SyncOutcome, SyncError>, reserved: bool) {
        let now = self.deps.clock.now_unix();
        let kind = task.kind();
        let key = task.key();
        let outcome = outcome.unwrap_or_else(|e| SyncOutcome::TransientFail {
            reason: e.to_string(),
        });

        // The listing's accumulated summary, taken **once**, at the settle that ends it.
        let summary = match (task, &outcome) {
            (SyncTask::AccountRepos { account_id }, o)
                if !matches!(o, SyncOutcome::NextPage { .. }) =>
            {
                self.listings
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .remove(&account_id.0)
                    .map(|(_, summary)| summary)
            }
            _ => None,
        };
        if summary.is_some() {
            self.live
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .listing = None;
        }

        let settled_row = {
            let mut guard = self.index.lock().unwrap_or_else(PoisonError::into_inner);
            let before = load(guard.conn(), kind, key)
                .ok()
                .flatten()
                .unwrap_or_else(|| SyncTaskStateRow::queued(kind, key, now));
            let (mut next, _) = apply_outcome(&before, &outcome, now);
            if reserved {
                // §21.5: it did not fail and it was not throttled by the server — it yielded to
                // the reserve, and `reason` is what lets a status reader tell the three apart.
                next.reason = Some("reserve".to_owned());
            }
            let row = next.clone();
            let _ = guard.with_tx(|tx| put(tx, &next));
            row
        };

        // §22.7's trigger, and its only one: *"after a sync completes — on the terminal `Done`
        // outcome only, never on a `NextPage` and never on a throttled park, because 'no listing
        // matched' is not knowable until the listing ends"*
        // (`core/src/identity/rename_repair.rs:9`). Queued rather than called, so the repair gets
        // its own budget check, its own row and its own settle instead of riding on the listing's.
        if let (SyncTask::AccountRepos { account_id }, SyncOutcome::Done) = (task, &outcome) {
            self.enqueue(SyncTask::RenameProbe {
                account_id: *account_id,
            });
        }

        self.mirror_foreign_observations();

        // §28.2's singleton evaluator — **R145's second call site, and both are §28's.**
        // **`ci_red`'s input arrives on a sync, not on a job**, so an evaluator hooked to
        // job-settle alone holds §28's previous answer until some unrelated job settles that
        // project. `ProjectRemote` is the task that writes the run record, so it is the one
        // settle that can have changed the reading.
        //
        // It runs **before** §31's completion evaluator here too; do not reorder them.
        if let SyncTask::ProjectRemote { project_id } = task {
            self.settle_project_debt(*project_id, now);
        }

        // [p3] §31.5's evaluator — **hook site 2 of exactly two** (R123).
        //
        // **Two of §31's ten checks come from sync and one from a scheduled sweep**, so a
        // job-shaped trigger cannot fire for `description`, `ciGreen` or `deps` at all: a forge
        // description written here would otherwise sit unread until some unrelated job settled.
        //
        // The project set is resolved from the task, and **the account-scoped kinds fire on a
        // terminal outcome only, never on a `NextPage`** — the same rule §22.7's trigger applies
        // twelve lines above, for the same reason: a twenty-page listing would otherwise
        // re-evaluate every project of that account twenty times. **Correctness does not depend
        // on which settles fire** — the evaluator is idempotent and change-gated — only cost
        // does, and this is where the cost is decided.
        //
        // **This plan emits no event from `settle`.** The gated `projects.upserted` emit is
        // §30's (R121) and its gate is a different mechanism: the diff gate below decides
        // whether ten rows are *written*, R121's two conjuncts decide whether a row change is
        // *announced*. A recompute during a bulk first scan writes rows and announces nothing,
        // which is correct — the bulk case stays covered by `scan/finished`.
        let projects = self.projects_for_sync_task(task, &outcome);
        if !projects.is_empty() {
            let mut guard = self.index.lock().unwrap_or_else(PoisonError::into_inner);
            let _ = guard.with_tx(|tx| {
                for project in &projects {
                    let _ = crate::completion::evaluate_and_write(tx, *project, now);
                }
                Ok(())
            });
        }

        // [p3] §32.12's **one** notification, decided at the settle that could have changed it.
        //
        // The core observes the transition and emits; `app/src/main` posts. The renderer's
        // `notifications` permission stays denied and nothing in `app/src/renderer` may originate
        // an OS notification — the same invariant as *the renderer may never originate a
        // filesystem path or an executable*, applied to the one interruption the product has.
        //
        // **At most one per settle**, and the ledger it consumes is written in the same
        // transaction, so a crash between deciding and recording cannot re-fire it.
        if matches!(task, SyncTask::Advisories) {
            self.maybe_alert(now);
        }

        // Every budget row this step touched, so a surface renders `—` for what was never
        // observed rather than a zero nobody measured.
        if let Ok(budgets) = self.read_budgets() {
            for budget in &budgets {
                emit_budget(self.events.as_ref(), budget);
            }
        }

        let observed = LastSettle {
            outcome: outcome.kind(),
            summary: summary.as_ref().map(repos::ListingSummary::payload),
        };
        let payload = settled_of(&settled_row, Some(&observed));
        let cleared = {
            let mut live = self.live.lock().unwrap_or_else(PoisonError::into_inner);
            live.last.insert((kind, key), observed);
            // **One banner, whatever the number of failed tasks**: a single value, so three
            // failures at once cannot produce three candidates. A success clears it.
            let had = live.notice.is_some();
            live.notice = notice_for(&outcome);
            had && live.notice.is_none()
        };
        if let Some(notice) = notice_for(&outcome) {
            emit_notice(self.events.as_ref(), notice);
        } else if cleared {
            // The wire has no event for *no notice*, so a cleared banner travels as the whole
            // status — see `emit_snapshot`. **On the transition only**: one per settle would
            // re-send every task and every budget for every page of every listing.
            self.emit_status_snapshot();
        }
        emit_settled(self.events.as_ref(), &payload);
        // A settle that parked or re-queued the row leaves work outstanding; one that ended it
        // may have emptied the table, and the next `take_next` is what establishes which.
        self.outstanding.store(true, Ordering::SeqCst);
    }

    /// [p3] §32.12's decision and, if it fires, its one event.
    ///
    /// Its own function because `settle` is at clippy's line ceiling and because this is a
    /// separable step: the alert is decided from stored facts, not from the outcome that just
    /// settled, so nothing above it is in scope here.
    fn maybe_alert(&self, now: i64) {
        let alert = {
            let mut guard = self.index.lock().unwrap_or_else(PoisonError::into_inner);
            guard
                .with_tx(|tx| {
                    // §32.12's fifth conjunct is §30's surface gate. Read before the decision, so
                    // a failed read is an index error rather than a guess inside the predicate.
                    let suppressed = surface_suppressed_projects(tx)?;
                    // §32.12 rule 3: a project not yet computed is seeded here, never told. Its
                    // pairs can already match — a batch folded, or its lockfile named a triple an
                    // earlier sweep answered — at a settle that computes no items.
                    crate::advisories::notify::seed_first_computations(tx, now)
                        .and_then(|_| {
                            crate::advisories::notify::notifiable(tx, now, &|p| {
                                suppressed.contains(&p.0)
                            })
                        })
                        .map_err(|e| match e {
                            crate::advisories::AdvisoryError::Index(index) => index,
                            other => {
                                // A decision this build could not make is not one to guess at:
                                // no event, a line on stderr for the log the shell keeps, and the
                                // ledger untouched so the next settle can decide it again.
                                eprintln!("sync: the advisory alert could not be decided: {other}");
                                IndexError::Corrupt {
                                    detail: other.to_string(),
                                }
                            }
                        })
                })
                .ok()
                .flatten()
        };
        if let Some(alert) = alert {
            crate::sync::events::emit_advisory_alert(self.events.as_ref(), &alert);
        }
    }

    /// [p3] §31.5's hook-site-2 project set, resolved from the task that just settled.
    ///
    /// | Task | Projects re-evaluated |
    /// |---|---|
    /// | `ProjectRemote { project_id }` | that one |
    /// | `AccountRepos` · `RenameProbe` | every project with a `project_account` row for that account |
    /// | `Advisories` (key NULL, library-wide) | every project the sweep's verdict covers |
    ///
    /// **The account-scoped kinds answer an empty set on a `NextPage`**, which is what keeps a
    /// twenty-page listing from re-evaluating every project of that account twenty times. A
    /// throttle, a park and every other non-terminal outcome are the same: nothing has settled,
    /// so nothing has changed.
    fn projects_for_sync_task(&self, task: &SyncTask, outcome: &SyncOutcome) -> Vec<ProjectId> {
        let terminal = !matches!(outcome, SyncOutcome::NextPage { .. });
        let guard = self.index.lock().unwrap_or_else(PoisonError::into_inner);
        let conn = guard.conn();
        match task {
            SyncTask::ProjectRemote { project_id } => vec![*project_id],
            SyncTask::AccountRepos { account_id } | SyncTask::RenameProbe { account_id } => {
                if !terminal {
                    return Vec::new();
                }
                project_ids(
                    conn,
                    "SELECT project_id FROM project_account WHERE account_id = ?1",
                    rusqlite::params![account_id.0],
                )
            }
            // The sweep is library-wide, so *the projects its verdict covers* is the set that has
            // a dependency reading at all. A project with no lockfile scan has no `deps` answer
            // the sweep could have moved.
            SyncTask::Advisories => {
                if !terminal {
                    return Vec::new();
                }
                project_ids(
                    conn,
                    "SELECT project_id FROM project_dependency_scan",
                    rusqlite::params![],
                )
            }
        }
    }

    /// §21.6's *every response*, for the ones this process made on some **other** thread.
    ///
    /// The Device Flow pump shares this decorator, and its poll is unauthenticated — no token has
    /// been issued yet — so what it spends is the **per-IP** pool, which §21.6 keys by the absence
    /// of an account rather than by a sentinel one. Attributing it to whichever account this task
    /// happens to belong to would put another pool's numbers under that account's name.
    ///
    /// It also flushes the channel, which is what stops a long connect flow accumulating
    /// observations nobody claims.
    fn mirror_foreign_observations(&self) {
        let foreign = self.deps.transport.drain_foreign();
        if foreign.is_empty() {
            return;
        }
        let mut guard = self.index.lock().unwrap_or_else(PoisonError::into_inner);
        let _ = guard.with_tx(|tx| {
            for one in &foreign {
                crate::sync::budget::mirror(tx, None, &one.rate, one.at)?;
            }
            Ok(())
        });
    }

    /// Re-send the whole of `sync.status`, so a subscriber sees a banner that is gone.
    fn emit_status_snapshot(&self) {
        let live = self.live();
        let status = {
            let guard = self.index.lock().unwrap_or_else(PoisonError::into_inner);
            crate::sync::events::status_payload(guard.conn(), &live)
        };
        if let Ok(status) = status {
            crate::sync::events::emit_snapshot(self.events.as_ref(), &status);
        }
    }

    fn read_budgets(&self) -> Result<Vec<crate::protocol::SyncBudget>, IndexError> {
        let guard = self.index.lock().unwrap_or_else(PoisonError::into_inner);
        crate::sync::events::budgets(guard.conn())
    }
}

/// [p3] Every project §30's surface gate suppresses, by id.
///
/// The gate is `health::enrolment::surface_suppressed` and nothing here restates it: this reads
/// the two facts it takes, for every project, once per decision.
fn surface_suppressed_projects(
    tx: &rusqlite::Transaction<'_>,
) -> Result<std::collections::HashSet<i64>, IndexError> {
    let mut stmt = tx.prepare("SELECT id, acknowledged_at, is_archived FROM project")?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, Option<i64>>(1)?,
            r.get::<_, i64>(2)? != 0,
        ))
    })?;
    let mut suppressed = std::collections::HashSet::new();
    for row in rows {
        let (id, acknowledged_at, is_archived) = row?;
        if crate::health::enrolment::surface_suppressed(
            crate::health::enrolment::is_enrolled(acknowledged_at),
            is_archived,
        ) {
            suppressed.insert(id);
        }
    }
    Ok(suppressed)
}

/// Queue an `account_repos` row for every account `due_listings` names, and say how many.
///
/// A fresh `queued` row rather than a promotion: a scheduled run starts with both counters at
/// zero, and `due_listings` already refuses to touch a row in any state but `ok` — so this cannot
/// overwrite a park with a clock or revive something `blocked`.
fn sweep_schedule(tx: &rusqlite::Transaction<'_>, now: i64) -> Result<usize, IndexError> {
    let due = crate::sync::schedule::due_listings(tx, now)?;
    for account in &due {
        put(
            tx,
            &SyncTaskStateRow::queued(SyncTaskKind::AccountRepos, Some(account.0), now),
        )?;
    }
    // [p3] §32.2's cadence, queued beside the listings and on the same terms: a fresh `queued`
    // row, and only when nothing is already in flight for it. Re-queueing a parked row would
    // discard the instant the server named.
    let mut queued = due.len();
    if crate::sync::schedule::advisory_due(tx, now)? {
        let existing = load(tx, SyncTaskKind::Advisories, None)?;
        let in_flight = matches!(
            existing.as_ref().map(|row| row.state),
            Some(
                SyncTaskState::Queued
                    | SyncTaskState::Running
                    | SyncTaskState::Parked
                    | SyncTaskState::Blocked
            )
        );
        if !in_flight {
            put(
                tx,
                &SyncTaskStateRow::queued(SyncTaskKind::Advisories, None, now),
            )?;
            queued += 1;
        }
    }
    Ok(queued)
}

/// Whether any row is still the loop's to act on.
///
/// `running` counts: this process put it there and owes it a settle. `ok`, `deferred` and
/// `blocked` do not — each is left by a trigger, a revival cause or an account change, every one
/// of which goes through `enqueue` or `reset_for` and signals.
///
/// **[p3] It counts what the pick can claim, and is derived from the same read.** Counting by
/// `state` alone made a row the pick cannot resolve keep `outstanding` true for ever, so the loop
/// polled a task it could never take — a busy runner with no work, and no error to say so.
/// [`load_all`] already drops the rows [`crate::sync::store::read_row`] skipped, and [`task_of`]
/// drops the shapes it refuses to guess at. **The pick and the wakefulness predicate disagreeing
/// is the defect; one input is the fix.**
///
/// It is `pub` so a test can assert that agreement directly rather than by inferring it from how
/// long a loop stays awake.
///
/// # Errors
/// Fails when SQLite cannot be read.
pub fn any_outstanding(tx: &rusqlite::Transaction<'_>) -> Result<bool, IndexError> {
    Ok(load_all(tx)?.iter().any(|row| {
        matches!(
            row.state,
            SyncTaskState::Queued | SyncTaskState::Running | SyncTaskState::Parked
        ) && task_of(row).is_some()
    }))
}

/// §21.10's four sentences, from the outcome that produced them.
///
/// **One banner, whatever the number of failed tasks**: the notice is a single value on the live
/// state, not a list, so three failures at once cannot produce three candidates.
fn notice_for(outcome: &SyncOutcome) -> Option<SyncNotice> {
    match outcome {
        SyncOutcome::Throttled { .. } => Some(SyncNotice::Throttled),
        SyncOutcome::Unauthorized { reason } => Some(match reason {
            UnauthorizedReason::TokenInvalid => SyncNotice::Unauthorized,
            UnauthorizedReason::SsoRequired | UnauthorizedReason::Forbidden => {
                SyncNotice::Forbidden
            }
        }),
        SyncOutcome::TransientFail { .. } => Some(SyncNotice::Offline),
        SyncOutcome::Done
        | SyncOutcome::NotModified
        | SyncOutcome::NextPage { .. }
        | SyncOutcome::NotFound => None,
        // **A client defect is not a permission state, and §21.13 has no banner for it.**
        // §21.8 step 6 makes `Rejected` *any other 4xx* — a 400, a 422, a 409 — and the
        // classifier's own note says a request this client formed wrongly will be formed wrongly
        // again. `Forbidden`'s copy names three causes, all of them about the account, and sends
        // the user to a screen where none of them is true and nothing can be fixed; `Offline` is
        // no better. So: no banner, a line on stderr for the log the shell keeps, and a `blocked`
        // row that says `rejected_<status>` for whoever reads the status.
        SyncOutcome::Rejected { status } => {
            eprintln!("sync: the forge rejected a request this build formed: {status}");
            None
        }
    }
}

/// The pool to read **before issuing**, for one task.
///
/// [p3] §32.3's second defect. [`DEFAULT_RESOURCE`]'s own doc comment concedes the guess only
/// because *"an unobserved pool answers `Unknown`, which spends, so a wrong guess here costs one
/// request and corrects itself"* — true for an **on-demand** task. For a scheduled sweep,
/// Unknown-spends is **no brake at all** until the source refuses, which is the direction that
/// rate-limits the IP for every other unauthenticated call the app makes.
///
/// So the advisory sweep is keyed by the resource **its own last mirrored response named**, and
/// `DEFAULT_RESOURCE` is the fallback only until one has. Every other task keeps the constant:
/// each of phase 2's six reads is an authenticated REST call and `core` is what they all answer
/// from.
fn resource_for(conn: &rusqlite::Connection, task: &SyncTask) -> String {
    match task {
        SyncTask::Advisories => crate::advisories::store::last_settled_resource(conn)
            .ok()
            .flatten()
            .unwrap_or_else(|| DEFAULT_RESOURCE.to_owned()),
        _ => DEFAULT_RESOURCE.to_owned(),
    }
}

/// A stored row back into the work item it describes.
///
/// **[p3] It matches on the `(kind, key)` pair, not on the kind alone.** The shipped version
/// opened `let key = row.key?;`, which is reached only through the pick's `filter_map` — so a
/// `key IS NULL` row was filtered out **before it could be picked**: never claimed, never run,
/// never settled, and raising no error anywhere. The three keyed kinds require `Some`;
/// `Advisories` requires `None`.
///
/// **A mismatched pair yields `None` rather than a guess**, exactly as
/// [`crate::sync::store::read_row`] refuses a slug it cannot resolve. Guessing which id an
/// `advisories` row's stray key was would run the wrong task against it, and inventing one for a
/// keyed kind would run a task against account or project zero.
fn task_of(row: &SyncTaskStateRow) -> Option<SyncTask> {
    match (row.kind, row.key) {
        (SyncTaskKind::AccountRepos, Some(key)) => Some(SyncTask::AccountRepos {
            account_id: AccountId(key),
        }),
        (SyncTaskKind::ProjectRemote, Some(key)) => Some(SyncTask::ProjectRemote {
            project_id: ProjectId(key),
        }),
        (SyncTaskKind::RenameProbe, Some(key)) => Some(SyncTask::RenameProbe {
            account_id: AccountId(key),
        }),
        (SyncTaskKind::Advisories, None) => Some(SyncTask::Advisories),
        (
            SyncTaskKind::AccountRepos | SyncTaskKind::ProjectRemote | SyncTaskKind::RenameProbe,
            None,
        )
        | (SyncTaskKind::Advisories, Some(_)) => None,
    }
}

impl SyncSink for SyncRunner {
    fn on_project_visible(&self, project: ProjectId) {
        self.enqueue(SyncTask::ProjectRemote {
            project_id: project,
        });
    }
}

/// [p3] A project-id column into a list, with a read failure answering **empty** rather than
/// guessing at a set.
///
/// An empty set skips a recompute, which is a missed refresh; a wrong set recomputes projects
/// whose inputs did not move. The evaluator is change-gated, so the first costs nothing a later
/// settle does not fix.
fn project_ids(
    conn: &rusqlite::Connection,
    sql: &str,
    params: &[&dyn rusqlite::ToSql],
) -> Vec<ProjectId> {
    let Ok(mut st) = conn.prepare(sql) else {
        return Vec::new();
    };
    let Ok(rows) = st.query_map(params, |r| r.get::<_, i64>(0)) else {
        return Vec::new();
    };
    rows.flatten().map(ProjectId).collect()
}

#[cfg(all(test, feature = "testkit"))]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
mod tests {
    use super::*;

    /// [p3] §31.5's hook-site-2 project set.
    ///
    /// **The account-scoped kinds answer an empty set on a `NextPage`.** A twenty-page listing
    /// would otherwise re-evaluate every project of that account twenty times — and correctness
    /// does not depend on which settles fire, because the evaluator is idempotent and
    /// change-gated. **Only cost does**, which is what this asserts.
    #[test]
    fn a_next_page_resolves_no_project_for_the_account_scoped_kinds() {
        let dir = tempfile::tempdir().unwrap();
        let mut index = Index::open_at(dir.path(), 0).unwrap();
        let (account, project) = index
            .with_tx(|tx| {
                tx.execute(
                    "INSERT INTO account (provider, host, login, auth_kind, scope_tier,
                                          granted_scopes, token_ref, connected_at)
                     VALUES ('github', 'h', 'o', 'device', 'private', 'repo', 'r', 1)",
                    [],
                )?;
                let account = AccountId(tx.last_insert_rowid());
                tx.execute(
                    "INSERT INTO project (name, seed_basename, created_at, updated_at)
                     VALUES ('a', 'a', 1, 1)",
                    [],
                )?;
                let project = ProjectId(tx.last_insert_rowid());
                tx.execute(
                    "INSERT INTO project_account
                        (project_id, account_id, affiliation, can_push, observed_at)
                     VALUES (?1, ?2, 'owner', 1, 1)",
                    rusqlite::params![project.0, account.0],
                )?;
                Ok((account, project))
            })
            .unwrap();

        let runner = SyncRunner::new(
            Arc::new(Mutex::new(index)),
            test_deps(),
            Arc::new(NullEvents) as Arc<dyn EventSink>,
        );

        let listing = SyncTask::AccountRepos {
            account_id: account,
        };
        assert_eq!(
            runner.projects_for_sync_task(
                &listing,
                &SyncOutcome::NextPage {
                    cursor: "2".to_owned()
                }
            ),
            Vec::new(),
            "a page is not a settle"
        );
        assert_eq!(
            runner.projects_for_sync_task(&listing, &SyncOutcome::Done),
            vec![project],
            "the terminal outcome resolves the account's projects once"
        );

        // A per-project task has no pagination to wait for, so it resolves whatever the outcome.
        let one = SyncTask::ProjectRemote {
            project_id: project,
        };
        assert_eq!(
            runner.projects_for_sync_task(&one, &SyncOutcome::Done),
            vec![project]
        );
    }

    #[derive(Debug)]
    struct NullEvents;
    impl EventSink for NullEvents {
        fn emit(&self, _topic: &str, _event: &str, _payload: serde_json::Value) {}
    }

    fn test_deps() -> SyncDeps {
        let transport = Arc::new(crate::testing::FakeTransport::new());
        let clock = Arc::new(crate::testing::FakeClock::new(1));
        let observing = Arc::new(crate::sync::ObservingTransport::new(
            Arc::clone(&transport) as Arc<dyn crate::http::HttpTransport>,
            Arc::clone(&clock) as Arc<dyn crate::clock::Clock>,
        ));
        SyncDeps {
            provider: Arc::new(crate::provider::GitHubProvider::new(
                Arc::clone(&observing) as Arc<dyn crate::http::HttpTransport>,
                "h".to_owned(),
            )),
            transport: observing,
            tokens: Arc::new(crate::testing::FakeTokenStore::available()),
            clock: Arc::clone(&clock) as Arc<dyn crate::clock::Clock>,
            cancel: crate::cancel::CancelToken::new(),
            tz_offset_min: 0,
        }
    }
}
