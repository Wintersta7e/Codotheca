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
