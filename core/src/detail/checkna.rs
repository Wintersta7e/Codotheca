//! `projects.setCheckNa` (§31.6) — **the writer `user_na` did not have.**
//!
//! §31.4's N/A is **two stored facts, not one**: the archetype *proposes* and the user *rules*,
//! and `project_check.user_na` is NULL when the user has not. That is what lets a J3 re-run
//! re-propose **without erasing a decision the user made**.
//!
//! ```text
//! state = 'na'  iff  user_na = 1  or  (user_na IS NULL and the archetype proposes na)
//! ```
//!
//! `na = false` is the user **overriding** a proposal — the check is evaluated — and `na = null`
//! clears the override and returns the key to the proposal. Three values, not two.
//!
//! **This command is `D10 §3.2`'s one omission** (A11.1): a stored fact with no writer is a
//! feature that is off, and this one would have shipped off with every test around it green.

use crate::detail::DetailCtx;
use crate::proto::dispatch::{parse_args, CommandFailure};
use crate::protocol::{Empty, ProjectsSetCheckNaArgs};

/// # Errors
/// `PROTOCOL` for an argument shape the schema does not admit or an id that names no project;
/// `INTERNAL` for an index fault.
pub fn handle_projects_set_check_na(
    ctx: &DetailCtx<'_>,
    args: serde_json::Value,
) -> Result<Empty, CommandFailure> {
    let a: ProjectsSetCheckNaArgs = parse_args(args)?;
    let conn = ctx.index.conn();

    let exists: i64 = conn
        .query_row(
            "SELECT count(*) FROM project WHERE id = ?1 AND merged_into IS NULL",
            [a.id.0],
            |r| r.get(0),
        )
        .map_err(|e| CommandFailure::internal(e.to_string()))?;
    if exists == 0 {
        return Err(CommandFailure::protocol(format!("no project {}", a.id.0)));
    }

    // **Already what was asked for** — including *already cleared* — writes nothing and announces
    // nothing. A no-op that emitted would refresh every shelf tile on a click that changed
    // nothing.
    let current: Option<Option<i64>> = conn
        .query_row(
            "SELECT user_na FROM project_check WHERE project_id = ?1 AND check_key = ?2",
            rusqlite::params![a.id.0, slug(&a.check)],
            |r| r.get(0),
        )
        .ok();
    if current == Some(a.na.map(i64::from)) {
        return Ok(Empty {});
    }

    // The write and the recompute land in **one** transaction: a stored ruling whose ten rows
    // were not re-evaluated is a checklist disagreeing with the flag that produced it.
    let _tx_guard = crate::proto::txguard::TxGuard::enter();
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| CommandFailure::internal(e.to_string()))?;
    let written = crate::completion::set_check_na(&tx, a.id, a.check, a.na, ctx.now)
        .map_err(|e| CommandFailure::internal(format!("{e:?}")))?;
    tx.commit()
        .map_err(|e| CommandFailure::internal(e.to_string()))?;

    // The tier frame on the shelf reads `completionLit`/`completionApplicable`, so a ruling that
    // moved the denominator moved a rendered figure. A skipped or unchanged recompute moved
    // nothing and says so.
    if matches!(written, crate::completion::Written::Rewritten { .. }) {
        crate::detail::emit_upserted(ctx, a.id);
    }
    Ok(Empty {})
}

fn slug<T: serde::Serialize>(value: &T) -> String {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(raw)) => raw,
        _ => String::new(),
    }
}
