//! §11's surfaces: the scan summary, settings, the two repair controls and the
//! diagnostics bundle. Each command is a plain function over the index; the
//! dispatcher below is a seam, not an owner — it answers `None` for anything it
//! does not implement so a later plan can chain its own beside it.

pub mod problems;
pub mod settings;

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

// R15: `parse_args` is plan 03's, in `crate::proto::dispatch`. This module does not
// re-export it — each surface module imports it from there directly.

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
