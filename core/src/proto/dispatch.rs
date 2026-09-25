//! The handshake and the command loop.

use crate::lifecycle::{OsParentProbe, PARENT_POLL};
use crate::proto::pubsub::{Publisher, PublisherSink};
use crate::proto::transport::{FrameSink, SendError, Transport};
use crate::proto::wire::{Epoch, Inbound, Outbound, Outcome, RequestId};
use crate::protocol::{ErrorCode, Topic, PROTOCOL_VERSION};
use serde_json::Value;

/// A refusal, ready to become an `Outbound::Error`. `message` is diagnostic and is never
/// shown to the user: the shell owns every user-facing string.
#[derive(Debug)]
pub struct CommandFailure {
    /// What went wrong, as the code the shell chooses its words from.
    pub code: ErrorCode,
    /// Diagnostic text for the log.
    pub message: String,
    /// `None` — the command definitely did not take effect. `Some(Outcome::Unknown)` — it may
    /// have, and §2.2 forbids auto-replaying it. There is no `failed` value on the wire.
    pub outcome: Option<Outcome>,
}

impl CommandFailure {
    /// A `PROTOCOL` refusal: the request itself was malformed, so it did not take effect.
    #[must_use]
    pub fn protocol(message: impl Into<String>) -> Self {
        Self {
            code: ErrorCode::Protocol,
            message: message.into(),
            outcome: None,
        }
    }

    /// An `INTERNAL` refusal: a defect in the core, reported as not having taken effect.
    #[must_use]
    pub fn internal(message: impl Into<String>) -> Self {
        Self {
            code: ErrorCode::Internal,
            message: message.into(),
            outcome: None,
        }
    }
}

/// Deserialises one **command's** `args` object, turning a shape error into `PROTOCOL` (**R15**).
///
/// It lives here because `CommandFailure` does. Not to be confused with
/// `crate::lifecycle::parse_args`, which parses **argv** into `CoreArgs` — a different function
/// with a different job that happens to share a name across two modules.
///
/// # Errors
/// A `PROTOCOL` failure when `args` does not deserialise as a `T`.
pub fn parse_args<T: serde::de::DeserializeOwned>(args: Value) -> Result<T, CommandFailure> {
    serde_json::from_value(args).map_err(|e| CommandFailure::protocol(e.to_string()))
}

/// Everything the loop delegates.
///
/// **The handler publishes through the `Arc<dyn EventSink>` it holds, never through a borrowed
/// `Publisher`.** There is one `Publisher` in the process and `PublisherSink` owns it; handing a
/// second reference to it here would either duplicate the per-topic sequence counters or
/// deadlock the loop against a handler that emits while it runs.
pub trait CommandHandler {
    /// Executes one command.
    ///
    /// # Errors
    /// A `CommandFailure` when the command is unknown, its arguments are malformed, or it is
    /// refused; its `outcome` says whether it may have taken effect.
    fn handle(&mut self, command: &str, args: Value) -> Result<Value, CommandFailure>;

    /// The current state of one topic, for a snapshot. Must not be called with a transaction
    /// open — the pipe write that follows would be refused.
    fn snapshot(&mut self, topic: Topic) -> Value;

    /// Called once per loop iteration, request or timeout. Periodic work goes here; the loop
    /// wakes at least every `PARENT_POLL`, so a job on a longer period keeps its own deadline.
    fn pump(&mut self) {}

    /// Called after the loop breaks and before the transport is joined. Every `FrameSink` clone
    /// the handler holds — directly or through an `Arc<PublisherSink>` it handed to a worker —
    /// must be dropped here, or `Transport::join` never returns.
    fn shutdown(&mut self) {}
}

/// The handler this plan ships. Every command is a protocol error until plan 04 lands.
#[derive(Debug, Default)]
pub struct RefusingHandler;

impl CommandHandler for RefusingHandler {
    fn handle(&mut self, command: &str, _args: Value) -> Result<Value, CommandFailure> {
        Err(CommandFailure::protocol(format!(
            "no handler for {command}"
        )))
    }

    fn snapshot(&mut self, _topic: Topic) -> Value {
        Value::Null
    }
}

/// Why `run_loop` stopped.
#[derive(Debug, PartialEq, Eq)]
pub enum LoopExit {
    /// The inbound channel disconnected: stdin ended or carried a frame that did not decode.
    StdinEof,
    /// The shell sent `app.shutdown`.
    Shutdown,
    /// The parent process is provably gone, or its pid now belongs to another process.
    ParentGone,
    /// The writer thread is gone. Nothing constructs this: the loop does not watch the writer.
    WriterGone,
}

/// The first frame on stdout, always.
///
/// # Errors
/// Those of `FrameSink::send`: `Closed` when the writer thread is already gone, and
/// `InTransaction` if called with a transaction open on this thread.
pub fn send_hello(sink: &FrameSink, epoch: Epoch) -> Result<(), SendError> {
    sink.send(&Outbound::Hello {
        protocol_version: PROTOCOL_VERSION,
        core_version: env!("CARGO_PKG_VERSION").to_owned(),
        epoch,
        pid: std::process::id(),
    })
}

fn respond(sink: &FrameSink, epoch: Epoch, id: RequestId, result: Result<Value, CommandFailure>) {
    let frame = match result {
        Ok(ok) => Outbound::Response { epoch, id, ok },
        Err(f) => Outbound::Error {
            epoch,
            id,
            code: f.code,
            message: f.message,
            outcome: f.outcome,
        },
    };
    if let Err(SendError::TooLarge { len }) = sink.send(&frame) {
        let _ = sink.send(&Outbound::Error {
            epoch,
            id,
            code: ErrorCode::Internal,
            message: format!("response of {len} bytes exceeds the frame cap"),
            outcome: None,
        });
    }
}

/// Runs until stdin closes, `app.shutdown` arrives, or the parent goes away.
///
/// Takes the sink **by reference**: the caller keeps an `Arc` so the handler and its workers can
/// share the one publisher that owns the per-topic sequence counters.
pub fn run_loop(
    transport: Transport,
    events: &std::sync::Arc<PublisherSink>,
    handler: &mut dyn CommandHandler,
    epoch: Epoch,
    parent: &OsParentProbe,
) -> LoopExit {
    let sink = transport.sink.clone();
    let exit = loop {
        match transport.inbound.recv_timeout(PARENT_POLL) {
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                if parent.parent_gone() {
                    break LoopExit::ParentGone;
                }
                handler.pump();
                events.with(Publisher::flush);
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break LoopExit::StdinEof,
            Ok(Inbound::Subscribe { topic }) => {
                events.with(|publisher| publisher.subscribe(topic));
                let data = handler.snapshot(topic);
                events.with(|publisher| publisher.supply_snapshot(topic, data));
                handler.pump();
            }
            Ok(Inbound::Unsubscribe { topic }) => {
                events.with(|publisher| publisher.unsubscribe(topic));
                handler.pump();
            }
            Ok(Inbound::Resync { topic }) => {
                if events.with(|publisher| publisher.resync(topic)).is_some() {
                    let data = handler.snapshot(topic);
                    events.with(|publisher| publisher.supply_snapshot(topic, data));
                }
                handler.pump();
            }
            Ok(Inbound::Request { id, command, args }) => {
                let exit = match command.as_str() {
                    "app.hello_ack" => {
                        respond(&sink, epoch, id, Ok(serde_json::json!({})));
                        None
                    }
                    "app.shutdown" => {
                        respond(&sink, epoch, id, Ok(serde_json::json!({})));
                        Some(LoopExit::Shutdown)
                    }
                    other => {
                        let result = handler.handle(other, args);
                        respond(&sink, epoch, id, result);
                        None
                    }
                };

                // A topic that overflowed while the command ran wants a fresh snapshot, and
                // only the handler can compute one. The lock is released around that call.
                for topic in events.take_snapshot_requests() {
                    let data = handler.snapshot(topic);
                    events.with(|publisher| publisher.supply_snapshot(topic, data));
                }
                handler.pump();
                // Anything the command queued goes out here, where no transaction is open.
                events.with(Publisher::flush);

                if let Some(exit) = exit {
                    break exit;
                }
            }
        }
    };
    handler.shutdown();
    events.with(Publisher::close);
    drop(sink);
    transport.join();
    exit
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{parse_args, CommandFailure};
    use crate::protocol::ErrorCode;

    #[derive(serde::Deserialize)]
    struct Args {
        id: u32,
    }

    #[test]
    fn command_arguments_of_the_wrong_shape_are_a_protocol_failure_not_a_panic() {
        let e: CommandFailure = parse_args::<Args>(serde_json::json!({ "id": "seven" }))
            .err()
            .expect("a string where a number belongs must be refused");
        assert_eq!(e.code, ErrorCode::Protocol);
        assert!(
            !e.message.is_empty(),
            "the diagnostic is for the log, never for the user"
        );
    }

    #[test]
    fn well_shaped_command_arguments_deserialise() {
        let a: Args = parse_args(serde_json::json!({ "id": 7 })).expect("must accept");
        assert_eq!(a.id, 7);
    }
}
