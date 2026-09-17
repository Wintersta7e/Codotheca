//! The composition root: the real `CommandHandler`, the startup sequence and the tick pump.

pub mod handoff;
pub mod jobs;
pub mod route;
pub mod startup;
pub mod sync;

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
    /// The sync runner (§21.1). Held here for the same reason `jobs` is: `shutdown` stops it
    /// **before** the publisher closes, so a thread mid-write to SQLite when `main` returns is
    /// not a torn observation.
    pub sync: sync::SyncPump,
    pub events: Arc<PublisherSink>,
    /// §20.13's one typed forge seam, and §20.6's keychain. Both arrive as production
    /// implementations from the composition root: a seam whose only implementation is a fake
    /// compiles, passes, and fails at assembly, which has cost this project four rulings.
    pub provider: Arc<dyn crate::provider::Provider>,
    pub tokens: Arc<dyn crate::accounts::keychain::TokenStore>,
    /// The one HTTP client (R66), shared rather than rebuilt: each `reqwest::blocking::Client`
    /// owns a runtime thread and a connection pool, and the Device Flow's pump needs the same
    /// one the provider reads through.
    pub http: Arc<dyn crate::http::HttpTransport>,
    /// §20.2's public client id. Injected rather than read from the constant so a test can drive
    /// the Device Flow's real path: with the shipped empty value `request_device_code` refuses
    /// before it issues anything, and a test that took that branch would assert nothing about
    /// what happens when a request *is* made.
    pub client_id: String,
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
    sync: sync::SyncPump,
    events: Arc<PublisherSink>,
    provider: Arc<dyn crate::provider::Provider>,
    tokens: Arc<dyn crate::accounts::keychain::TokenStore>,
    http: Arc<dyn crate::http::HttpTransport>,
    client_id: String,
    /// The live Device Flow, or none. **Core-side state, not renderer state**: closing the
    /// settings drawer does not cancel it, and `accounts.cancelConnect` or the deadline elapsing
    /// are the only two endings (§20.2).
    connect: Option<crate::accounts::pump::ConnectPump>,
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
            sync: deps.sync,
            events: deps.events,
            provider: deps.provider,
            tokens: deps.tokens,
            http: deps.http,
            client_id: deps.client_id,
            connect: None,
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

    /// §7's arm. Extracted for the same reason as the accounts one below, and it keeps the
    /// no-index-lock rule visible: this context takes a `&dyn ScanStore`, never an `&Index`.
    fn scan_arm(
        &self,
        command: &str,
        args: Value,
        now: i64,
    ) -> Option<Result<Value, CommandFailure>> {
        let ctx = ScanCtx {
            store: self.scan_store.as_ref(),
            events: self.events.as_ref(),
            scans: &self.scans,
            now,
        };
        crate::scan::dispatch_scan_command(&ctx, command, args)
    }

    /// §20.2's two network commands, answered with **no index guard held** (R75).
    ///
    /// `accounts.connect` with a flow already pending returns **that** flow's grant and starts no
    /// second one, and its `expiresInSecs` is the time actually left — a countdown that restarts
    /// on drawer reopen is the surface lying about the deadline.
    fn accounts_net_arm(
        &mut self,
        command: &str,
        args: Value,
        now: i64,
    ) -> Result<Value, CommandFailure> {
        match command {
            "accounts.connect" => {
                if let Some(grant) = self.connect.as_ref().and_then(|p| p.grant(now)) {
                    return serde_json::to_value(grant)
                        .map_err(|e| CommandFailure::internal(e.to_string()));
                }
                // A pump whose flow has ended is replaced, not reused: its worker has returned.
                if let Some(previous) = self.connect.take() {
                    previous.stop();
                }
                let pump = crate::accounts::pump::ConnectPump::start(self.connect_deps());
                let grant = pump.grant(now);
                let refusal = no_flow_reason(&pump);
                self.connect = Some(pump);
                match grant {
                    Some(grant) => serde_json::to_value(grant)
                        .map_err(|e| CommandFailure::internal(e.to_string())),
                    // §20.2: a flow that cannot start fails **by its own reason**. An
                    // unregistered application and an unreachable forge are different facts and
                    // send the user to different places.
                    None => Err(CommandFailure::internal(refusal)),
                }
            }
            "accounts.cancelConnect" => {
                if let Some(pump) = self.connect.as_ref() {
                    pump.cancel();
                }
                Ok(serde_json::json!({}))
            }
            "accounts.connectPat" => self.connect_pat_arm(args, now),
            "accounts.setOrgEnabled" => {
                let org = crate::accounts::commands::set_org_enabled_off_lock(
                    &self.index,
                    self.provider.as_ref(),
                    self.tokens.as_ref(),
                    args,
                    now,
                )?;
                serde_json::to_value(org).map_err(|e| CommandFailure::internal(e.to_string()))
            }
            "accounts.upgradeScope" => self.upgrade_scope_arm(args, now),
            "accounts.disconnect" => crate::accounts::commands::handle_disconnect(
                &self.index,
                self.tokens.as_ref(),
                args,
            ),
            other => Err(Self::declined(other, Route::AccountsNet)),
        }
    }

    /// §20.2's PAT path, answered off the index lock: the verification is a forge round trip.
    fn connect_pat_arm(&mut self, args: Value, now: i64) -> Result<Value, CommandFailure> {
        let parsed: crate::protocol::AccountsConnectPatArgs =
            crate::proto::dispatch::parse_args(args)?;
        let token = crate::accounts::keychain::SecretToken::new(parsed.token);
        let account = crate::accounts::commands::connect_pat(
            &self.index,
            &self.http,
            self.tokens.as_ref(),
            &parsed.host,
            &token,
            now,
        )?;
        serde_json::to_value(account).map_err(|e| CommandFailure::internal(e.to_string()))
    }

    /// §20.2's upgrade: **a fresh Device Flow requesting the private tier.** On success the new
    /// token replaces the old in the same keychain entry; **on any failure the existing token is
    /// untouched and the tier does not change**, because nothing is written until a grant
    /// arrives. There is no in-place downgrade — a client cannot narrow a grant it already has.
    fn upgrade_scope_arm(&mut self, args: Value, now: i64) -> Result<Value, CommandFailure> {
        let parsed: crate::protocol::AccountsUpgradeScopeArgs =
            crate::proto::dispatch::parse_args(args)?;
        let identity = {
            let guard = self
                .index
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            crate::accounts::store::account_identity(guard.conn(), parsed.account_id)
                .map_err(|e| CommandFailure::internal(e.to_string()))?
        };

        if let Some(previous) = self.connect.take() {
            previous.stop();
        }
        let mut deps = self.connect_deps();
        deps.scopes = crate::provider::scopes::SCOPES_PRIVATE;
        deps.host.clone_from(&identity.host);
        deps.sink = Arc::new(crate::accounts::pump::IndexConnectSink::upgrading(
            Arc::clone(&self.index),
            Arc::clone(&self.provider),
            parsed.account_id,
            identity.token_ref,
        ));
        let pump = crate::accounts::pump::ConnectPump::start(deps);
        let grant = pump.grant(now);
        let refusal = no_flow_reason(&pump);
        self.connect = Some(pump);
        match grant {
            Some(grant) => {
                serde_json::to_value(grant).map_err(|e| CommandFailure::internal(e.to_string()))
            }
            None => Err(CommandFailure::internal(refusal)),
        }
    }

    /// Everything the pump needs, assembled from the deps the handler already holds.
    fn connect_deps(&self) -> crate::accounts::pump::ConnectPumpDeps {
        let sink = crate::accounts::pump::IndexConnectSink::new(
            Arc::clone(&self.index),
            Arc::clone(&self.provider),
        );
        crate::accounts::pump::ConnectPumpDeps {
            transport: Arc::clone(&self.http),
            clock: Arc::clone(&self.clock),
            events: Arc::clone(&self.events) as Arc<dyn EventSink>,
            tokens: Arc::clone(&self.tokens),
            sink: Arc::new(sink),
            host: self.provider.canonical_host().to_owned(),
            client_id: self.client_id.clone(),
            scopes: crate::provider::scopes::SCOPES_PUBLIC,
        }
    }

    /// §20.8's arm, extracted so `handle` stays under the line cap rather than growing one
    /// module's context inline.
    ///
    /// It takes the guard and **nothing else** — no `&self`, so it cannot reach the provider, the
    /// keychain or the transport, and no clock, because the two commands left here read a row and
    /// nothing more. R75 as a signature rather than a convention.
    fn accounts_arm(
        guard: &Index,
        command: &str,
        args: Value,
    ) -> Option<Result<Value, CommandFailure>> {
        let mut ctx = crate::accounts::AccountsCtx { index: guard };
        crate::accounts::dispatch_accounts_command(&mut ctx, command, args)
    }

    /// §11's arm. Extracted for the same reason every other arm here is: `handle` sits on
    /// clippy's `too_many_lines` ceiling, so a route added anywhere costs one of these.
    fn surfaces_arm(
        guard: &Index,
        command: &str,
        args: Value,
        now: i64,
    ) -> Option<Result<Value, CommandFailure>> {
        let ctx = crate::surfaces::SurfaceCtx { index: guard, now };
        crate::surfaces::dispatch_surface_command(&ctx, command, args)
    }

    /// §21.13's one read. Its own method for the same reason `accounts_arm` is one: `handle` is
    /// at clippy's `too_many_lines` ceiling.
    ///
    /// It takes the guard and **nothing else**, which is R94's first side as a signature rather
    /// than a convention: a handler path is already under the one index mutex and may not take it
    /// again. The runner is what locks the index, on its own thread.
    fn sync_arm(
        guard: &Index,
        live: crate::sync::events::SyncLive,
        command: crate::protocol::CommandName,
        args: Value,
    ) -> Option<Result<Value, CommandFailure>> {
        let ctx = crate::sync::commands::SyncCtx { index: guard, live };
        crate::sync::commands::dispatch_sync_command(&ctx, command, args)
    }

    /// §25.2's opener. Its own method for the same reason `accounts_arm` is one: `handle` is at
    /// clippy's `too_many_lines` ceiling, and a two-line arm there costs the whole function.
    fn remote_arm(
        guard: &Index,
        command: &str,
        args: Value,
        now: i64,
    ) -> Option<Result<Value, CommandFailure>> {
        let ctx = crate::remote::RemoteCtx { index: guard, now };
        crate::remote::dispatch_remote_command(&ctx, command, args)
    }

    /// §25.5's asset read, answered off the index lock.
    ///
    /// It takes the `Arc<Mutex<Index>>` and locks it itself, which is R94's second side: nothing
    /// above it holds the guard, so it must take one — briefly, before the first socket.
    fn readme_net_arm(&self, args: Value, now: i64) -> Result<Value, CommandFailure> {
        crate::readme::handle_readme_assets_off_lock(
            &self.index,
            self.http.as_ref(),
            crate::readme::fetch::system_resolver,
            args,
            now,
        )
    }

    /// §25.5's README reads and its consent write. Its own method for the same reason
    /// `remote_arm` is one: `handle` is at clippy's `too_many_lines` ceiling, and a four-line arm
    /// there costs the whole function.
    fn readme_arm(
        guard: &Index,
        events: &dyn EventSink,
        command: &str,
        args: Value,
        now: i64,
    ) -> Option<Result<Value, CommandFailure>> {
        let ctx = crate::readme::ReadmeCtx {
            index: guard,
            events,
            now,
        };
        crate::readme::dispatch_readme_command(&ctx, command, args)
    }

    /// §2.2's handshake pair, which `run_loop` answers before the handler is consulted.
    fn loop_only(command: &str) -> CommandFailure {
        CommandFailure::protocol(format!(
            "{command} is answered by the command loop and must not reach the handler"
        ))
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
            // only. There is no frame to build for these three, so `Null` is the whole answer —
            // a command's result is not a topic's snapshot type. [p2] §20.8 declares three
            // events on `accounts` and no `snapshot`, so it joins them.
            //
            // **[p2] `install` shares the arm and not the reason, and must not be read as a
            // fourth topic without a snapshot.** §24.9 *does* declare `install.snapshot` (R54):
            // a renderer that opens mid-clone has to build a frame from something. The store
            // that answers it is p2-24 Task 13's `InstallStateStore` and does not exist yet, so
            // this is `None` meaning *not computed*, and deliberately not `{"runs": []}` — an
            // empty list would say *nothing is installing*, which a core that records nothing
            // cannot know. Task 13 gives it an arm of its own.
            Topic::Scan | Topic::Session | Topic::Accounts | Topic::Install => None,
            // [p2] §21.13 declares one, and it is the command's own answer: a subscriber that
            // missed every delta renders exactly what `sync.status` would have told it.
            Topic::Sync => self.handle("sync.status", serde_json::json!({})).ok(),
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

/// Why a device flow did not start, in the pump's own words.
///
/// Read **before** the pump is stored, because storing it moves it. The four `ConnectError`
/// variants are four different user actions — register the application, reconnect the network,
/// read the forge's refusal, report a malformed answer — and one sentence for all four sent
/// every offline user to check a build setting.
fn no_flow_reason(pump: &crate::accounts::pump::ConnectPump) -> String {
    pump.start_error().map_or_else(
        || "no device flow could be started".to_owned(),
        |error| format!("no device flow could be started: {error}"),
    )
}

impl CommandHandler for CoreHandler {
    fn handle(&mut self, command: &str, args: Value) -> Result<Value, CommandFailure> {
        let name = command_name(command)?;
        let dest = route(name);
        let now = self.clock.now_unix();

        match dest {
            Route::Loop => return Err(Self::loop_only(command)),
            Route::NoOwner(plan) => return Err(Self::unowned(command, plan)),
            // R75: answered **without** the index lock. `ConnectPump::start` issues the Device
            // Flow's first request synchronously, so taking the guard here would hold the
            // process's one SQLite mutex across a forge round trip.
            Route::AccountsNet => return self.accounts_net_arm(command, args, now),
            // Answered **without** the index lock for the same reason: up to 24 asset fetches of
            // 5 s each, and the one SQLite mutex may not be held across them.
            Route::ReadmeNet => return self.readme_net_arm(args, now),
            // Answered **without** the index lock. `ScanCtx` takes a `&dyn ScanStore`, not an
            // `&Index`, and `SqliteScanStore` locks the same mutex internally; `std::sync::Mutex`
            // is not reentrant, so holding it here would deadlock the core on `scan.status`.
            Route::Scan => {
                return self
                    .scan_arm(command, args, now)
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
            Route::Loop
            | Route::NoOwner(_)
            | Route::Scan
            | Route::AccountsNet
            | Route::ReadmeNet => unreachable!(),
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
            Route::Surfaces => Self::surfaces_arm(&guard, command, args, now),
            Route::Accounts => Self::accounts_arm(&guard, command, args),
            Route::Sync => Self::sync_arm(&guard, self.sync.live(), name, args),
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
                    sync: self.sync.sink_ref(),
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
                    sync: self.sync.sink_ref(),
                    now,
                };
                crate::detail::dispatch_detail_command(&ctx, command, args)
            }
            Route::View => {
                let ctx = crate::view::ViewCtx { index: &guard, now };
                crate::view::dispatch_view_command(&ctx, command, args)
            }
            Route::Remote => Self::remote_arm(&guard, command, args, now),
            Route::Readme => Self::readme_arm(&guard, self.events.as_ref(), command, args, now),
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
        // Beside `jobs`, in the same place and the same order, and **before** `Publisher::close()`
        // for the same reason: a sync thread mid-write when `main` returns is a torn observation.
        self.sync.stop();
        // Before `Publisher::close()`, so the last `connect_progress` still reaches the shell,
        // and before the process exits, so no poll thread is mid-write.
        if let Some(pump) = self.connect.take() {
            pump.stop();
        }
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
