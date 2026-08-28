//! filled in by plan 10b, Tasks 1 and 2.
use crate::art::ArtCtx;
use crate::proto::dispatch::CommandFailure;

pub fn handle_url(
    _ctx: &ArtCtx<'_>,
    _args: serde_json::Value,
) -> Result<serde_json::Value, CommandFailure> {
    Err(CommandFailure::internal("art.url not yet implemented"))
}

pub fn handle_rerender(
    _ctx: &ArtCtx<'_>,
    _args: serde_json::Value,
) -> Result<serde_json::Value, CommandFailure> {
    Err(CommandFailure::internal("art.rerender not yet implemented"))
}
