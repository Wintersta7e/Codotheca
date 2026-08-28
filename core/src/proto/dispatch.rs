//! The handshake and the command loop.

use crate::lifecycle::{OsParentProbe, PARENT_POLL};
use crate::proto::pubsub::Publisher;
use crate::proto::transport::{FrameSink, SendError, Transport};
use crate::proto::wire::{Epoch, Inbound, Outbound, Outcome, RequestId};
use crate::protocol::{ErrorCode, Topic, PROTOCOL_VERSION};
use serde_json::Value;

/// A refusal, ready to become an `Outbound::Error`. `message` is diagnostic and is never
/// shown to the user: the shell owns every user-facing string.
#[derive(Debug)]
pub struct CommandFailure {
    pub code: ErrorCode,
    pub message: String,
    /// `None` — the command definitely did not take effect. `Some(Outcome::Unknown)` — it may
    /// have, and §2.2 forbids auto-replaying it. There is no `failed` value on the wire.
    pub outcome: Option<Outcome>,
}

impl CommandFailure {
    #[must_use]
    pub fn protocol(message: impl Into<String>) -> Self {
        Self {
            code: ErrorCode::Protocol,
            message: message.into(),
            outcome: None,
        }
    }

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
pub fn parse_args<T: serde::de::DeserializeOwned>(args: Value) -> Result<T, CommandFailure> {
    serde_json::from_value(args).map_err(|e| CommandFailure::protocol(e.to_string()))
}

/// Everything the loop delegates. Plan 04 implements this over the index.
pub trait CommandHandler {
    /// Executes one command. `publisher` is available so a command that changes state can
    /// publish the resulting events before it returns.
    fn handle(
        &mut self,
        command: &str,
        args: Value,
        publisher: &mut Publisher,
    ) -> Result<Value, CommandFailure>;

    /// The current state of one topic, for a snapshot. Must not be called with a transaction
    /// open — the pipe write that follows would be refused.
    fn snapshot(&mut self, topic: Topic) -> Value;
}

/// The handler this plan ships. Every command is a protocol error until plan 04 lands.
#[derive(Debug, Default)]
pub struct RefusingHandler;

impl CommandHandler for RefusingHandler {
    fn handle(
        &mut self,
        command: &str,
        _args: Value,
        _publisher: &mut Publisher,
    ) -> Result<Value, CommandFailure> {
        Err(CommandFailure::protocol(format!(
            "no handler for {command}"
        )))
    }

    fn snapshot(&mut self, _topic: Topic) -> Value {
        Value::Null
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum LoopExit {
    StdinEof,
    Shutdown,
    ParentGone,
    WriterGone,
}

/// The first frame on stdout, always.
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
/// Takes the `Publisher` **by value**: it holds a `FrameSink` clone, and `Transport::join`
/// cannot finish until every clone is dropped. Borrowing it here hangs the process on exit.
pub fn run_loop(
    transport: Transport,
    mut publisher: Publisher,
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
                publisher.flush();
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break LoopExit::StdinEof,
            Ok(Inbound::Subscribe { topic }) => {
                publisher.subscribe(topic);
                let data = handler.snapshot(topic);
                publisher.supply_snapshot(topic, data);
            }
            Ok(Inbound::Unsubscribe { topic }) => publisher.unsubscribe(topic),
            Ok(Inbound::Resync { topic }) => {
                if publisher.resync(topic).is_some() {
                    let data = handler.snapshot(topic);
                    publisher.supply_snapshot(topic, data);
                }
            }
            Ok(Inbound::Request { id, command, args }) => match command.as_str() {
                "app.hello_ack" => respond(&sink, epoch, id, Ok(serde_json::json!({}))),
                "app.shutdown" => {
                    respond(&sink, epoch, id, Ok(serde_json::json!({})));
                    break LoopExit::Shutdown;
                }
                other => {
                    let result = handler.handle(other, args, &mut publisher);
                    respond(&sink, epoch, id, result);
                    // Anything the command queued goes out here, where no transaction is open.
                    publisher.flush();
                }
            },
        }
    };
    drop(sink);
    drop(publisher);
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
