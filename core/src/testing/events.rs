//! Every event a test emits is checked against the type the schema declares for it (§48.11).
//!
//! The core emits through `EventSink` as untyped JSON, so a payload can drift from its schema type
//! with every Rust test still green and only the renderer's reader finding out. Each test sink
//! calls [`validated`] first, and `PublisherSink` does under this feature, so a drifted emitter
//! fails whichever test reaches it.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};

use serde_json::Value;

/// How many payloads [`validated`] has accepted in this process, so a test can fail when it
/// checked none.
pub static VALIDATED: AtomicU64 = AtomicU64::new(0);

/// Checks `payload` against the type the schema declares for `event` on `topic`, and counts it.
///
/// # Panics
/// When the schema declares no such event, or the payload is not its type: the message names the
/// topic, the event and serde's reason.
// Failing the test that reached the emitter is the point; a test build has no caller to return to.
#[allow(clippy::panic)]
pub fn validated(topic: &str, event: &str, payload: &Value) {
    if let Err(reason) = crate::protocol::validate_event_payload(topic, event, payload) {
        panic!("{topic}/{event} is not its schema type: {reason}; payload {payload}");
    }
    VALIDATED.fetch_add(1, Ordering::Relaxed);
}

/// An `EventSink` that validates every payload and keeps it, in order.
#[derive(Debug, Default)]
pub struct ValidatingSink {
    /// Every `(topic, event, payload)` emitted so far.
    pub events: Mutex<Vec<(String, String, Value)>>,
}

impl ValidatingSink {
    /// How many events named `event` on `topic` were emitted.
    #[must_use]
    pub fn count(&self, topic: &str, event: &str) -> usize {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .filter(|(t, e, _)| t == topic && e == event)
            .count()
    }
}

impl crate::proto::EventSink for ValidatingSink {
    fn emit(&self, topic: &str, event: &str, payload: Value) {
        validated(topic, event, &payload);
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((topic.to_owned(), event.to_owned(), payload));
    }
}
