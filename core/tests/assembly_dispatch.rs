#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! The loop's side of the composition: one publisher, no deadlock, a clean exit.

use codotheca_core::accounts::keychain::TokenStore as _;
use codotheca_core::proto::dispatch::{run_loop, CommandFailure, CommandHandler, LoopExit};
use codotheca_core::proto::pubsub::{EventSink, Publisher, PublisherSink, TOPIC_HIGH_WATER};
use codotheca_core::proto::transport::{FrameSink, Transport, WRITER_CAPACITY};
use codotheca_core::proto::wire::Epoch;
use codotheca_core::protocol::Topic;
use std::sync::Arc;

/// Emits through the sink it holds, from inside `handle` — the shape that deadlocks if the
/// loop takes the sink's lock across the call.
#[derive(Debug)]
struct EmittingHandler {
    events: Option<Arc<PublisherSink>>,
    held_sink: Option<FrameSink>,
    pumped: u32,
    shut_down: bool,
}

impl CommandHandler for EmittingHandler {
    fn handle(
        &mut self,
        command: &str,
        _args: serde_json::Value,
    ) -> Result<serde_json::Value, CommandFailure> {
        self.events
            .as_ref()
            .expect("events are live until shutdown")
            .emit(
                "projects",
                "condition_changed",
                serde_json::json!({ "id": 1 }),
            );
        Ok(serde_json::json!({ "echo": command }))
    }

    fn snapshot(&mut self, _topic: Topic) -> serde_json::Value {
        serde_json::json!({})
    }

    fn pump(&mut self) {
        self.pumped += 1;
    }

    fn shutdown(&mut self) {
        self.shut_down = true;
        self.events = None;
        self.held_sink = None;
    }
}

fn fixture(
    input: std::io::Cursor<Vec<u8>>,
) -> (
    Transport,
    Arc<PublisherSink>,
    EmittingHandler,
    wire::CapturingWriter,
) {
    let out = wire::CapturingWriter::new();
    let transport = Transport::start(input, out.clone(), WRITER_CAPACITY).expect("transport");
    let events = Arc::new(PublisherSink::new(Publisher::new(
        transport.sink.clone(),
        Epoch(1),
        TOPIC_HIGH_WATER,
    )));
    let handler = EmittingHandler {
        events: Some(Arc::clone(&events)),
        held_sink: Some(transport.sink.clone()),
        pumped: 0,
        shut_down: false,
    };
    (transport, events, handler, out)
}

#[test]
fn a_command_that_emits_through_the_sink_does_not_deadlock_the_loop() {
    let input = wire::inbound(&[
        serde_json::json!({ "t": "subscribe", "topic": "projects" }),
        serde_json::json!({ "t": "request", "id": 1, "command": "targets.list", "args": {} }),
        serde_json::json!({ "t": "request", "id": 2, "command": "app.shutdown", "args": {} }),
    ]);
    let (transport, events, mut handler, out) = fixture(input);

    let exit = run_loop(
        transport,
        &events,
        &mut handler,
        Epoch(1),
        &wire::live_parent(),
    );

    assert_eq!(exit, LoopExit::Shutdown);
    assert!(handler.shut_down, "run_loop calls shutdown before it joins");
    let frames = out.frames();
    assert!(frames
        .iter()
        .any(|frame| frame["t"] == "response" && frame["id"] == 1));
    assert!(
        frames
            .iter()
            .any(|frame| frame["t"] == "event" && frame["event"] == "condition_changed"),
        "the event emitted inside handle reached the wire"
    );
}

#[test]
fn every_loop_iteration_pumps_the_handler_once() {
    let input = wire::inbound(&[
        serde_json::json!({ "t": "request", "id": 1, "command": "targets.list", "args": {} }),
        serde_json::json!({ "t": "request", "id": 2, "command": "targets.list", "args": {} }),
        serde_json::json!({ "t": "request", "id": 3, "command": "app.shutdown", "args": {} }),
    ]);
    let (transport, events, mut handler, _out) = fixture(input);

    let exit = run_loop(
        transport,
        &events,
        &mut handler,
        Epoch(1),
        &wire::live_parent(),
    );

    assert_eq!(exit, LoopExit::Shutdown);
    assert_eq!(handler.pumped, 3);
}

mod wire {
    use codotheca_core::lifecycle::OsParentProbe;
    use codotheca_core::proto::frame::{read_frame, write_frame, FrameError};
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Debug, Default)]
    pub(super) struct CapturingWriter {
        bytes: Arc<Mutex<Vec<u8>>>,
    }

    impl CapturingWriter {
        pub(super) fn new() -> Self {
            Self::default()
        }

        pub(super) fn frames(&self) -> Vec<serde_json::Value> {
            let bytes = self.bytes.lock().expect("capture lock").clone();
            let mut input = std::io::Cursor::new(bytes);
            let mut body = Vec::new();
            let mut frames = Vec::new();
            loop {
                match read_frame(&mut input, &mut body) {
                    Ok(()) => frames.push(serde_json::from_slice(&body).expect("outbound frame")),
                    Err(FrameError::Eof) => break,
                    Err(error) => panic!("captured malformed frame: {error}"),
                }
            }
            frames
        }
    }

    impl std::io::Write for CapturingWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.bytes
                .lock()
                .expect("capture lock")
                .extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    pub(super) fn inbound(frames: &[serde_json::Value]) -> std::io::Cursor<Vec<u8>> {
        let mut bytes = Vec::new();
        for frame in frames {
            let body = serde_json::to_vec(frame).expect("inbound frame");
            write_frame(&mut bytes, &body).expect("frame input");
        }
        std::io::Cursor::new(bytes)
    }

    pub(super) fn live_parent() -> OsParentProbe {
        OsParentProbe::new(std::process::id())
    }
}

// ---------------------------------------------------------------------------
// Task 3: the real handler. Every assertion below is that a command *reached*
// its module, never what that module does with it — those are the modules' own
// suites. The point is the seam: `route` and the ten sub-dispatchers agreeing.
// ---------------------------------------------------------------------------

mod corehandler {
    use super::*;
    use codotheca_core::assembly::{CoreDeps, CoreHandler};
    use codotheca_core::protocol::ErrorCode;

    const NOW: i64 = 1_750_000_000;

    /// A `CoreHandler` over the fakes plan 06 ships. Nothing here touches the real filesystem,
    /// a real git binary, or a real process.
    fn handler(dir: &std::path::Path) -> CoreHandler {
        handler_parts(dir).0
    }

    /// The handler plus the two things a test needs to observe it: the sink whose `Arc` count
    /// proves `shutdown` released the session manager's clone, and the clock the pump's period
    /// is measured against.
    fn handler_parts(
        dir: &std::path::Path,
    ) -> (
        CoreHandler,
        Arc<PublisherSink>,
        Arc<codotheca_core::testing::FakeClock>,
    ) {
        let index = codotheca_core::index::Index::open_at(dir, NOW).expect("index opens");
        let index = Arc::new(std::sync::Mutex::new(index));
        let events = Arc::new(PublisherSink::new(Publisher::detached()));
        let clock = Arc::new(codotheca_core::testing::FakeClock::new(NOW));

        let http: Arc<dyn codotheca_core::http::HttpTransport> =
            Arc::new(codotheca_core::testing::FakeTransport::new());
        let handler = handler_over(
            dir,
            &index,
            &events,
            &clock,
            http,
            Arc::new(codotheca_core::testing::FakeTokenStore::unavailable()),
        );
        (handler, events, clock)
    }

    /// The same handler, over a caller-supplied transport, so a test can watch what the
    /// account commands do to the one index lock while a request is in flight.
    fn handler_over(
        dir: &std::path::Path,
        index: &Arc<std::sync::Mutex<codotheca_core::index::Index>>,
        events: &Arc<PublisherSink>,
        clock: &Arc<codotheca_core::testing::FakeClock>,
        http: Arc<dyn codotheca_core::http::HttpTransport>,
        tokens: Arc<dyn codotheca_core::accounts::keychain::TokenStore>,
    ) -> CoreHandler {
        let clock = Arc::clone(clock);
        let index = Arc::clone(index);
        let events = Arc::clone(events);
        let sync_observing = Arc::new(codotheca_core::sync::http::ObservingTransport::new(
            Arc::clone(&http),
            Arc::clone(&clock) as Arc<dyn codotheca_core::clock::Clock>,
        ));
        let sync_http: Arc<dyn codotheca_core::http::HttpTransport> =
            Arc::clone(&sync_observing) as Arc<dyn codotheca_core::http::HttpTransport>;
        CoreHandler::new(CoreDeps {
            index: Arc::clone(&index),
            // The seam and a fake of it. `FakeTransport` answers nothing here: every accounts
            // command in this file is a routing assertion, not a network one.
            provider: Arc::new(codotheca_core::provider::GitHubProvider::new(
                Arc::clone(&http),
                codotheca_core::provider::listing::GITHUB_CANONICAL_HOST.to_owned(),
            )),
            tokens,
            http,
            // Non-empty, so a test drives the flow's real path rather than the
            // no-application-registered refusal that returns before any request is made.
            client_id: "test-client-id".to_owned(),
            clock: clock.clone(),
            git: Arc::new(codotheca_core::testing::FakeGitBackend::new()),
            mount: Arc::new(codotheca_core::testing::FakeMountResolver::default()),
            spawner: Box::new(codotheca_core::launch::spawn::RecordingSpawner::new()),
            sessions: codotheca_core::session::manager::SessionManager::new(
                Arc::clone(&clock) as Arc<dyn codotheca_core::clock::Clock>,
                Arc::clone(&events) as Arc<dyn EventSink>,
                Box::new(codotheca_core::session::watch::FakeActivitySource::new()),
                Arc::new(codotheca_core::session::activity::FakeIgnoreCheck::new(&[])),
            ),
            scans: codotheca_core::scan::ScanSupervisor::new(Arc::new(
                codotheca_core::testing::ScanLauncherFake::new(),
            )),
            scan_store: Arc::new(codotheca_core::testing::MemScanStore::new()),
            firstrun: firstrun_env(dir),
            // A real pump over the fake git: `shutdown` stops it, and a handler built with one
            // that never started would not exercise that.
            jobs: codotheca_core::assembly::jobs::JobPump::start(
                Arc::clone(&index),
                Arc::new(codotheca_core::testing::FakeGitBackend::new()),
                Arc::clone(&clock) as Arc<dyn codotheca_core::clock::Clock>,
                Arc::clone(&events) as Arc<dyn EventSink>,
            ),
            // [p2] §21.1's runner, real and started, for the same reason the job pump above is:
            // `shutdown` stops it, and a handler built with one that never started would not
            // exercise that.
            sync: codotheca_core::assembly::sync::SyncPump::start(
                Arc::clone(&index),
                codotheca_core::sync::SyncDeps {
                    provider: Arc::new(codotheca_core::provider::GitHubProvider::new(
                        Arc::clone(&sync_http),
                        codotheca_core::provider::listing::GITHUB_CANONICAL_HOST.to_owned(),
                    )),
                    transport: Arc::clone(&sync_observing),
                    tokens: Arc::new(codotheca_core::testing::FakeTokenStore::unavailable()),
                    clock: Arc::clone(&clock) as Arc<dyn codotheca_core::clock::Clock>,
                    cancel: codotheca_core::cancel::CancelToken::new(),
                },
                Arc::clone(&events) as Arc<dyn EventSink>,
            ),
            events: Arc::clone(&events),
            tz_offset_min: 0,
        })
    }

    /// An unreachable forge and an unregistered application are **different facts**.
    ///
    /// The pump discarded its `ConnectError` and the arm reported one sentence for all four
    /// causes, so an offline user was told this build has no OAuth client id compiled in — a
    /// claim about the build, which sends them to fix something that is not broken.
    #[test]
    fn a_forge_it_cannot_reach_is_not_reported_as_a_build_with_no_client_id() {
        let dir = tempfile::tempdir().expect("tmp");
        let index = Arc::new(std::sync::Mutex::new(
            codotheca_core::index::Index::open_at(dir.path(), NOW).expect("index opens"),
        ));
        let events = Arc::new(PublisherSink::new(Publisher::detached()));
        let clock = Arc::new(codotheca_core::testing::FakeClock::new(NOW));
        let mut h = handler_over(
            dir.path(),
            &index,
            &events,
            &clock,
            // A machine with no route to the forge. `client_id` is non-empty in `handler_over`,
            // so the only thing wrong here is the network.
            Arc::new(codotheca_core::http::RefusingTransport),
            Arc::new(codotheca_core::testing::FakeTokenStore::available()),
        );

        let failure = h
            .handle("accounts.connect", serde_json::json!({}))
            .expect_err("an unreachable forge cannot start a flow");
        let message = failure.message.to_ascii_lowercase();
        assert!(
            !message.contains("client id"),
            "an unreachable forge was reported as a build defect: {}",
            failure.message
        );
        assert!(
            message.contains("could not be reached"),
            "the refusal does not name the network: {}",
            failure.message
        );
        h.shutdown();
    }

    /// A `TokenStore` that tries the index lock **from inside `delete`**, on the answering thread.
    ///
    /// The same discriminator as `LockProbingTransport` and for the same reason: `std::sync::
    /// Mutex` is not reentrant, so a guard held by the arm makes this `try_lock` fail with no
    /// threads and no timing involved.
    #[derive(Debug)]
    struct LockProbingTokenStore {
        index: Arc<std::sync::Mutex<codotheca_core::index::Index>>,
        lock_was_free: std::sync::atomic::AtomicBool,
        calls: std::sync::atomic::AtomicUsize,
    }

    impl codotheca_core::accounts::keychain::TokenStore for LockProbingTokenStore {
        fn probe(&self) -> Result<(), codotheca_core::accounts::keychain::KeychainError> {
            Ok(())
        }

        fn store(
            &self,
            _entry: &str,
            _token: &codotheca_core::accounts::keychain::SecretToken,
        ) -> Result<(), codotheca_core::accounts::keychain::KeychainError> {
            Ok(())
        }

        fn read(
            &self,
            _entry: &str,
        ) -> Result<
            codotheca_core::accounts::keychain::SecretToken,
            codotheca_core::accounts::keychain::KeychainError,
        > {
            Err(codotheca_core::accounts::keychain::KeychainError::NotFound)
        }

        fn delete(
            &self,
            _entry: &str,
        ) -> Result<(), codotheca_core::accounts::keychain::KeychainError> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let free = self.index.try_lock().is_ok();
            self.lock_was_free
                .store(free, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }
    }

    /// R75 again, for the call that is **not** a forge round trip.
    ///
    /// `accounts.disconnect` deletes the keychain entry before the row, and `keyring` puts no
    /// timeout on that: a locked Windows credential store can prompt, and a secret-service call
    /// can wait on D-Bus. Holding the process's one SQLite mutex across it stops every other
    /// command for however long the user takes to answer a dialog.
    #[test]
    fn disconnect_holds_no_index_lock_while_it_deletes_the_keychain_entry() {
        let dir = tempfile::tempdir().expect("tmp");
        let index = Arc::new(std::sync::Mutex::new(
            codotheca_core::index::Index::open_at(dir.path(), NOW).expect("index opens"),
        ));
        let account = {
            let mut guard = index.lock().unwrap();
            let tx = guard.conn_mut().transaction().expect("a transaction");
            let id = codotheca_core::accounts::store::insert_account(
                &tx,
                &codotheca_core::accounts::store::NewAccount {
                    provider: "github".to_owned(),
                    host: "forge.example.invalid".to_owned(),
                    login: "octo".to_owned(),
                    display_name: None,
                    auth_kind: codotheca_core::protocol::AuthKind::Device,
                    scope_tier: codotheca_core::protocol::ScopeTier::Public,
                    granted_scopes: Vec::new(),
                    token_ref: "github:forge.example.invalid:octo".to_owned(),
                },
                NOW,
            )
            .expect("the account inserts");
            tx.commit().expect("the insert commits");
            id
        };

        let events = Arc::new(PublisherSink::new(Publisher::detached()));
        let clock = Arc::new(codotheca_core::testing::FakeClock::new(NOW));
        let probe = Arc::new(LockProbingTokenStore {
            index: Arc::clone(&index),
            lock_was_free: std::sync::atomic::AtomicBool::new(false),
            calls: std::sync::atomic::AtomicUsize::new(0),
        });
        let mut h = handler_over(
            dir.path(),
            &index,
            &events,
            &clock,
            Arc::new(codotheca_core::testing::FakeTransport::new()),
            Arc::clone(&probe) as Arc<dyn codotheca_core::accounts::keychain::TokenStore>,
        );

        h.handle(
            "accounts.disconnect",
            serde_json::json!({ "accountId": account.0 }),
        )
        .expect("the disconnect succeeds");

        // No escape hatch: the keychain MUST have been reached, or this proves nothing.
        assert!(
            probe.calls.load(std::sync::atomic::Ordering::SeqCst) > 0,
            "the command never reached the keychain, so the lock was never at risk"
        );
        assert!(
            probe
                .lock_was_free
                .load(std::sync::atomic::Ordering::SeqCst),
            "accounts.disconnect deleted the keychain entry with the index lock held"
        );
        h.shutdown();
    }

    /// A transport that tries the index lock **from inside `send`**, on the very thread that is
    /// answering the command.
    ///
    /// `std::sync::Mutex` is not reentrant, so this discriminates with no threads and no timing:
    /// if the arm answering `accounts.connect` held the guard, this `try_lock` returns `Err` on
    /// the same thread; if it takes no guard, it succeeds. Forcing the condition rather than
    /// waiting for one is R72's rule.
    #[derive(Debug)]
    struct LockProbingTransport {
        index: Arc<std::sync::Mutex<codotheca_core::index::Index>>,
        lock_was_free: std::sync::atomic::AtomicBool,
        calls: std::sync::atomic::AtomicUsize,
    }

    impl codotheca_core::http::HttpTransport for LockProbingTransport {
        fn send(
            &self,
            _req: &codotheca_core::http::HttpRequest,
        ) -> Result<codotheca_core::http::HttpResponse, codotheca_core::http::TransportError>
        {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let free = self.index.try_lock().is_ok();
            self.lock_was_free
                .store(free, std::sync::atomic::Ordering::SeqCst);
            Err(codotheca_core::http::TransportError::Timeout)
        }
    }

    /// R75: the account commands that reach the network are answered **without** the index lock.
    ///
    /// Holding the process's one SQLite mutex across a forge round trip would stop every other
    /// command for as long as `ACCOUNT_LIMITS.total_secs` — thirty seconds — and nothing else in
    /// the core would say so.
    #[test]
    fn a_network_account_command_holds_no_index_lock_while_it_is_in_flight() {
        let dir = tempfile::tempdir().expect("tmp");
        let index = Arc::new(std::sync::Mutex::new(
            codotheca_core::index::Index::open_at(dir.path(), NOW).expect("index opens"),
        ));
        let events = Arc::new(PublisherSink::new(Publisher::detached()));
        let clock = Arc::new(codotheca_core::testing::FakeClock::new(NOW));
        let probe = Arc::new(LockProbingTransport {
            index: Arc::clone(&index),
            lock_was_free: std::sync::atomic::AtomicBool::new(false),
            calls: std::sync::atomic::AtomicUsize::new(0),
        });
        let mut h = handler_over(
            dir.path(),
            &index,
            &events,
            &clock,
            Arc::clone(&probe) as Arc<dyn codotheca_core::http::HttpTransport>,
            Arc::new(codotheca_core::testing::FakeTokenStore::unavailable()),
        );

        // The command's own outcome is beside the point: the client id is empty in this build,
        // so it may refuse before it ever reaches the transport. What must never happen is that
        // it reaches the transport *while holding the guard*.
        let _ = h.handle("accounts.connect", serde_json::json!({}));

        // No escape hatch: the request MUST have been issued, or this test proves nothing
        // about what the arm holds while it is in flight.
        assert!(
            probe.calls.load(std::sync::atomic::Ordering::SeqCst) > 0,
            "the command never reached the transport, so the lock was never at risk"
        );
        assert!(
            probe
                .lock_was_free
                .load(std::sync::atomic::Ordering::SeqCst),
            "accounts.connect reached the network with the index lock held"
        );
    }

    /// R75 again, for the command that already had this defect when the ruling landed:
    /// `accounts.setOrgEnabled` preflights the forge before it writes, and the guarded form held
    /// the process's one SQLite mutex for as long as `ACCOUNT_LIMITS.total_secs` — thirty
    /// seconds in which no other command could be answered.
    #[test]
    fn setting_an_org_gate_holds_no_index_lock_while_it_preflights() {
        let dir = tempfile::tempdir().expect("tmp");
        let index = Arc::new(std::sync::Mutex::new(
            codotheca_core::index::Index::open_at(dir.path(), NOW).expect("index opens"),
        ));
        // One account, so the preflight gets as far as the network.
        {
            let guard = index.lock().expect("lock");
            let tx = guard.conn().unchecked_transaction().expect("tx");
            codotheca_core::accounts::store::insert_account(
                &tx,
                &codotheca_core::accounts::store::NewAccount {
                    provider: "github".to_owned(),
                    host: "forge.example.invalid".to_owned(),
                    login: "octo".to_owned(),
                    display_name: None,
                    auth_kind: codotheca_core::protocol::AuthKind::Device,
                    scope_tier: codotheca_core::protocol::ScopeTier::Private,
                    granted_scopes: vec![],
                    token_ref: "github:forge.example.invalid:octo".to_owned(),
                },
                NOW,
            )
            .expect("account inserts");
            tx.commit().expect("commit");
        }

        let events = Arc::new(PublisherSink::new(Publisher::detached()));
        let clock = Arc::new(codotheca_core::testing::FakeClock::new(NOW));
        let probe = Arc::new(LockProbingTransport {
            index: Arc::clone(&index),
            lock_was_free: std::sync::atomic::AtomicBool::new(false),
            calls: std::sync::atomic::AtomicUsize::new(0),
        });
        let mut h = handler_over(
            dir.path(),
            &index,
            &events,
            &clock,
            Arc::clone(&probe) as Arc<dyn codotheca_core::http::HttpTransport>,
            // A keychain that answers, so the preflight reaches the transport rather than
            // stopping at the token read — the test's own guard caught that first.
            {
                let tokens = codotheca_core::testing::FakeTokenStore::available();
                tokens
                    .store(
                        "github:forge.example.invalid:octo",
                        &codotheca_core::accounts::keychain::SecretToken::new(
                            "sentinel".to_owned(),
                        ),
                    )
                    .expect("stored");
                Arc::new(tokens)
            },
        );
        let _ = h.handle(
            "accounts.setOrgEnabled",
            serde_json::json!({ "accountId": 1, "orgLogin": "an-org", "enabled": true }),
        );

        assert!(
            probe.calls.load(std::sync::atomic::Ordering::SeqCst) > 0,
            "the preflight never reached the transport, so the lock was never at risk"
        );
        assert!(
            probe
                .lock_was_free
                .load(std::sync::atomic::Ordering::SeqCst),
            "accounts.setOrgEnabled preflighted the forge with the index lock held"
        );
    }

    /// The other half, stated positively: a command that only reads a row **does** take the
    /// guard, so the split above is a split and not a blanket exemption.
    #[test]
    fn a_reading_account_command_is_answered_under_the_index_guard() {
        assert_eq!(
            codotheca_core::assembly::route::route(
                codotheca_core::assembly::route::command_name("accounts.list").expect("routable")
            ),
            codotheca_core::assembly::route::Route::Accounts
        );
        assert_eq!(
            codotheca_core::assembly::route::route(
                codotheca_core::assembly::route::command_name("accounts.cancelConnect")
                    .expect("routable")
            ),
            codotheca_core::assembly::route::Route::AccountsNet
        );
        assert_eq!(
            codotheca_core::assembly::route::route(
                codotheca_core::assembly::route::command_name("accounts.setOrgEnabled")
                    .expect("routable")
            ),
            codotheca_core::assembly::route::Route::AccountsNet,
            "setOrgEnabled preflights the forge, so it takes no guard"
        );
    }

    fn firstrun_env(home: &std::path::Path) -> codotheca_core::firstrun::FirstRunEnv {
        codotheca_core::firstrun::FirstRunEnv {
            sources: codotheca_core::firstrun::sources::SourceEnv {
                home: home.to_path_buf(),
                app_data: None,
                xdg_config: None,
            },
            classifier: Arc::new(codotheca_core::firstrun::classify::FixedClassifier::new(
                vec![],
            )),
            distros: Arc::new(codotheca_core::firstrun::classify::NoDistros),
            platform: codotheca_core::index::path::PathPlatform::Unix,
            skip: codotheca_core::scan::skiplist::SkipList::default(),
            cache: codotheca_core::firstrun::roots::SuggestionCache::new(),
        }
    }

    /// The whole point of the plan: **every** schema command the router claims is answerable
    /// reaches a module. Against `RefusingHandler` every one of these was a PROTOCOL refusal.
    #[test]
    fn every_routed_command_reaches_its_module() {
        let dir = tempfile::tempdir().expect("tmp");
        let mut h = handler(dir.path());

        let text = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../protocol/schema/protocol.json"),
        )
        .expect("schema readable");
        let doc: serde_json::Value = serde_json::from_str(&text).expect("schema parses");
        let names: Vec<String> = doc["commands"]
            .as_array()
            .expect("commands array")
            .iter()
            .map(|c| c["name"].as_str().expect("name").to_owned())
            .collect();

        let loop_only = ["app.hello_ack", "app.shutdown"];
        // Read from the router's own census, never a second list here: a command declared with
        // no module must be skipped by exactly the constant that names it, and the assertion
        // below proves the skip is earned rather than a blanket exemption.
        let unowned: Vec<&str> = codotheca_core::assembly::route::UNOWNED_COMMANDS
            .iter()
            .map(|(c, _)| *c)
            .collect();
        let mut checked = 0_u32;
        let mut refused = 0_u32;
        for name in &names {
            if loop_only.contains(&name.as_str()) {
                continue;
            }
            if unowned.contains(&name.as_str()) {
                let e = h
                    .handle(name, serde_json::json!({}))
                    .expect_err("an unowned command must be refused, not answered");
                assert!(
                    e.message.contains("no handler in the core"),
                    "{name} is listed unowned but the router answered it: {}",
                    e.message
                );
                refused += 1;
                continue;
            }
            checked += 1;
            // Bad arguments are fine and expected — they prove the module parsed them. What must
            // never come back is the router's own "no handler" or a module declining its route.
            if let Err(e) = h.handle(name, serde_json::json!({})) {
                assert!(
                    !e.message.contains("no handler in the core"),
                    "{name} reached no module: {}",
                    e.message
                );
                assert!(
                    !e.message.contains("disagree about ownership"),
                    "{name} was routed to a module that declined it: {}",
                    e.message
                );
            }
        }
        // [p2] 40, plus the five §20.8 commands the core now answers — three under the index
        // guard and two, the network pair, without it (R75). The remaining three stay unowned
        // and are refused by name above, which is what the `refused` count opposite asserts.
        // [p2] §25.8's `remote.webUrl` is the 49th, answered under the index guard; its
        // `projects.readme` is the 50th and `projects.setReadmeRemote` the 51st, answered the
        // same way — a file read under a location root and a consent column reach no network.
        // `projects.readmeAssets` is the 52nd and is answered **without** the guard (R75),
        // because it reaches arbitrary hosts.
        // [p2] §21.13's `sync.status` is the 53rd, answered **under** the guard: it reads two
        // tables and the runner's own process state, and the runner is what reaches the network.
        assert_eq!(
            checked, 53,
            "the schema's answerable set, minus the loop's pair and the unowned set"
        );
        assert_eq!(
            refused,
            u32::try_from(unowned.len()).expect("the census is small"),
            "every unowned command was reached and refused by name"
        );
        assert_eq!(
            usize::try_from(checked + refused).expect("small") + loop_only.len(),
            names.len(),
            "every schema command is answerable, refused by name, or the loop's"
        );
    }

    /// A gate whose passing run scans zero files is a failing gate; the same is true of a loop.
    #[test]
    fn the_handshake_pair_never_reaches_the_handler() {
        let dir = tempfile::tempdir().expect("tmp");
        let mut h = handler(dir.path());
        for name in ["app.hello_ack", "app.shutdown"] {
            let e = h
                .handle(name, serde_json::json!({}))
                .expect_err("must refuse");
            assert_eq!(e.code, ErrorCode::Protocol);
            assert!(e.message.contains("command loop"), "{}", e.message);
        }
    }

    #[test]
    fn an_unknown_command_is_a_protocol_error_and_took_no_effect() {
        let dir = tempfile::tempdir().expect("tmp");
        let mut h = handler(dir.path());
        let e = h
            .handle("nope.notacommand", serde_json::json!({}))
            .expect_err("must refuse");
        assert_eq!(e.code, ErrorCode::Protocol);
        assert_eq!(
            e.outcome, None,
            "a name that never dispatched took no effect"
        );
    }

    #[test]
    fn scan_and_session_have_no_snapshot_event_and_answer_null() {
        // Plan 02's `topics` block declares a `snapshot` event for `projects` and `core` only,
        // so there is no frame to build for these two. `scan.status` existing does not change
        // it — a command's result is not a topic's snapshot type.
        let dir = tempfile::tempdir().expect("tmp");
        let mut h = handler(dir.path());
        assert_eq!(h.snapshot(Topic::Scan), serde_json::Value::Null);
        assert_eq!(h.snapshot(Topic::Session), serde_json::Value::Null);
    }

    /// The invariant that cuts both ways. An empty library is *measured, none* and must be
    /// `rows: []` — answering `Null` here would tell the shell it had never been looked at.
    #[test]
    fn an_empty_library_snapshots_as_measured_none_not_as_uncomputed() {
        let dir = tempfile::tempdir().expect("tmp");
        let mut h = handler(dir.path());
        let snap = h.snapshot(Topic::Projects);
        assert!(
            snap.is_object(),
            "an answerable topic must not be Null: {snap}"
        );
        assert_eq!(
            snap["rows"],
            serde_json::json!([]),
            "no projects is an empty row set, never null"
        );
        assert!(
            snap.get("generation").is_some(),
            "generation is part of the payload"
        );
        assert!(
            snap.get("epoch").is_none() && snap.get("throughSeq").is_none(),
            "epoch and throughSeq are the publisher's and are stamped by supply_snapshot"
        );
    }

    /// [p2] §21.13: the `sync` snapshot **is** `sync.status`' answer, byte for byte. Two
    /// producers for one payload would let a subscriber and a caller disagree about the same
    /// moment, which is the whole reason `snapshot_of` delegates through `handle` rather than
    /// reaching into a module.
    #[test]
    fn the_sync_snapshot_is_exactly_what_sync_status_returns() {
        let dir = tempfile::tempdir().expect("tmp");
        let mut h = handler(dir.path());
        let answered = h
            .handle("sync.status", serde_json::json!({}))
            .expect("sync.status answers");
        let snap = h.snapshot(Topic::Sync);
        assert_eq!(snap, answered, "the snapshot must not be a second producer");
        // And an empty runner is *measured, none*: empty arrays, and null for the two fields
        // that have no observation rather than a zero nobody made.
        assert_eq!(snap["tasks"], serde_json::json!([]));
        assert_eq!(snap["budgets"], serde_json::json!([]));
        assert_eq!(snap["listing"], serde_json::Value::Null);
        assert_eq!(snap["notice"], serde_json::Value::Null);
    }

    /// Every `?` field is read, never synthesised. An unset `git_version` is a real null.
    #[test]
    fn the_core_snapshot_reads_its_optional_fields_and_never_invents_them() {
        let dir = tempfile::tempdir().expect("tmp");
        let mut h = handler(dir.path());
        let snap = h.snapshot(Topic::Core);
        assert!(snap.is_object(), "core is answerable: {snap}");
        assert_eq!(
            snap["gitVersion"],
            serde_json::Value::Null,
            "never written yet"
        );
        assert_eq!(
            snap["firstRunCompletedAt"],
            serde_json::Value::Null,
            "first run has not finished in a fresh index"
        );
        assert!(
            snap["schemaVersion"].is_u64(),
            "schema version is read from the database"
        );
        assert!(snap["scan"].is_object(), "scan delegates to scan.status");
        // `runId` null is "no scan has ever run" — distinct from a run that found nothing.
        assert_eq!(snap["scan"]["runId"], serde_json::Value::Null);
    }

    /// Once written, the value is passed through rather than recomputed.
    #[test]
    fn a_recorded_git_version_reaches_the_core_snapshot() {
        let dir = tempfile::tempdir().expect("tmp");
        let mut h = handler(dir.path());
        h.index()
            .lock()
            .expect("index lock")
            .set_app_meta("git_version", "git version 2.43.0")
            .expect("write");
        let snap = h.snapshot(Topic::Core);
        assert_eq!(snap["gitVersion"], "git version 2.43.0");
    }

    /// The last hop of §10.5a's `NEW` chip. Three plans meet here: the residency card sends a
    /// patch carrying `autostart`, `settings.set` stamps `first_run_completed_at` on it, and
    /// this snapshot hands the stamp back as `firstRunCompletedAt`. Break any one and the
    /// renderer's `isNewArrival` is false for every project forever — the chip and the arrivals
    /// row never appear. It does not fail; it never shows. Driven through the real dispatcher,
    /// because that is the path the renderer actually takes.
    #[test]
    fn declining_autostart_reaches_the_snapshot_the_new_chip_reads() {
        let dir = tempfile::tempdir().expect("tmp");
        let mut h = handler(dir.path());
        assert_eq!(
            h.snapshot(Topic::Core)["firstRunCompletedAt"],
            serde_json::Value::Null,
            "the residency ask has not been answered yet"
        );

        // `LEAVE IT OFF` — the answer that changes nothing observable, and still ends first run.
        h.handle(
            "settings.set",
            serde_json::json!({"patch": {"autostart": false}}),
        )
        .expect("settings.set is answerable");

        let stamped = h.snapshot(Topic::Core)["firstRunCompletedAt"].clone();
        assert!(
            stamped.is_i64(),
            "declining is an answer, and the answer is what arms the NEW chip: {stamped}"
        );
    }

    /// The three non-fatal steps run and none of them can stop the core.
    ///
    /// The fakes make git fail (there is no real binary behind `FakeGitBackend`), which is
    /// exactly the case that must **not** be fatal: a missing git is a drawn window, never a
    /// refusal to start. So `git` is `None` here and startup still returns.
    #[test]
    fn every_startup_step_after_the_index_is_non_fatal() {
        let dir = tempfile::tempdir().expect("tmp");
        let mut h = handler(dir.path());
        let summary = codotheca_core::assembly::startup::run_startup(&mut h);

        assert!(
            summary.orphans.is_some(),
            "orphan closure runs against a real index and must succeed: {summary:?}"
        );
        // The core is still usable afterwards — the point of every step being non-fatal.
        assert!(
            h.handle("scan.status", serde_json::json!({})).is_ok(),
            "startup left the core unable to answer"
        );
        assert!(
            h.index().try_lock().is_ok(),
            "startup left the index lock held"
        );
    }

    #[test]
    fn the_pump_ticks_on_its_own_period_not_on_every_iteration() {
        use codotheca_core::session::DEFAULT_TICK_SECS;

        let dir = tempfile::tempdir().expect("tmp");
        let (mut h, _events, clock) = handler_parts(dir.path());

        for _ in 0..20 {
            h.pump();
        }
        assert_eq!(
            h.ticks(),
            1,
            "the first pump ticks; the next nineteen are inside the period"
        );

        clock.advance_ms(DEFAULT_TICK_SECS * 1_000);
        h.pump();
        assert_eq!(h.ticks(), 2, "and one lands once the period has elapsed");
    }

    /// The period is measured against `monotonic_ms`, which never decreases, and never against
    /// `now_unix`, which a user or NTP can move. A wall-clock step must not fire a burst.
    #[test]
    fn a_wall_clock_step_does_not_fire_a_burst_of_ticks() {
        let dir = tempfile::tempdir().expect("tmp");
        let (mut h, _events, clock) = handler_parts(dir.path());

        h.pump();
        assert_eq!(h.ticks(), 1);

        // A year forward and then back again, with the monotonic clock untouched.
        clock.set_unix(NOW + 60 * 60 * 24 * 365);
        for _ in 0..10 {
            h.pump();
        }
        clock.set_unix(NOW - 60 * 60 * 24 * 365);
        for _ in 0..10 {
            h.pump();
        }
        assert_eq!(h.ticks(), 1, "wall time moved; the tick period did not");
    }

    /// `shutdown`'s load-bearing half is the **drop**: the session manager holds an
    /// `Arc<PublisherSink>` clone, and `Transport::join` waits until every `FrameSink` clone is
    /// gone. Without it the process hangs on exit with its window already closed.
    #[test]
    fn shutdown_releases_the_session_managers_sink_clone() {
        let dir = tempfile::tempdir().expect("tmp");
        let (mut h, events, _clock) = handler_parts(dir.path());

        let before = Arc::strong_count(&events);
        assert!(
            before >= 3,
            "the test, the handler and the session manager each hold one: {before}"
        );

        h.shutdown();

        assert_eq!(
            Arc::strong_count(&events),
            before - 1,
            "the session manager's clone must be gone, or Transport::join never returns"
        );
        // And the core says so rather than pretending: a launch after shutdown is refused.
        let e = h
            .handle("projects.launch", serde_json::json!({}))
            .expect_err("no session manager");
        assert!(e.message.contains("shut down"), "{}", e.message);
    }

    /// Shutting down twice must not panic or double-release.
    #[test]
    fn shutdown_is_idempotent() {
        let dir = tempfile::tempdir().expect("tmp");
        let (mut h, events, _clock) = handler_parts(dir.path());
        h.shutdown();
        let after_first = Arc::strong_count(&events);
        h.shutdown();
        assert_eq!(Arc::strong_count(&events), after_first);
    }

    /// `scan.status` is answered without the index lock. If it were taken, this deadlocks:
    /// `SqliteScanStore` locks the same mutex and `std::sync::Mutex` is not reentrant.
    #[test]
    fn a_scan_command_is_answered_without_holding_the_index_lock() {
        let dir = tempfile::tempdir().expect("tmp");
        let mut h = handler(dir.path());
        let answer = h.handle("scan.status", serde_json::json!({}));
        assert!(answer.is_ok(), "scan.status: {answer:?}");
        // And the lock really is free afterwards, from this thread.
        assert!(h.index().try_lock().is_ok(), "the index lock was left held");
    }
}
