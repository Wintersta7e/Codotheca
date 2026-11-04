//! §43.3: the one emitter of `motivation/changed`, pushed and never polled.
//!
//! The core recomputes the band's payload when the index changed or the local date did, and
//! pushes it only when it differs from the last one pushed.
//!
//! The change signal is the one connection's change counter, so every committed write marks the
//! payload stale by construction: a writer added later cannot forget a hook it never needed. A
//! burst of writes between two loop iterations is one computation, not one per write.

use crate::clock::LocalZone;
use crate::index::{Index, IndexError};
use crate::proto::pubsub::EventSink;
use crate::proto::txguard::TxGuard;

use super::state::motivation_state;

/// What the last computation read, and what was last pushed.
#[derive(Debug, Default)]
pub struct MotivationPublisher {
    /// The connection's change counter right after the last computation, its own writes in it.
    changes: Option<u64>,
    /// The local day the last computation was dated in.
    day: Option<i64>,
    /// The payload last pushed, serialised.
    last: Option<serde_json::Value>,
}

impl MotivationPublisher {
    /// A publisher that has computed nothing: its first poll computes and pushes.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            changes: None,
            day: None,
            last: None,
        }
    }

    /// Recompute the band's payload if the index or the local date moved since the last
    /// computation, and push it as `motivation/changed` when it differs from the last push.
    /// `index` is under the caller's guard for the whole call, so no write lands between the
    /// computation and the counter read after it. Answers whether it pushed.
    ///
    /// # Errors
    /// Fails when the payload cannot be read or serialised; nothing is pushed then, and the
    /// next poll computes again.
    pub fn poll(
        &mut self,
        index: &Index,
        now: i64,
        zone: &dyn LocalZone,
        events: &dyn EventSink,
    ) -> Result<bool, IndexError> {
        let conn = index.conn();
        let day = zone.at(now).day();
        if self.changes == Some(conn.total_changes()) && self.day == Some(day) {
            return Ok(false);
        }
        let state = {
            let _tx_guard = TxGuard::enter();
            let tx = conn.unchecked_transaction()?;
            let state = motivation_state(&tx, now, zone)?;
            tx.commit()?;
            state
        };
        // Read after the computation: a floor its reading raised is not a change to answer.
        self.changes = Some(conn.total_changes());
        self.day = Some(day);
        let payload = serde_json::to_value(state).map_err(|e| IndexError::Corrupt {
            detail: format!("the motivation payload did not serialise: {e}"),
        })?;
        if self.last.as_ref() == Some(&payload) {
            return Ok(false);
        }
        events.emit("motivation", "changed", payload.clone());
        self.last = Some(payload);
        Ok(true)
    }
}
