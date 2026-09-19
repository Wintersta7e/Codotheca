//! The opened project page's core half (§8.5): one project's detail, its scratchpad note, and
//! RELOCATE. Plan 13's module answers the shelf's `projects.list` / `peek` / `setFlags`; this
//! one answers a disjoint set of three, and the two are kept apart on purpose — a
//! `dispatch_project_command` beside a `dispatch_projects_command` would have made one letter
//! the only thing between a command and the wrong module.
//!
//! **Nothing here is destructive** (§17). RELOCATE rewrites one `location` row's path columns:
//! no file is moved, deleted, created or opened for writing, no git command mutates anything,
//! and no row is removed.

pub mod checkna;
pub mod get;
pub mod note;
pub mod relocate;

use crate::git::GitBackend;
use crate::index::Index;
use crate::mount::MountResolver;
use crate::proto::dispatch::CommandFailure;
use crate::proto::EventSink;

// R15: `parse_args` is plan 03's, in `crate::proto::dispatch`. It is not declared here and not
// re-exported, so there is one path to it; each handler imports it directly.
// R16: `EventSink` is plan 03's trait, declared beside `Publisher`.

/// Everything the three commands need. `now` is unix **seconds**, supplied by the caller so no
/// handler reads the clock itself.
pub struct DetailCtx<'a> {
    pub index: &'a Index,
    pub git: &'a dyn GitBackend,
    pub mount: &'a dyn MountResolver,
    pub events: &'a dyn EventSink,
    /// §6: an opened page asks for a current worktree reading for the copy it is showing.
    pub jobs: &'a dyn crate::jobs::JobSink,
    /// §21.5: an opened page also asks for its **remote** facts, at the priority of the thing the
    /// user is looking at. One of exactly two sites.
    pub sync: &'a dyn crate::sync::runner::SyncSink,
    pub now: i64,
}

impl std::fmt::Debug for DetailCtx<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DetailCtx")
            .field("now", &self.now)
            .finish_non_exhaustive()
    }
}

fn encode<T: serde::Serialize>(value: &T) -> Result<serde_json::Value, CommandFailure> {
    serde_json::to_value(value).map_err(|e| CommandFailure::internal(e.to_string()))
}

/// `projects/upserted` carries a whole `ProjectRow` (`ProjectUpserted`), so the two writes here
/// re-read the row through plan 13's projection rather than publishing an id-shaped payload the
/// generated type could not deserialise. A row that cannot be re-read publishes nothing: the
/// write already succeeded, and inventing an event for it would be worse than a missed refresh.
fn emit_upserted(ctx: &DetailCtx<'_>, project: crate::protocol::ProjectId) {
    if let Some(value) = upserted_payload(ctx.index.conn(), project) {
        ctx.events.emit("projects", "upserted", value);
    }
}

/// The `projects/upserted` payload, built **once** for every emitter.
///
/// [p3] `JobRunner::settle` emits this event too (R121), and a second hand-built payload there
/// would be the same whole `ProjectRow` assembled by two producers — R12's rule applied to an
/// event rather than a formatter. `None` is *the row could not be re-read*, which publishes
/// nothing: the write already succeeded, and inventing an event for it would be worse than a
/// missed refresh.
#[must_use]
pub fn upserted_payload(
    conn: &rusqlite::Connection,
    project: crate::protocol::ProjectId,
) -> Option<serde_json::Value> {
    let loaded = crate::projects::rows::load_project_row(conn, project).ok()?;
    let payload = crate::protocol::ProjectUpserted { row: loaded.row };
    serde_json::to_value(&payload).ok()
}

/// The commands this module owns, in the order the dispatcher matches them. Exposed so the
/// table can be asserted without constructing an `Index`.
#[must_use]
pub fn dispatch_detail_command_names() -> [&'static str; 4] {
    [
        "projects.get",
        "projects.setNote",
        // [p3] §31.6's writer. It writes one `project_check.user_na`, re-runs §31's evaluator in
        // the same transaction and re-emits the row — all three of which this module owns.
        "projects.setCheckNa",
        "locations.relocate",
    ]
}

/// `None` means "this module does not own that command" — plan 21's router chains on it. A
/// dispatcher that returned `Err(PROTOCOL)` for a stranger's command would stop the chain at
/// whichever module happened to be asked first, and the failure would name the wrong owner.
#[must_use]
pub fn dispatch_detail_command(
    ctx: &DetailCtx<'_>,
    command: &str,
    args: serde_json::Value,
) -> Option<Result<serde_json::Value, CommandFailure>> {
    match command {
        "projects.get" => Some(get::handle_project_get(ctx, args).and_then(|v| encode(&v))),
        "projects.setNote" => {
            Some(note::handle_project_set_note(ctx, args).and_then(|v| encode(&v)))
        }
        "projects.setCheckNa" => {
            Some(checkna::handle_projects_set_check_na(ctx, args).and_then(|v| encode(&v)))
        }
        "locations.relocate" => {
            Some(relocate::handle_location_relocate(ctx, args).and_then(|v| encode(&v)))
        }
        _ => None,
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
mod tests {
    use super::*;

    #[test]
    fn dispatch_declines_a_command_this_module_does_not_own() {
        let names = dispatch_detail_command_names();
        assert_eq!(
            names,
            [
                "projects.get",
                "projects.setNote",
                "projects.setCheckNa",
                "locations.relocate"
            ]
        );
        // Plan 13's set. A dispatcher that answered one of these would take a command from a
        // module that implements it properly, and plan 21's router would never reach it.
        for theirs in ["projects.list", "projects.peek", "projects.setFlags"] {
            assert!(
                !names.contains(&theirs),
                "{theirs} is plan 13's, not this module's"
            );
        }
    }
}
