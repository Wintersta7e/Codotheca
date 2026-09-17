//! `sync_task_state` — the durable scheduling state of one sync task (§21.4).

use crate::protocol::SyncTaskState;

/// Every state, in §21.4's table order. Walked by the CHECK-agreement gate, which inserts each
/// one against a **migrated database** rather than reading the DDL text.
pub const SYNC_STATES: [SyncTaskState; 6] = [
    SyncTaskState::Queued,
    SyncTaskState::Running,
    SyncTaskState::Parked,
    SyncTaskState::Ok,
    SyncTaskState::Deferred,
    SyncTaskState::Blocked,
];

/// The stored form, written into `sync_task_state.state` and constrained by that column's CHECK.
///
/// R64 keeps this name beside `core/src/art/store.rs`'s `state_slug`: two unrelated enums with
/// two unrelated CHECK lists, each the R26 mirror for its own column. Same name, different shape
/// — R15's rule, not a duplicate.
#[must_use]
pub fn state_slug(state: SyncTaskState) -> &'static str {
    match state {
        SyncTaskState::Queued => "queued",
        SyncTaskState::Running => "running",
        SyncTaskState::Parked => "parked",
        SyncTaskState::Ok => "ok",
        SyncTaskState::Deferred => "deferred",
        SyncTaskState::Blocked => "blocked",
    }
}
