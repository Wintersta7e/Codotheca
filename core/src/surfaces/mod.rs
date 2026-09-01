//! §11's surfaces: the scan summary, settings, the two repair controls and the
//! diagnostics bundle. Each command is a plain function over the index; the
//! dispatcher below is a seam, not an owner — it answers `None` for anything it
//! does not implement so a later plan can chain its own beside it.

pub mod anonymise;
pub mod diag;
pub mod problems;
pub mod repair;
pub mod settings;
pub mod startup_failure;

use crate::index::Index;
use crate::proto::dispatch::CommandFailure;
use crate::protocol::ErrorCode;

/// Everything a §11 command needs. `now` is unix **seconds**, supplied by the
/// caller so no surface reads the clock itself.
#[derive(Debug)]
pub struct SurfaceCtx<'a> {
    pub index: &'a Index,
    pub now: i64,
}

/// §11.1's table, and nothing else, may be stored in `project.error_kind`.
pub const PROJECT_ERROR_KINDS: [ErrorCode; 6] = [
    ErrorCode::PermissionDenied,
    ErrorCode::UntrustedRepo,
    ErrorCode::RepoUnreadable,
    ErrorCode::PathGone,
    ErrorCode::StoreOffline,
    ErrorCode::BudgetExceeded,
];

#[must_use]
pub fn is_project_error_kind(code: ErrorCode) -> bool {
    PROJECT_ERROR_KINDS.contains(&code)
}

/// Every command this module owns, for the router that chains dispatchers.
///
/// A name enters this list in the same change that gives it an arm below, and the test at the
/// bottom of this file fails if the two disagree — a router built against a list the
/// dispatcher does not answer would refuse a command that is implemented.
pub const SURFACE_COMMANDS: [&str; 6] = [
    "problems.list",
    "settings.get",
    "settings.set",
    "locations.setTrusted",
    "projects.requeue",
    "diag.bundle",
];

// R15: `parse_args` is plan 03's, in `crate::proto::dispatch`. This module does not
// re-export it — each surface module imports it from there directly.

/// §1.10: `path_display` is write-once and the core never reads it back, except through the
/// one function permitted to — `index::path::display_paths_for_ui`, which
/// `core/tests/index_paths.rs` enforces by scanning the source. Every §11 surface that shows a
/// path resolves its ids here rather than joining the column into its own query.
fn display_map(
    conn: &rusqlite::Connection,
    table: crate::index::path::DisplayPathTable,
    ids: &[i64],
) -> Result<std::collections::BTreeMap<i64, String>, crate::index::IndexError> {
    Ok(crate::index::path::display_paths_for_ui(conn, table, ids)?
        .into_iter()
        .collect())
}

/// Serialises a command's return value. Every arm of the dispatcher that answers a typed
/// value goes through this one function.
fn encode<T: serde::Serialize>(value: &T) -> Result<serde_json::Value, CommandFailure> {
    serde_json::to_value(value).map_err(|e| CommandFailure::internal(e.to_string()))
}

/// `None` means "not mine". A later plan chains its own dispatcher after this one.
///
/// The arms are added by the task that writes each command's module, so this file
/// never names a function that does not exist yet.
#[must_use]
pub fn dispatch_surface_command(
    ctx: &SurfaceCtx<'_>,
    command: &str,
    args: serde_json::Value,
) -> Option<Result<serde_json::Value, CommandFailure>> {
    match command {
        "problems.list" => Some(problems::handle(ctx, args).and_then(|v| encode(&v))),
        "settings.get" => Some(settings::handle_get(ctx).and_then(|v| encode(&v))),
        "settings.set" => Some(settings::handle_set(ctx, args).and_then(|v| encode(&v))),
        "locations.setTrusted" => Some(repair::handle_set_trusted(ctx, args)),
        "projects.requeue" => Some(repair::handle_requeue(ctx, args).and_then(|v| encode(&v))),
        "diag.bundle" => Some(diag::handle(ctx, args).and_then(|v| encode(&v))),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use super::*;
    use crate::protocol::ErrorCode;

    #[test]
    fn only_the_six_kinds_section_11_1_tabulates_may_reach_project_error_kind() {
        for admissible in [
            ErrorCode::PermissionDenied,
            ErrorCode::UntrustedRepo,
            ErrorCode::RepoUnreadable,
            ErrorCode::PathGone,
            ErrorCode::StoreOffline,
            ErrorCode::BudgetExceeded,
        ] {
            assert!(
                is_project_error_kind(admissible),
                "{admissible:?} is in §11.1's table"
            );
        }
        // §11.1: these set on every project at once, so they render once in §11.2's surface.
        assert!(!is_project_error_kind(ErrorCode::GitMissing));
        assert!(!is_project_error_kind(ErrorCode::GitTooOld));
        // §11.1: app-level, and never reach project.error_kind.
        assert!(!is_project_error_kind(ErrorCode::CoreRestarted));
        assert!(!is_project_error_kind(ErrorCode::Protocol));
        assert!(!is_project_error_kind(ErrorCode::Internal));
        assert!(!is_project_error_kind(ErrorCode::ProjectMerged));
    }

    #[test]
    fn every_command_the_router_is_given_is_a_command_the_dispatcher_answers() {
        let dir = tempfile::tempdir().expect("tempdir");
        let index = Index::open(dir.path()).expect("open");
        let ctx = SurfaceCtx {
            index: &index,
            now: 1_700_000_000,
        };
        for command in SURFACE_COMMANDS {
            let args = match command {
                "settings.set" => serde_json::json!({ "patch": {} }),
                "locations.setTrusted" => serde_json::json!({ "locationId": 1 }),
                "projects.requeue" => serde_json::json!({ "id": 1 }),
                "diag.bundle" => serde_json::json!({ "includeRealPaths": false }),
                _ => serde_json::json!({}),
            };
            assert!(
                dispatch_surface_command(&ctx, command, args).is_some(),
                "{command} is in SURFACE_COMMANDS and must not fall through to None"
            );
        }
    }

    #[test]
    fn dispatch_declines_a_command_this_module_does_not_own() {
        let dir = tempfile::tempdir().expect("tempdir");
        let index = Index::open(dir.path()).expect("open");
        let ctx = SurfaceCtx {
            index: &index,
            now: 1_700_000_000,
        };
        assert!(dispatch_surface_command(&ctx, "projects.launch", serde_json::json!({})).is_none());
    }
}
