//! filled in by Task 18.
use crate::projects::ProjectsCtx;
use crate::proto::dispatch::CommandFailure;

pub fn handle(
    _ctx: &ProjectsCtx<'_>,
    _args: serde_json::Value,
) -> Result<serde_json::Value, CommandFailure> {
    Err(CommandFailure::internal(
        "projects.peek not yet implemented",
    ))
}
