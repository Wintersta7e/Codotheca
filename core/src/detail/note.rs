//! `projects.setNote` (§8.5.4). **One column.**
//!
//! Not `updated_at`, not `last_touched_at`: those record that the *repository* moved, and a
//! scratchpad edit is not a git fact — writing them would age a project on the shelf for typing
//! a reminder to yourself. A `null` note stores SQL NULL; an empty string would give §5.2's
//! description chain an empty first line to pick up and blank the shelf row.

use crate::detail::DetailCtx;
use crate::proto::dispatch::{parse_args, CommandFailure};
use crate::protocol::{Empty, ProjectsSetNoteArgs};

/// # Errors
/// `PROTOCOL` for an argument shape the schema does not admit or an id that names no project;
/// `INTERNAL` for an index fault.
pub fn handle_project_set_note(
    ctx: &DetailCtx<'_>,
    args: serde_json::Value,
) -> Result<Empty, CommandFailure> {
    let a: ProjectsSetNoteArgs = parse_args(args)?;
    let conn = ctx.index.conn();
    // An empty scratchpad is NULL, not `""` — §5.2 reads this column as a description source.
    let stored = a.note.as_deref().map(str::trim).filter(|s| !s.is_empty());
    let updated = conn
        .execute(
            "UPDATE project SET notes = ?2 WHERE id = ?1 AND merged_into IS NULL",
            rusqlite::params![a.id.0, stored],
        )
        .map_err(|e| CommandFailure::internal(e.to_string()))?;
    if updated == 0 {
        return Err(CommandFailure::protocol(format!("no project {}", a.id.0)));
    }

    // The open page follows its own write rather than holding an optimistic copy the core has
    // never confirmed.
    crate::detail::emit_upserted(ctx, a.id);
    Ok(Empty {})
}
