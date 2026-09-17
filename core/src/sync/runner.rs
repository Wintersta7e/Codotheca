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
    pub fn new(
        index: Arc<Mutex<Index>>,
        deps: SyncDeps,
        events: Arc<dyn EventSink>,
    ) -> Arc<SyncRunner> {
        Arc::new(SyncRunner {
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
            let key = Some(task.key());
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
        emit_started(
            self.events.as_ref(),
            &SyncTaskStarted {
                kind,
                key: Some(key),
            },
        );

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
        };
        let row = read_budget(guard.conn(), account, DEFAULT_RESOURCE)
            .ok()
            .flatten();
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
            let before = load(guard.conn(), kind, Some(key))
                .ok()
                .flatten()
                .unwrap_or_else(|| SyncTaskStateRow::queued(kind, Some(key), now));
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
            live.last.insert((kind, Some(key)), observed);
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

    fn read_budgets(&self) -> Result<Vec<crate::protocol::SyncBudget>, crate::index::IndexError> {
        let guard = self.index.lock().unwrap_or_else(PoisonError::into_inner);
        crate::sync::events::budgets(guard.conn())
    }
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
    Ok(due.len())
}

/// Whether any row is still the loop's to act on.
///
/// `running` counts: this process put it there and owes it a settle. `ok`, `deferred` and
/// `blocked` do not — each is left by a trigger, a revival cause or an account change, every one
/// of which goes through `enqueue` or `reset_for` and signals.
fn any_outstanding(tx: &rusqlite::Transaction<'_>) -> Result<bool, crate::index::IndexError> {
    let n: i64 = tx.query_row(
        "SELECT count(*) FROM sync_task_state WHERE state IN ('queued', 'running', 'parked')",
        [],
        |row| row.get(0),
    )?;
    Ok(n > 0)
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
        SyncOutcome::Rejected { .. } => Some(SyncNotice::Forbidden),
    }
}

/// A stored row back into the work item it describes.
fn task_of(row: &SyncTaskStateRow) -> Option<SyncTask> {
    let key = row.key?;
    Some(match row.kind {
        SyncTaskKind::AccountRepos => SyncTask::AccountRepos {
            account_id: AccountId(key),
        },
        SyncTaskKind::ProjectRemote => SyncTask::ProjectRemote {
            project_id: ProjectId(key),
        },
        SyncTaskKind::RenameProbe => SyncTask::RenameProbe {
            account_id: AccountId(key),
        },
    })
}

impl SyncSink for SyncRunner {
    fn on_project_visible(&self, project: ProjectId) {
        self.enqueue(SyncTask::ProjectRemote {
            project_id: project,
        });
    }
}
