//! `sync.status`, and nothing else.
//!
//! **No manual refresh command ships in phase 2** (§21.5). It cannot make the budget larger, and
//! every trigger that matters — connect, scope change, page open, Peek open — is already
//! automatic. A control that only re-queues something already queued is furniture.
//!
//! **R94's first side.** This is a handler path: `Assembly` already holds the one
//! `Arc<Mutex<Index>>` guard when it dispatches, so this context takes a `&Index` from its caller
//! and never locks one. A signature taking `&Arc<Mutex<Index>>` here would self-deadlock —
//! `std::sync::Mutex` is not reentrant — and wedge the guard for the life of the process.

use serde_json::Value;

use crate::index::Index;
use crate::proto::dispatch::CommandFailure;
use crate::protocol::CommandName;
use crate::sync::events::{status_payload, SyncLive};

/// What `sync.status` reads.
#[derive(Debug)]
pub struct SyncCtx<'a> {
    pub index: &'a Index,
    /// The runner's process-lifetime half. Empty before the pump has observed anything.
    pub live: SyncLive,
}

/// Answer one `sync.*` command, or decline it so the router's own disagreement is named.
///
/// # Errors
/// `INTERNAL` when the index cannot be read.
#[must_use]
pub fn dispatch_sync_command(
    ctx: &SyncCtx<'_>,
    command: CommandName,
    _args: Value,
) -> Option<Result<Value, CommandFailure>> {
    match command {
        CommandName::SyncStatus => Some(
            status_payload(ctx.index.conn(), &ctx.live)
                .map_err(|e| CommandFailure::internal(e.to_string()))
                .and_then(|status| {
                    serde_json::to_value(status)
                        .map_err(|e| CommandFailure::internal(e.to_string()))
                }),
        ),
        _ => None,
    }
}
