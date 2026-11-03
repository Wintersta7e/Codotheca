//! §40.10's two commands: `badges.list`, which the reveal and Profile read, and
//! `badges.markLanded`, the one landing writer.
//!
//! Neither is privileged: the list reads the index and the landing writes one column of it, and
//! neither touches a user file or git.

use crate::clock::LocalZone;
use crate::index::Index;
use crate::proto::dispatch::{parse_args, CommandFailure};
use crate::proto::txguard::TxGuard;
use crate::protocol::{BadgesListArgs, BadgesListed, BadgesMarkLandedArgs, Empty};

use super::evaluate::EvalCtx;
use super::landing::{due, list_awards, mark_landed};
use super::population::history_basis;
use super::BadgeError;

/// What both commands are answered with.
#[derive(Debug)]
pub struct BadgesCtx<'a> {
    /// The index, under the handler's guard.
    pub index: &'a Index,
    /// The command's instant, in Unix seconds: the local date `due` is computed for, and the
    /// moment a landing records.
    pub now: i64,
    /// The machine's zone, in which that local date is read.
    pub zone: &'a dyn LocalZone,
}

impl From<BadgeError> for CommandFailure {
    fn from(e: BadgeError) -> Self {
        match e {
            BadgeError::NotAwarded(_) => Self::protocol(e.to_string()),
            BadgeError::Index(_) | BadgeError::Evidence(_) => Self::internal(e.to_string()),
        }
    }
}

/// `None` means "this module does not own that command", which is what the router chains on.
#[must_use]
pub fn dispatch_badges_command(
    ctx: &BadgesCtx<'_>,
    command: &str,
    args: serde_json::Value,
) -> Option<Result<serde_json::Value, CommandFailure>> {
    match command {
        "badges.list" => Some(handle_list(ctx, args)),
        "badges.markLanded" => Some(handle_mark_landed(ctx, args)),
        _ => None,
    }
}

const fn eval_ctx<'a>(ctx: &BadgesCtx<'a>) -> EvalCtx<'a> {
    EvalCtx {
        now: ctx.now,
        zone: ctx.zone,
    }
}

fn handle_list(
    ctx: &BadgesCtx<'_>,
    args: serde_json::Value,
) -> Result<serde_json::Value, CommandFailure> {
    let BadgesListArgs {} = parse_args(args)?;
    let conn = ctx.index.conn();
    let listed = BadgesListed {
        awards: list_awards(conn)?,
        due: due(conn, &eval_ctx(ctx))?,
        basis: history_basis(conn).map_err(BadgeError::from)?,
    };
    serde_json::to_value(listed).map_err(|e| CommandFailure::internal(e.to_string()))
}

fn handle_mark_landed(
    ctx: &BadgesCtx<'_>,
    args: serde_json::Value,
) -> Result<serde_json::Value, CommandFailure> {
    let args: BadgesMarkLandedArgs = parse_args(args)?;
    let _tx_guard = TxGuard::enter();
    let tx = ctx
        .index
        .conn()
        .unchecked_transaction()
        .map_err(|e| CommandFailure::internal(e.to_string()))?;
    mark_landed(&tx, args.kind, &eval_ctx(ctx))?;
    tx.commit()
        .map_err(|e| CommandFailure::internal(e.to_string()))?;
    serde_json::to_value(Empty {}).map_err(|e| CommandFailure::internal(e.to_string()))
}
