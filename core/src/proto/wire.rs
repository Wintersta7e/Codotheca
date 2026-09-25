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

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum Outbound {
    Hello {
        protocol_version: u32,
        core_version: String,
        epoch: Epoch,
        pid: u32,
    },
    Response {
        epoch: Epoch,
        id: RequestId,
        ok: Value,
    },
    /// `outcome: None` is §2.2's "definitely did not take effect, safe to retry";
    /// `Some(Outcome::Unknown)` is "may have completed" and is never auto-replayed.
    /// The field is always serialized, `null` included, so both languages round-trip it.
    Error {
        epoch: Epoch,
        id: RequestId,
        code: ErrorCode,
        message: String,
        outcome: Option<Outcome>,
    },
    Event {
        epoch: Epoch,
        topic: Topic,
        event: String,
        seq: u64,
        data: Value,
    },
    Snapshot {
        epoch: Epoch,
        topic: Topic,
        through_seq: u64,
        data: Value,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum Inbound {
    Request {
        id: RequestId,
        command: String,
        args: Value,
    },
    Subscribe {
        topic: Topic,
    },
    Unsubscribe {
        topic: Topic,
    },
    Resync {
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
