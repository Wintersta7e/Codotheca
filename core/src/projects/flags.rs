//! §1.2's three organisation primitives, and nothing adjacent to them.
//!
//! Trust is `locations.setTrusted`; re-queueing is `projects.requeue`; `slow_repo` has no
//! phase-1 writer at all (§11.1). A taxonomy call that also changed trust or scheduling would be
//! two decisions behind one command.
//!
//! **Pinning sorts nothing** (§7.8a): no band, no section membership, no order within a section,
//! no fourth SORT key, no header aggregate, no attention chip. `is:pinned` matching it is the
//! entire phase-1 payoff, and nothing here may grow a second effect.
//!
//! **Nothing here is destructive** (§17): three integer columns on one row, by id. No row is
//! removed and no byte on disk is touched.

use crate::projects::{ProjectsCtx, ProjectsError};
use crate::proto::dispatch::{parse_args, CommandFailure};
use crate::protocol::{ProjectFlagsChanged, ProjectId, ProjectsSetFlagsArgs};

/// The three flags `projects.setFlags` may change; `None` leaves that flag alone.
///
/// §2 gives this wire no optional properties, so absence is not expressible and `null` is the
/// only way to say "unchanged" — which is why `COALESCE` below is the whole of the update rule.
#[derive(Debug, Clone, Copy, Default)]
pub struct FlagPatch {
    /// The new `is_pinned`, or `None` to keep it.
    pub is_pinned: Option<bool>,
    /// The new `is_archived`, or `None` to keep it.
    pub is_archived: Option<bool>,
    /// The new `is_hidden`, or `None` to keep it.
    pub is_hidden: Option<bool>,
}

/// Writes §1.2's three columns and emits `projects/flags_changed`.
///
/// # Errors
/// `UnknownProject` when the id names no live project; `Sqlite` for anything the index refuses.
pub fn apply_flags(
    ctx: &ProjectsCtx<'_>,
    project: ProjectId,
    patch: &FlagPatch,
) -> Result<ProjectFlagsChanged, ProjectsError> {
    let conn = ctx.index.conn();
    let updated = conn.execute(
        "UPDATE project
            SET is_pinned   = COALESCE(?2, is_pinned),
                is_archived = COALESCE(?3, is_archived),
                is_hidden   = COALESCE(?4, is_hidden),
                updated_at  = ?5
          WHERE id = ?1 AND merged_into IS NULL",
        rusqlite::params![
            project.0,
            patch.is_pinned.map(i64::from),
            patch.is_archived.map(i64::from),
            patch.is_hidden.map(i64::from),
            ctx.now,
        ],
    )?;
    if updated == 0 {
        return Err(ProjectsError::UnknownProject(project.0));
    }

    let changed = conn.query_row(
        "SELECT is_pinned, is_archived, is_hidden FROM project WHERE id = ?1",
        rusqlite::params![project.0],
        |r| {
            Ok(ProjectFlagsChanged {
                id: project,
                is_pinned: r.get::<_, i64>(0)? != 0,
                is_archived: r.get::<_, i64>(1)? != 0,
                is_hidden: r.get::<_, i64>(2)? != 0,
            })
        },
    )?;

    // §7.8a: the event exists so the renderer's optimistic flip and the wire cannot disagree.
    // The command itself returns `Empty` — the schema's shape — so the flags have one carrier.
    if let Ok(payload) = serde_json::to_value(&changed) {
        ctx.events.emit("projects", "flags_changed", payload);
    }
    Ok(changed)
}

/// `projects.setFlags`. Returns `Empty`; the new flags travel on `projects/flags_changed`.
///
/// # Errors
/// `PROTOCOL` for an argument shape the schema does not admit or an id that names no project;
/// `INTERNAL` for an index fault.
pub fn handle(
    ctx: &ProjectsCtx<'_>,
    args: serde_json::Value,
) -> Result<serde_json::Value, CommandFailure> {
    let args: ProjectsSetFlagsArgs = parse_args(args)?;
    apply_flags(
        ctx,
        args.id,
        &FlagPatch {
            is_pinned: args.is_pinned,
            is_archived: args.is_archived,
            is_hidden: args.is_hidden,
        },
    )?;
    Ok(serde_json::json!({}))
}
