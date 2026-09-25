//! The one read of `art_scene.scene_json`.
//!
//! The trait and its production implementation land in the same change (R1, R35a, R40, R46 and
//! 16b's `renderTurn` are five recorded instances of the opposite, each of which compiled and
//! passed against the fake and failed at assembly).

use crate::art::scene::Scene;
use crate::protocol::ProjectId;
use crate::weathering::WeatheringError;

/// The scene document for a project, with the hash it is addressed by.
///
/// `Ok(None)` is *this project has no `art_scene` row* — a fact §33.8 answers with a NULL hash
/// and an empty layer set. A document that will not deserialise is
/// [`WeatheringError::UnreadableScene`] and **never** an empty reply: absent and unreadable are
/// different facts, and answering the first for the second is the conflation A13 forbids one
/// level up.
pub trait SceneSource {
    /// The project's `(scene_hash, Scene)`, or `None` when it has no `art_scene` row.
    ///
    /// # Errors
    ///
    /// [`WeatheringError::UnreadableScene`] when the stored document does not deserialise, and
    /// [`WeatheringError::Store`] when the read itself fails.
    fn scene_for(&self, id: ProjectId) -> Result<Option<(String, Scene)>, WeatheringError>;
}

/// The production source: one `SELECT` against the index.
#[derive(Debug)]
pub struct SqliteSceneSource<'a> {
    conn: &'a rusqlite::Connection,
}

impl<'a> SqliteSceneSource<'a> {
    /// A source reading through `conn`, the caller's connection or transaction view.
    #[must_use]
    pub const fn new(conn: &'a rusqlite::Connection) -> Self {
        Self { conn }
    }
}

impl SceneSource for SqliteSceneSource<'_> {
    fn scene_for(&self, id: ProjectId) -> Result<Option<(String, Scene)>, WeatheringError> {
        let mut stmt = self
            .conn
            .prepare("SELECT scene_hash, scene_json FROM art_scene WHERE project_id = ?1")
            .map_err(|e| WeatheringError::Store(id, e.to_string()))?;
        let mut rows = stmt
            .query([id.0])
            .map_err(|e| WeatheringError::Store(id, e.to_string()))?;
        let Some(row) = rows
            .next()
            .map_err(|e| WeatheringError::Store(id, e.to_string()))?
        else {
            return Ok(None);
        };
        let hash: String = row
            .get(0)
            .map_err(|e| WeatheringError::Store(id, e.to_string()))?;
        let json: String = row
            .get(1)
            .map_err(|e| WeatheringError::Store(id, e.to_string()))?;
        let scene: Scene = serde_json::from_str(&json)
            .map_err(|e| WeatheringError::UnreadableScene(id, e.to_string()))?;
        Ok(Some((hash, scene)))
    }
}
