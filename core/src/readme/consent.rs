//! `projects.setReadmeRemote` — the per-repository consent for remote README images.
//!
//! **NULL is *never granted*; a timestamp is *granted at T*.** The shape mirrors
//! `location.trusted_at` (§1.3) deliberately: a per-thing consent whose absence must never read
//! as a denial the user made, and whose grant carries its own date.
//!
//! **Not folded into `projects.setFlags`.** `core/src/projects/flags.rs:1-8` scopes that command
//! to §1.2's three organisation primitives and says *"nothing here may grow a second effect"*.
//! A security consent under a taxonomy command would be the exact defect that comment exists to
//! prevent, and `core/tests/readme_consent.rs` asserts the scope guard still holds.

use rusqlite::{Connection, OptionalExtension as _};

use crate::proto::dispatch::{parse_args, CommandFailure};
use crate::protocol::{ProjectId, ProjectsSetReadmeRemoteArgs, ReadmeRemoteChanged};
use crate::readme::{ReadmeCtx, ReadmeError};

/// When remote README content was allowed for `project`, or `None` for never granted.
///
/// # Errors
/// Fails when the row cannot be read.
pub fn readme_remote_at(conn: &Connection, project: ProjectId) -> Result<Option<i64>, ReadmeError> {
    let stored: Option<Option<i64>> = conn
        .query_row(
            "SELECT readme_remote_at FROM project WHERE id = ?1 AND merged_into IS NULL",
            rusqlite::params![project.0],
            |r| r.get::<_, Option<i64>>(0),
        )
        .optional()?;
    // The outer `Option` is *no such project*, the inner is *never granted*. Flattening them
    // here would make a missing project read as a consent decision nobody took, so the caller
    // is handed the refusal instead.
    stored.ok_or(ReadmeError::UnknownSubject)
}

/// Grant or revoke, and publish what the row now holds.
///
/// `allow: true` writes `ctx.now`; `allow: false` writes NULL. The event carries the stored
/// value — never the boolean that was asked for — so an optimistic flip in the renderer and the
/// wire cannot disagree, which is the `projects/flags_changed` precedent
/// (`core/src/projects/flags.rs:69-73`).
///
/// # Errors
/// `UnknownSubject` when the id names no live project; `Sqlite` for an index fault.
pub fn set_readme_remote(
    ctx: &ReadmeCtx<'_>,
    project: ProjectId,
    allow: bool,
) -> Result<ReadmeRemoteChanged, ReadmeError> {
    let allowed_at = if allow { Some(ctx.now) } else { None };
    let conn = ctx.index.conn();
    let updated = conn.execute(
        "UPDATE project SET readme_remote_at = ?2, updated_at = ?3
          WHERE id = ?1 AND merged_into IS NULL",
        rusqlite::params![project.0, allowed_at, ctx.now],
    )?;
    if updated == 0 {
        return Err(ReadmeError::UnknownSubject);
    }

    // Read back rather than echo: what the renderer reconciles against is the column, and a
    // write that silently did something else must not be reported as the write that was asked
    // for.
    let stored = readme_remote_at(conn, project)?;
    let changed = ReadmeRemoteChanged {
        id: project,
        allowed_at: stored,
    };
    if let Ok(payload) = serde_json::to_value(&changed) {
        ctx.events
            .emit("projects", "readme_remote_changed", payload);
    }
    Ok(changed)
}

/// `projects.setReadmeRemote`. Returns `Empty`; the new value travels on the event.
///
/// # Errors
/// `PROTOCOL` for an argument shape the schema does not admit or an id that names no live
/// project; `INTERNAL` for an index fault.
pub fn handle_set_readme_remote(
    ctx: &ReadmeCtx<'_>,
    args: serde_json::Value,
) -> Result<serde_json::Value, CommandFailure> {
    let args: ProjectsSetReadmeRemoteArgs = parse_args(args)?;
    set_readme_remote(ctx, args.project_id, args.allow)?;
    Ok(serde_json::json!({}))
}
