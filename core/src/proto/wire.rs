//! The frame envelope. Every byte on stdout is one `Outbound`; every byte on stdin one
//! `Inbound`. Mirrored field for field by `app/src/main/core/wire.ts`.

use crate::protocol::{ErrorCode, Topic};
use serde_json::Value;

/// R31: `Outcome` is declared in `protocol/schema/protocol.json` and generated into
/// `crate::protocol`. Re-exported here so `proto::wire::Outcome` still names it, and declared
/// nowhere else — a second hand-written copy compiles and then drifts from the wire form.
pub use crate::protocol::Outcome;

/// Scopes request ids and event sequences. Assigned by the shell, echoed by the core.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(transparent)]
pub struct Epoch(pub u64);

/// Unique within one epoch. Assigned by the shell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct RequestId(pub u64);

/// Every frame the core writes to the shell.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum Outbound {
    /// The first frame on stdout (§2.2). The shell acks it, or refuses on a version mismatch.
    Hello {
        /// This build's `PROTOCOL_VERSION`, which the shell compares with its own.
        protocol_version: u32,
        /// The core's crate version.
        core_version: String,
        /// The epoch the shell passed in argv; it scopes every frame after this one.
        epoch: Epoch,
        /// The core's process id.
        pid: u32,
    },
    /// A command's success.
    Response {
        /// The epoch the request was made in.
        epoch: Epoch,
        /// The request answered.
        id: RequestId,
        /// The command's result.
        ok: Value,
    },
    /// `outcome: None` is §2.2's "definitely did not take effect, safe to retry";
    /// `Some(Outcome::Unknown)` is "may have completed" and is never auto-replayed.
    /// The field is always serialized, `null` included, so both languages round-trip it.
    Error {
        /// The epoch the request was made in.
        epoch: Epoch,
        /// The request answered.
        id: RequestId,
        /// What went wrong, as the code the shell chooses its words from.
        code: ErrorCode,
        /// Diagnostic text for the log; never shown to the user.
        message: String,
        /// Whether the command may have taken effect, as above.
        outcome: Option<Outcome>,
    },
    /// One delta on a subscribed topic.
    Event {
        /// The epoch it was published in.
        epoch: Epoch,
        /// The topic it belongs to.
        topic: Topic,
        /// The event's name within the topic.
        event: String,
        /// Its place in the topic's sequence; a gap tells the consumer to resync.
        seq: u64,
        /// The event's payload.
        data: Value,
    },
    /// A topic's whole state, standing in for every delta up to `through_seq` (§2.3).
    Snapshot {
        /// The epoch it was published in.
        epoch: Epoch,
        /// The topic it describes.
        topic: Topic,
        /// The last sequence number it covers; only deltas after it may follow.
        through_seq: u64,
        /// The topic's state, from the handler's `snapshot`.
        data: Value,
    },
}

/// Every frame the shell writes to the core.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum Inbound {
    /// One command.
    Request {
        /// Assigned by the shell and echoed on the answer.
        id: RequestId,
        /// The command's name, one of §2.4's surface.
        command: String,
        /// The command's arguments, deserialised by its handler.
        args: Value,
    },
    /// Start a topic: a snapshot now, its deltas after.
    Subscribe {
        /// The topic to start.
        topic: Topic,
    },
    /// Stop a topic; whatever it still had queued is dropped.
    Unsubscribe {
        /// The topic to stop.
        topic: Topic,
    },
    /// The consumer saw a gap in a topic's sequence and wants a fresh snapshot.
    Resync {
        /// The topic to snapshot again.
        topic: Topic,
    },
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{Inbound, Outbound};

    fn samples() -> serde_json::Value {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../protocol/wire-samples.json");
        let text = std::fs::read_to_string(path).expect("wire-samples.json");
        serde_json::from_str(&text).expect("valid json")
    }

    #[test]
    fn every_outbound_sample_round_trips_byte_for_byte() {
        let s = samples();
        let list = s
            .get("outbound")
            .and_then(serde_json::Value::as_array)
            .expect("outbound");
        assert_eq!(list.len(), 5);
        for value in list {
            let frame: Outbound =
                serde_json::from_value(value.clone()).expect("sample must deserialise");
            let back = serde_json::to_value(&frame).expect("re-serialise");
            assert_eq!(&back, value, "field names drifted");
        }
    }

    #[test]
    fn every_inbound_sample_round_trips_byte_for_byte() {
        let s = samples();
        let list = s
            .get("inbound")
            .and_then(serde_json::Value::as_array)
            .expect("inbound");
        assert_eq!(list.len(), 4);
        for value in list {
            let frame: Inbound =
                serde_json::from_value(value.clone()).expect("sample must deserialise");
            let back = serde_json::to_value(&frame).expect("re-serialise");
            assert_eq!(&back, value, "field names drifted");
        }
    }
}
