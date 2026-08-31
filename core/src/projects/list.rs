//! filled in by Task 17.
use crate::projects::ProjectsCtx;
use crate::proto::dispatch::CommandFailure;

pub fn handle(
    _ctx: &ProjectsCtx<'_>,
    _args: serde_json::Value,
) -> Result<serde_json::Value, CommandFailure> {
    Err(CommandFailure::internal(
        "projects.list not yet implemented",
    ))
}
