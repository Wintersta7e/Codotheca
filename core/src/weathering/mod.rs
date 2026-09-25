//! §33's anchor resolution, beside the debt list rather than inside `core/src/art/`.
//!
//! **Boundary 2 (§27.4): decay never enters the bitmap.** `scene_hash` takes no debt input, and
//! `core/src/art/` gains no decay knowledge at all — this module reads the art module's `Scene`
//! and the art module names no symbol of this one. `core/tests/acceptance_weathering.rs` audits
//! both halves: a byte-identity criterion over two rendition files, and a comment-stripped
//! identifier scan over `core/src/art/`.
//!
//! What it does **not** read: any debt table, `health_delta`, `location`, or a clock. The reply
//! is a derivation over a stored document, so it carries no `computedAt` — the reading's age is
//! §30's `HealthState.observedAt` and is restated nowhere. The lit set is the renderer's.

pub mod anchors;
pub mod store;

use crate::index::Index;
use crate::proto::dispatch::{parse_args, CommandFailure};
use crate::protocol::{HealthWeatheringArgs, ProjectId, SceneHash, Weathering};
use crate::weathering::store::{SceneSource, SqliteSceneSource};

/// Everything §33.8's command needs. **No `now`** — the anchor set is a derivation over a stored
/// document and carries no observation time.
#[derive(Debug)]
pub struct WeatheringCtx<'a> {
    pub index: &'a Index,
}

/// The commands this module owns, as data, so the seam and the router's table cannot drift apart.
pub const WEATHERING_COMMANDS: [&str; 1] = ["health.weathering"];

/// Why an anchor set could not be produced. Each carries the project the question was about, so
/// the diagnostic says which one rather than only that one failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WeatheringError {
    /// No `project` row. A protocol error, exactly as `projects.get` answers for the same input.
    NoProject(ProjectId),
    /// `scene_json` is present and will not deserialise into a `Scene`. **Never an empty reply**:
    /// an empty layer set and an unreadable document are different facts.
    UnreadableScene(ProjectId, String),
    /// A stored space whose dimensions do not fit the wire's `u32`. A negative width is a corrupt
    /// document, and a silent `as u32` would turn it into an enormous positive.
    BadSpace(ProjectId, i32, i32),
    /// The read itself failed.
    Store(ProjectId, String),
}

impl From<WeatheringError> for CommandFailure {
    fn from(error: WeatheringError) -> Self {
        match error {
            WeatheringError::NoProject(id) => Self::protocol(format!("no project {}", id.0)),
            WeatheringError::UnreadableScene(id, why) => Self::internal(format!(
                "project {}: art_scene.scene_json is not a scene document: {why}",
                id.0
            )),
            WeatheringError::BadSpace(id, w, h) => Self::internal(format!(
                "project {}: scene space {w}x{h} is not expressible on the wire",
                id.0
            )),
            WeatheringError::Store(id, why) => Self::internal(format!("project {}: {why}", id.0)),
        }
    }
}

/// `None` means "this module does not own that command", which is what the router chains on.
#[must_use]
pub fn dispatch_weathering_command(
    ctx: &WeatheringCtx<'_>,
    command: &str,
    args: serde_json::Value,
) -> Option<Result<serde_json::Value, CommandFailure>> {
    match command {
        "health.weathering" => Some(handle_weathering(ctx, args)),
        _ => None,
    }
}

fn handle_weathering(
    ctx: &WeatheringCtx<'_>,
    args: serde_json::Value,
) -> Result<serde_json::Value, CommandFailure> {
    let args: HealthWeatheringArgs = parse_args(args)?;
    let conn = ctx.index.conn();
    let reply = weathering_for(&SqliteSceneSource::new(conn), conn, args.project_id)?;
    serde_json::to_value(reply).map_err(|e| CommandFailure::internal(e.to_string()))
}

/// §33.8's answer, over any [`SceneSource`].
///
/// `sceneHash` is NULL when the project has no `art_scene` row, and `layers` is then **empty**:
/// five empty entries and no hash would be a claim about a surface that does not exist.
pub fn weathering_for(
    source: &impl SceneSource,
    conn: &rusqlite::Connection,
    id: ProjectId,
) -> Result<Weathering, WeatheringError> {
    let exists: bool = conn
        .query_row("SELECT 1 FROM project WHERE id = ?1", [id.0], |_| Ok(true))
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(false),
            other => Err(WeatheringError::Store(id, other.to_string())),
        })?;
    if !exists {
        return Err(WeatheringError::NoProject(id));
    }

    let Some((hash, scene)) = source.scene_for(id)? else {
        return Ok(Weathering {
            project_id: id,
            scene_hash: None,
            space_w: 0,
            space_h: 0,
            layers: Vec::new(),
        });
    };

    // The two spellings of the space are reconciled here and nowhere else. `Space.w`/`h` are
    // `i32` and the wire carries `u32`; the cast **refuses** rather than truncating, because a
    // stored document whose space is negative is corrupt and a silent `as u32` would turn it into
    // an enormous positive that every percentage downstream would divide by.
    let (Ok(space_w), Ok(space_h)) = (u32::try_from(scene.space.w), u32::try_from(scene.space.h))
    else {
        return Err(WeatheringError::BadSpace(id, scene.space.w, scene.space.h));
    };

    Ok(Weathering {
        project_id: id,
        scene_hash: Some(SceneHash(hash)),
        space_w,
        space_h,
        layers: anchors::resolve_anchors(&scene),
    })
}
