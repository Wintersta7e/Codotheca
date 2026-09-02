//! The composition root: the real `CommandHandler`, the startup sequence and the tick pump.

pub mod handoff;
pub mod jobs;
pub mod route;
pub mod startup;

use crate::art::ArtCtx;
use crate::commands::launch as launch_cmd;
use crate::commands::targets as targets_cmd;
use crate::index::Index;
use crate::proto::dispatch::{CommandFailure, CommandHandler};
use crate::proto::pubsub::EventSink;
use crate::proto::pubsub::PublisherSink;
use crate::protocol::Topic;
use crate::scan::presence::ScanStore;
use crate::scan::{ScanCtx, ScanSupervisor};
use crate::session::manager::SessionManager;
use crate::session::DEFAULT_TICK_SECS;
use route::{command_name, route, Route};
use serde_json::Value;
use std::sync::{Arc, Mutex, PoisonError};

/// Everything the core is composed from. Moved into `CoreHandler` and not used again.
pub struct CoreDeps {
    /// **`Arc<Mutex<Index>>`, not an owned `Index`.** `ScanStore` is `Send + Sync` because the
    /// walk thread writes presence rows through it, and `SqliteScanStore` therefore takes the
    /// index this way (`scan/store.rs:39`), as `JobRunner` already does (R39,
    /// `jobs/scheduler.rs:85`). There is still exactly one `rusqlite::Connection` in the
    /// process; the mutex is what lets the one connection be reached from the loop thread and
    /// from a worker without a second one existing.
    pub index: Arc<Mutex<Index>>,
    pub clock: Arc<dyn crate::clock::Clock>,
    pub git: Arc<dyn crate::git::GitBackend>,
    pub mount: Arc<dyn crate::mount::MountResolver>,
    pub spawner: Box<dyn crate::launch::spawn::Spawner>,
    pub sessions: SessionManager,
    /// `ScanCtx`'s (plan 07). A supervisor, not a `JobRunner`: `ScanLauncher` is `Send + Sync`
    /// and no `rusqlite::Connection` crosses it.
    pub scans: ScanSupervisor,
    /// `ScanCtx.store`. Built over the same `Arc<Mutex<Index>>` as `index`, so it is the same
    /// connection reached a different way — never a second one.
    pub scan_store: Arc<dyn ScanStore>,
    pub firstrun: crate::firstrun::FirstRunEnv,
    /// The job pump (§4.1a). Held here so `shutdown` can stop it **before** the publisher
    /// closes and before the process exits: a worker mid-write to SQLite when `main` returns is
    /// a torn observation, and a worker inside a twenty-second history read is a git process
    /// tree outliving the app.
    pub jobs: jobs::JobPump,
    pub events: Arc<PublisherSink>,
    /// `ProjectsCtx`'s (plan 13). §8.1's era bands cut on the local calendar year and nothing
    /// under `crate::projects` reads a clock or a zone, so the offset arrives with the deps.
    pub tz_offset_min: i32,
}

impl std::fmt::Debug for CoreDeps {
    /// Written by hand: `Spawner`, `Clock`, `GitBackend`, `MountResolver` and `ScanStore` do not
    /// require `Debug`, and none of their contents belongs in a log line.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CoreDeps")
            .field("tz_offset_min", &self.tz_offset_min)
            .finish_non_exhaustive()
    }
}

/// The real `CommandHandler`. Reaches the process's only `rusqlite::Connection` through `index`.
pub struct CoreHandler {
    index: Arc<Mutex<Index>>,
    clock: Arc<dyn crate::clock::Clock>,
    git: Arc<dyn crate::git::GitBackend>,
    mount: Arc<dyn crate::mount::MountResolver>,
    spawner: Box<dyn crate::launch::spawn::Spawner>,
    /// `Option` so `shutdown` can drop the manager, and with it the `Arc<PublisherSink>` clone
    /// it holds. `Transport::join` blocks until every `FrameSink` clone is gone.
    sessions: Option<SessionManager>,
    scans: ScanSupervisor,
    scan_store: Arc<dyn ScanStore>,
    firstrun: crate::firstrun::FirstRunEnv,
    jobs: jobs::JobPump,
    events: Arc<PublisherSink>,
    tz_offset_min: i32,
    /// Measured against `Clock::monotonic_ms`, never wall time: a clock step backwards must not
    /// fire a burst of ticks, and `now_unix` is something a user or NTP can move.
    last_tick_ms: u64,
    ticks: u64,
}

impl std::fmt::Debug for CoreHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CoreHandler")
            .field("tz_offset_min", &self.tz_offset_min)
            .field("shut_down", &self.sessions.is_none())
            .finish_non_exhaustive()
    }
}

impl CoreHandler {
    #[must_use]
    pub fn new(deps: CoreDeps) -> CoreHandler {
        let last_tick_ms = deps.clock.monotonic_ms();
        CoreHandler {
            index: deps.index,
            clock: deps.clock,
            git: deps.git,
            mount: deps.mount,
            spawner: deps.spawner,
            sessions: Some(deps.sessions),
            scans: deps.scans,
            scan_store: deps.scan_store,
            firstrun: deps.firstrun,
            jobs: deps.jobs,
            events: deps.events,
            tz_offset_min: deps.tz_offset_min,
            last_tick_ms,
            ticks: 0,
        }
    }

    /// How many ticks have run. The pump's period is otherwise unobservable from outside.
    #[must_use]
    pub fn ticks(&self) -> u64 {
        self.ticks
    }

    /// The one connection, shared rather than owned. Every caller takes the lock for the length
    /// of one call and releases it; a poisoned lock is recovered rather than propagated, for the
    /// reason `SqliteScanStore` gives — the connection's own state is intact after an unrelated
    /// panic, and refusing every later command would turn one panic into a permanently dead core.
    #[must_use]
    pub fn index(&self) -> &Arc<Mutex<Index>> {
        &self.index
    }

    fn unowned(command: &str, plan: &str) -> CommandFailure {
        CommandFailure::internal(format!(
            "{command}: no handler in the core; plan {plan} owns it"
        ))
    }

    /// One topic's snapshot payload, or `None` when it could not be computed.
    ///
    /// **`None` is not "empty".** Every step is a `?`, so a delegated command that *fails* makes
    /// the whole snapshot `Null` — §7.7a's invariant at the protocol layer. An empty `rows: []`
    /// says *there are no projects*; `null` says *not computed*, and a snapshot whose producing
    /// command could not answer is the second. The distinction cuts both ways: a **successful**
    /// `projects.list` over an empty library returns `rows: []`, and answering `Null` there would
    /// tell the shell the library had never been looked at.
    ///
    /// Delegation goes through `handle`, never into the modules directly, so a subscriber's
    /// snapshot is byte-for-byte what the same command would have returned.
    ///
    /// `epoch` and `throughSeq` are the publisher's and are stamped by `supply_snapshot`, so
    /// they are deliberately absent here.
    fn snapshot_of(&mut self, topic: Topic) -> Option<Value> {
        match topic {
            // Plan 02's `topics` block declares a `snapshot` event for `projects` and `core`
            // only. There is no frame to build for these two, so `Null` is the whole answer —
            // a command's result is not a topic's snapshot type.
            Topic::Scan | Topic::Session => None,
            Topic::Projects => {
                let page = self.handle("projects.list", serde_json::json!({})).ok()?;
                Some(serde_json::json!({
                    "generation": page.get("generation")?,
                    "rows": page.get("rows")?,
                }))
            }
            Topic::Core => {
                let scan = self.handle("scan.status", serde_json::json!({})).ok()?;
                let guard = self.index.lock().unwrap_or_else(PoisonError::into_inner);
                let schema_version = guard.schema_version().ok()?;
                // Both are `?` on the wire and both are read, never synthesised: `gitVersion` is
                // whatever the startup floor check recorded, and `firstRunCompletedAt` is
                // first run's own stamp. An unset key is a real `null`, not a zero.
                let git_version = guard.app_meta("git_version").ok()?;
                let first_run_completed_at =
                    crate::firstrun::first_run_completed_at(guard.conn()).ok()?;
                Some(serde_json::json!({
                    "gitVersion": git_version,
                    "schemaVersion": schema_version,
                    "scan": scan,
                    "firstRunCompletedAt": first_run_completed_at,
                }))
            }
        }
    }

    /// A tick step that failed is a `core/error` event, never a panic and never a silent skip.
    /// The message is diagnostic (§2.4) and is not rendered.
    fn emit_tick_error(&self, step: &str, message: &str) {
        self.events.emit(
            "core",
            "error",
            serde_json::json!({
                "code": "INTERNAL",
                "message": format!("tick: {step}: {message}"),
            }),
        );
    }

    fn declined(command: &str, dest: Route) -> CommandFailure {
        CommandFailure::internal(format!(
            "{command}: routed to {dest:?}, which declined it — the router and the module \
             disagree about ownership"
        ))
    }
}

impl CommandHandler for CoreHandler {
    fn handle(&mut self, command: &str, args: Value) -> Result<Value, CommandFailure> {
        let name = command_name(command)?;
        let dest = route(name);
        let now = self.clock.now_unix();

        match dest {
            Route::Loop => {
                return Err(CommandFailure::protocol(format!(
                    "{command} is answered by the command loop and must not reach the handler"
                )))
            }
            Route::NoOwner(plan) => return Err(Self::unowned(command, plan)),
            // Answered **without** the index lock. `ScanCtx` takes a `&dyn ScanStore`, not an
            // `&Index`, and `SqliteScanStore` locks the same mutex internally; `std::sync::Mutex`
            // is not reentrant, so holding it here would deadlock the core on `scan.status`.
            Route::Scan => {
                let ctx = ScanCtx {
                    store: self.scan_store.as_ref(),
                    events: self.events.as_ref(),
                    scans: &self.scans,
                    now,
                };
                return crate::scan::dispatch_scan_command(&ctx, command, args)
                    .unwrap_or_else(|| Err(Self::declined(command, dest)));
            }
            _ => {}
        }

        // One guard for the length of one command. Every context below is built from it, so the
        // borrow checker still enforces that no two are alive at once.
        //
        // Locked through the field rather than through a `&self` helper on purpose: a method
        // taking `&self` borrows *all* of `self`, and `Route::Launch` needs `self.sessions`
        // mutably at the same time. Field-disjoint borrows are what make that legal.
        let mut guard = self.index.lock().unwrap_or_else(PoisonError::into_inner);

        let claimed = match dest {
            // Handled above; a second arm keeps the match total without a wildcard on Route.
            Route::Loop | Route::NoOwner(_) | Route::Scan => unreachable!(),
            Route::FirstRun => {
                crate::firstrun::dispatch(guard.conn_mut(), &self.firstrun, command, &args, now)
            }
            Route::Art => {
                let ctx = ArtCtx {
                    index: &guard,
                    events: self.events.as_ref(),
                    now,
                };
                crate::art::dispatch_art_command(&ctx, command, args)
            }
            Route::Surfaces => {
                let ctx = crate::surfaces::SurfaceCtx { index: &guard, now };
                crate::surfaces::dispatch_surface_command(&ctx, command, args)
            }
            Route::Targets => {
                let mut ctx = targets_cmd::TargetsCtx {
                    index: &mut guard,
                    events: self.events.as_ref(),
                    now,
                };
                targets_cmd::dispatch_targets_command(&mut ctx, command, args)
            }
            Route::Launch => {
                let Some(sessions) = self.sessions.as_mut() else {
                    return Err(CommandFailure::internal("session manager is shut down"));
                };
                let mut ctx = launch_cmd::LaunchCtx {
                    index: &mut guard,
                    sessions,
                    spawner: self.spawner.as_ref(),
                    events: self.events.as_ref(),
                    mounts: self.mount.as_ref(),
                    now,
                };
                let answer = launch_cmd::dispatch_launch_command(&mut ctx, command, args);
                // §5's condition signal can change as a side effect of a launch or a stop.
                // Draining here, still inside `handle`, keeps it in the same request the user
                // caused — and it only queues; `run_loop` flushes outside every transaction.
                launch_cmd::publish_condition_changes(&mut ctx);
                answer
            }
            Route::Identity => {
                let ctx = crate::identity::commands::IdentityCtx {
                    index: &guard,
                    events: self.events.as_ref(),
                    now,
                };
                crate::identity::commands::dispatch_identity_command(&ctx, command, args)
            }
            Route::Projects => {
                let ctx = crate::projects::ProjectsCtx {
                    index: &guard,
                    events: self.events.as_ref(),
                    jobs: self.jobs.sink_ref(),
                    mounts: self.mount.as_ref(),
                    now,
                    tz_offset_min: self.tz_offset_min,
                };
                crate::projects::dispatch_projects_command(&ctx, command, args)
            }
            Route::Detail => {
                let ctx = crate::detail::DetailCtx {
                    index: &guard,
                    git: self.git.as_ref(),
                    mount: self.mount.as_ref(),
                    events: self.events.as_ref(),
                    jobs: self.jobs.sink_ref(),
                    now,
                };
                crate::detail::dispatch_detail_command(&ctx, command, args)
            }
            Route::View => {
                let ctx = crate::view::ViewCtx { index: &guard, now };
                crate::view::dispatch_view_command(&ctx, command, args)
            }
        };

        claimed.unwrap_or_else(|| Err(Self::declined(command, dest)))
    }

    fn snapshot(&mut self, topic: Topic) -> Value {
        self.snapshot_of(topic).unwrap_or(Value::Null)
    }

    /// Called once per loop iteration. The tick runs **on the loop thread**, not a timer thread:
    /// it needs `&mut Index`, and the one `rusqlite::Connection` is `Send` but not `Sync`, so a
    /// timer thread would need a second connection and there is exactly one (§1.10).
    ///
    /// `run_loop` wakes at least every `PARENT_POLL` (2 s), so a 15 s period lands within 2 s of
    /// its deadline — finer than a segment boundary needs.
    fn pump(&mut self) {
        let mono = self.clock.monotonic_ms();
        let due = self.ticks == 0
            || mono.saturating_sub(self.last_tick_ms) >= DEFAULT_TICK_SECS.saturating_mul(1_000);
        if !due {
            return;
        }
        self.last_tick_ms = mono;
        self.ticks += 1;
        let now = self.clock.now_unix();

        // 1. Plan 11c's pump: drain the activity watcher, advance every live segment, close what
        //    has gone idle. It publishes condition changes itself, so the shelf learns about a
        //    closed segment here rather than at the next request.
        if let Some(sessions) = self.sessions.as_mut() {
            let mut guard = self.index.lock().unwrap_or_else(PoisonError::into_inner);
            let mut ctx = launch_cmd::LaunchCtx {
                index: &mut guard,
                sessions,
                spawner: self.spawner.as_ref(),
                events: self.events.as_ref(),
                mounts: self.mount.as_ref(),
                now,
            };
            if let Err(e) = launch_cmd::tick(&mut ctx) {
                drop(guard);
                self.emit_tick_error("session tick", &e.message);
            }
        }

        // 2. §1's hourly sidecar export. It rides this tick because it is the only other
        //    periodic work the core has, and a second scheduler would want a second connection.
        let guard = self.index.lock().unwrap_or_else(PoisonError::into_inner);
        match guard.sidecar_due(now) {
            Ok(true) => {
                if let Err(e) = guard.export_sidecar(now) {
                    let message = e.to_string();
                    drop(guard);
                    self.emit_tick_error("sidecar export", &message);
                }
            }
            Ok(false) => {}
            Err(e) => {
                let message = e.to_string();
                drop(guard);
                self.emit_tick_error("sidecar due", &message);
            }
        }
    }

    /// Stops the job pump, ends every live session with `app_exit`, then **drops the session
    /// manager**.
    ///
    /// The pump goes first, and without the index lock held: its workers need that lock to
    /// settle whatever they are running, and a worker still writing when `main` returns is a
    /// torn observation stored as a finished one. It goes here rather than after the loop
    /// because this runs before `Publisher::close()`, so a job's last `job_done` still reaches
    /// the shell.
    ///
    /// The session drop is the other load-bearing half: the manager holds an
    /// `Arc<PublisherSink>` clone, and `Transport::join` waits until every `FrameSink` clone is
    /// gone.
    fn shutdown(&mut self) {
        self.jobs.stop();
        let Some(mut sessions) = self.sessions.take() else {
            return;
        };
        {
            let mut guard = self.index.lock().unwrap_or_else(PoisonError::into_inner);
            if let Err(e) = sessions.shutdown(&mut guard) {
                let message = e.to_string();
                drop(guard);
                self.emit_tick_error("session shutdown", &message);
            }
        }
        drop(sessions);
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
    fn the_unowned_refusal_names_the_command_and_the_plan_and_reports_no_effect() {
        // `UNOWNED_COMMANDS` is empty (R37), so no command reaches this constructor through
        // `handle` today and the integration test's loop runs zero times. The refusal itself
        // still has to be right, because it is what the next unhandled command gets — and a
        // diagnostic that names neither is how nineteen of them stayed invisible.
        let e = CoreHandler::unowned("view.set", "15");
        assert_eq!(e.code, crate::protocol::ErrorCode::Internal);
        assert!(e.message.contains("view.set"));
        assert!(e.message.contains("plan 15"));
        assert_eq!(
            e.outcome, None,
            "nothing was dispatched, so nothing took effect"
        );
    }

    #[test]
    fn a_module_declining_its_own_route_is_internal_not_protocol() {
        // A `PROTOCOL` refusal reads to the shell as "no such command", which is exactly how a
        // routing disagreement would hide. It must name both sides instead.
        let e = CoreHandler::declined("art.url", Route::Art);
        assert_eq!(e.code, crate::protocol::ErrorCode::Internal);
        assert!(e.message.contains("art.url"));
        assert!(e.message.contains("Art"));
    }
}
