#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! The loop's side of the composition: one publisher, no deadlock, a clean exit.

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

        let handler = CoreHandler::new(CoreDeps {
            index: Arc::clone(&index),
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
            events: Arc::clone(&events),
            tz_offset_min: 0,
        });
        (handler, events, clock)
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
        let mut checked = 0_u32;
        for name in &names {
            if loop_only.contains(&name.as_str()) {
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
        assert_eq!(
            checked, 40,
            "the schema's answerable set, minus the loop's pair"
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
