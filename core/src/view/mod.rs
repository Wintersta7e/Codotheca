//! §8's shelf view state and §8.8's collections — the five commands this plan's renderer calls
//! and **R37** found no handler for in any plan.
//!
//! Nothing in here is destructive. `collections.remove` deletes one `collection` row and its
//! `collection_member` rows; §17 gives phase 1 no destructive operation at all, and no statement
//! below may grow one.
//!
//! Nothing here publishes an event either. 15b's provider re-lists after every mutation and §8.8
//! persists no active-collection key, so a `collections` topic would be a second source of truth
//! for a set the renderer has just written. **R16**'s `EventSink` is plan 03's and is consumed by
//! the plans that do publish; `ViewCtx` deliberately does not carry one.
//!
//! And no string in this module is user-facing: §2.4 gives every rendered word to the shell, and
//! §5.6 confines the dry one-line note to an opened project card.

pub mod collections;
pub mod state;

use crate::index::Index;
use crate::proto::dispatch::CommandFailure;

/// Everything a §8 command needs. `now` is unix **seconds**, supplied by the caller so no handler
/// reads the clock itself.
#[derive(Debug)]
pub struct ViewCtx<'a> {
    /// The index `view_state` and the collection tables live in.
    pub index: &'a Index,
    /// The caller's clock reading, in unix seconds; `view.set` stamps `saved_at` with it.
    pub now: i64,
}

/// The five commands this module owns, as data, so the seam and the table cannot drift apart —
/// which is the shape of the defect R37 found between the schema and the handlers.
pub const VIEW_COMMANDS: [&str; 5] = [
    "view.get",
    "view.set",
    "collections.list",
    "collections.upsert",
    "collections.remove",
];

fn encode<T: serde::Serialize>(value: &T) -> Result<serde_json::Value, CommandFailure> {
    serde_json::to_value(value).map_err(|e| CommandFailure::internal(e.to_string()))
}

/// `None` means "this module does not own that command".
///
/// Plan 21's router chains the sub-dispatchers on exactly that, so answering here for a
/// neighbour's command would take it away from the plan that owns it.
#[must_use]
pub fn dispatch_view_command(
    ctx: &ViewCtx<'_>,
    command: &str,
    args: serde_json::Value,
) -> Option<Result<serde_json::Value, CommandFailure>> {
    match command {
        "view.get" => Some(state::handle_view_get(ctx).and_then(|v| encode(&v))),
        "view.set" => Some(state::handle_view_set(ctx, args)),
        "collections.list" => {
            Some(collections::handle_collection_list(ctx).and_then(|v| encode(&v)))
        }
        "collections.upsert" => {
            Some(collections::handle_collection_upsert(ctx, args).and_then(|v| encode(&v)))
        }
        "collections.remove" => Some(collections::handle_collection_remove(ctx, args)),
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

    fn args_for(command: &str) -> serde_json::Value {
        // `deny_unknown_fields` is on every generated arg struct, so a command with required
        // fields needs them present for the dispatcher to reach its handler at all.
        match command {
            "view.set" => serde_json::json!({ "patch": {
                "query": null, "sort": null, "viewMode": null, "density": null,
                "collapsedSections": null, "scrollOffset": null, "selectedProjectId": null,
                "dismissedNotices": null, "windowGeometry": null
            }}),
            "collections.upsert" => serde_json::json!({
                "id": null, "name": "n", "kind": "query", "queryText": "lang:rust",
                "queryGrammarVersion": crate::query::QUERY_GRAMMAR_VERSION, "sortIndex": 0
            }),
            "collections.remove" => serde_json::json!({ "id": 1 }),
            _ => serde_json::json!({}),
        }
    }

    #[test]
    fn dispatch_declines_a_command_this_module_does_not_own() {
        let dir = tempfile::tempdir().expect("tempdir");
        let index = Index::open(dir.path()).expect("open");
        let ctx = ViewCtx {
            index: &index,
            now: 1_700_000_000,
        };
        // `None` is "not mine" — plan 21's router chains the sub-dispatchers on it, so claiming a
        // neighbour's command here would silently take it away from the plan that owns it.
        assert!(dispatch_view_command(&ctx, "projects.launch", serde_json::json!({})).is_none());
        assert!(dispatch_view_command(&ctx, "settings.get", serde_json::json!({})).is_none());
        assert!(dispatch_view_command(&ctx, "projects.list", serde_json::json!({})).is_none());
        assert!(dispatch_view_command(&ctx, "", serde_json::json!({})).is_none());
        // …and every name the table claims is actually answered, so the seam and the table cannot
        // drift apart the way R37 found the schema and the handlers had.
        for command in VIEW_COMMANDS {
            let answered = dispatch_view_command(&ctx, command, args_for(command));
            assert!(
                answered.is_some(),
                "{command} is in VIEW_COMMANDS and must be dispatched"
            );
            assert!(
                answered.and_then(Result::ok).is_some(),
                "{command} was dispatched but refused its own well-formed arguments"
            );
        }
    }

    #[test]
    fn nothing_in_this_module_spells_the_forbidden_token() {
        // §17: phase 1 has no destructive operation, and FORGET is the word one would be spelled
        // with. `collections.remove` deletes a saved query's name; it deletes no work.
        for command in VIEW_COMMANDS {
            assert!(!command.to_ascii_uppercase().contains("FORGET"));
        }
        assert!(!include_str!("collections.rs").contains("FORGET"));
        assert!(!include_str!("state.rs").contains("FORGET"));
    }
}
