//! A no-transport `EventSink` for this module's tests.
//!
//! **Gated on `testkit`, unlike the plan body, which compiles it always.** The plan's reason for
//! always-on was that integration tests under `core/tests/` cannot see a `#[cfg(test)]` item —
//! true, and exactly what the `testkit` feature already exists for: plan 06 gates `core::testing`
//! the same way so that "the shipped binary contains no fake clock, no fake mount table and no
//! fixture builder". An always-compiled recording sink would put a test double back into the
//! release binary for no gain, since every gate already runs `--features testkit`.

use std::sync::Mutex;

/// Records what was emitted, in order.
#[derive(Debug, Default)]
pub struct CollectingSink {
    emitted: Mutex<Vec<(String, String, serde_json::Value)>>,
}

impl CollectingSink {
    #[must_use]
    pub fn events(&self) -> Vec<(String, String, serde_json::Value)> {
        self.emitted.lock().map(|v| v.clone()).unwrap_or_default()
    }

    #[must_use]
    pub fn named(&self, topic: &str, event: &str) -> Vec<serde_json::Value> {
        self.events()
            .into_iter()
            .filter(|(t, e, _)| t == topic && e == event)
            .map(|(_, _, data)| data)
            .collect()
    }

    pub fn clear(&self) {
        if let Ok(mut held) = self.emitted.lock() {
            held.clear();
        }
    }
}

impl crate::proto::EventSink for CollectingSink {
    fn emit(&self, topic: &str, event: &str, payload: serde_json::Value) {
        if let Ok(mut held) = self.emitted.lock() {
            held.push((topic.to_owned(), event.to_owned(), payload));
        }
    }
}
