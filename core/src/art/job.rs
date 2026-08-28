//! J5 (§4.1): scene + bitmaps, on a CPU pool, off the git path.
//!
//! Idempotent by construction (ruling 7): if the recomputed hash matches the stored one, the
//! card file is there and the state is `ready`, it writes nothing and emits nothing. That is
//! what makes §7.1a's "no card changes appearance while the user is watching it" a property of
//! the data rather than only of the swap.

use crate::art::generate::{generate, load_inputs};
use crate::art::scene::scene_hash;
use crate::art::store::{
    load_row, put_scene, record_failure, rendition_exists, state_slug, write_rendition,
};
use crate::art::ArtError;
use crate::index::Index;
use crate::jobs::{JobError, JobOutcome};
use crate::protocol::{ArtState, Rendition};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtRenderOutcome {
    pub scene_hash: String,
    /// False when the scene and its file were already current, so nothing was written.
    pub rendered: bool,
}

/// Generate, compare, and draw only if something moved.
pub fn render_card(index: &Index, project_id: i64, now: i64) -> Result<ArtRenderOutcome, ArtError> {
    let inputs = load_inputs(index.conn(), project_id)?.ok_or(ArtError::NoScene(project_id))?;
    let scene = generate(&inputs);
    let hash = scene_hash(&scene)?;
    let data_dir = index.data_dir();
    let current = load_row(index.conn(), project_id)?;
    let unchanged = current.as_ref().is_some_and(|row| {
        row.scene_hash == hash
            && row.state == ArtState::Ready
            && row.schema_version == crate::art::ART_SCHEMA_VERSION
    });
    if unchanged && rendition_exists(data_dir, &hash, Rendition::Card) {
        return Ok(ArtRenderOutcome {
            scene_hash: hash,
            rendered: false,
        });
    }
    write_rendition(data_dir, &hash, Rendition::Card, &scene)?;
    put_scene(
        index.conn(),
        project_id,
        &hash,
        &scene,
        ArtState::Ready,
        Some(now),
    )?;
    Ok(ArtRenderOutcome {
        scene_hash: hash,
        rendered: true,
    })
}

/// The scheduler's entry point. A failure increments §7.5's per-scene counter and asks for a
/// retry; the scheduler's own counter decides when to stop asking.
pub fn run_j5(index: &Index, project_id: i64, now: i64) -> Result<JobOutcome, JobError> {
    match render_card(index, project_id, now) {
        Ok(_) => Ok(JobOutcome::Done),
        Err(ArtError::NoScene(id)) => Ok(JobOutcome::HardFail {
            error_kind: "INTERNAL",
            detail: format!("no project row for {id}"),
        }),
        Err(err) => {
            // The counter is best-effort: if the row is not there yet, there is nothing to count.
            let _ = record_failure(index.conn(), project_id);
            Ok(JobOutcome::TransientFail {
                reason: err.to_string(),
            })
        }
    }
}

/// True when this project's **card** needs drawing on the strength of its row alone: no row, a
/// row behind the schema, or a state that is not `ready`.
pub fn needs_art(conn: &rusqlite::Connection, project_id: i64) -> Result<bool, ArtError> {
    let Some(row) = load_row(conn, project_id)? else {
        return Ok(true);
    };
    Ok(row.state != ArtState::Ready || row.schema_version != crate::art::ART_SCHEMA_VERSION)
}

/// The same question including §7.5's missing-file case — a `ready` row whose bitmap has gone.
/// This is the form the scheduler's visible-tile hook uses, because it has the data directory.
pub fn needs_art_at(
    conn: &rusqlite::Connection,
    data_dir: &std::path::Path,
    project_id: i64,
) -> Result<bool, ArtError> {
    let Some(row) = load_row(conn, project_id)? else {
        return Ok(true);
    };
    if row.state != ArtState::Ready || row.schema_version != crate::art::ART_SCHEMA_VERSION {
        return Ok(true);
    }
    Ok(!rendition_exists(
        data_dir,
        &row.scene_hash,
        Rendition::Card,
    ))
}

/// The `projects/art_ready` payload. `None` when there is no row to describe.
#[must_use]
pub fn art_ready_payload(
    index: &Index,
    project_id: i64,
    rendition: Rendition,
) -> Option<serde_json::Value> {
    let row = load_row(index.conn(), project_id).ok()??;
    Some(serde_json::json!({
        "projectId": project_id,
        "sceneHash": row.scene_hash,
        "rendition": crate::art::rendition_slug(rendition),
        "artState": state_slug(row.state),
    }))
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
    use crate::art::store::{load_row, rendition_exists};
    use crate::protocol::{ArtState, Rendition};

    fn seeded() -> (tempfile::TempDir, crate::index::Index) {
        let dir = tempfile::tempdir().expect("tempdir");
        let index = crate::index::Index::open(dir.path()).expect("open");
        index
            .conn()
            .execute(
                "INSERT INTO project (id, name, seed_basename, created_at, updated_at)
                 VALUES (7, 'alpha tool', 'alpha-tool', 0, 0)",
                [],
            )
            .expect("insert");
        (dir, index)
    }

    #[test]
    fn a_first_run_writes_the_row_and_the_card_file() {
        let (dir, index) = seeded();
        let out = render_card(&index, 7, 1_700_000_000).expect("render");
        assert!(out.rendered);
        assert!(crate::art::is_scene_hash(&out.scene_hash));
        assert!(rendition_exists(
            dir.path(),
            &out.scene_hash,
            Rendition::Card
        ));
        // §7.6: art_state tracks the card only, so a project whose page has never been opened
        // is `ready`, not `pending`, despite having no hero file.
        let row = load_row(index.conn(), 7).expect("load").expect("row");
        assert_eq!(row.state, ArtState::Ready);
        assert_eq!(row.rendered_at, Some(1_700_000_000));
        assert!(!rendition_exists(
            dir.path(),
            &out.scene_hash,
            Rendition::Hero
        ));
    }

    #[test]
    fn a_second_run_with_nothing_moved_writes_nothing_and_says_so() {
        // Ruling 7 / §7.1a: "No card changes appearance while the user is watching it."
        let (dir, index) = seeded();
        let first = render_card(&index, 7, 1_000).expect("first");
        let path = crate::art::rendition_path(dir.path(), &first.scene_hash, Rendition::Card)
            .expect("path");
        let stamp = std::fs::metadata(&path)
            .expect("meta")
            .modified()
            .expect("mtime");
        let second = render_card(&index, 7, 2_000).expect("second");
        assert_eq!(second.scene_hash, first.scene_hash);
        assert!(!second.rendered, "an unchanged scene must not be redrawn");
        assert_eq!(
            std::fs::metadata(&path)
                .expect("meta")
                .modified()
                .expect("m"),
            stamp
        );
        // rendered_at is the observation time of the render that happened, so it did not move.
        assert_eq!(
            load_row(index.conn(), 7)
                .expect("l")
                .expect("r")
                .rendered_at,
            Some(1_000)
        );
    }

    #[test]
    fn a_deleted_card_file_is_redrawn_even_though_the_hash_is_unchanged() {
        let (dir, index) = seeded();
        let first = render_card(&index, 7, 1_000).expect("first");
        crate::art::store::remove_rendition(dir.path(), &first.scene_hash, Rendition::Card)
            .expect("remove");
        let second = render_card(&index, 7, 2_000).expect("second");
        assert_eq!(second.scene_hash, first.scene_hash);
        assert!(second.rendered);
        assert!(rendition_exists(
            dir.path(),
            &first.scene_hash,
            Rendition::Card
        ));
    }

    #[test]
    fn an_input_moving_produces_a_new_address_and_a_new_file() {
        let (dir, index) = seeded();
        let first = render_card(&index, 7, 1_000).expect("first");
        index
            .conn()
            .execute(
                "UPDATE project SET size_tracked_bytes = 900000000 WHERE id = 7",
                [],
            )
            .expect("grow");
        let second = render_card(&index, 7, 2_000).expect("second");
        assert_ne!(second.scene_hash, first.scene_hash);
        assert!(second.rendered);
        assert!(rendition_exists(
            dir.path(),
            &second.scene_hash,
            Rendition::Card
        ));
        // The superseded file is still on disk; the sweep is what removes it (§7.5).
        assert!(rendition_exists(
            dir.path(),
            &first.scene_hash,
            Rendition::Card
        ));
    }

    #[test]
    fn a_missing_project_is_a_hard_failure_not_a_panic() {
        let (_dir, index) = seeded();
        assert!(render_card(&index, 999, 1).is_err());
        assert!(matches!(
            run_j5(&index, 999, 1),
            Ok(JobOutcome::HardFail { .. })
        ));
    }

    #[test]
    fn a_write_that_cannot_land_asks_for_a_retry_and_moves_the_scene_counter() {
        let (dir, index) = seeded();
        // First render so there is a row for the counter to live on, then block the tree by
        // putting a regular file where the art directory has to be.
        render_card(&index, 7, 1_000).expect("first");
        index
            .conn()
            .execute(
                "UPDATE project SET size_tracked_bytes = 900000000 WHERE id = 7",
                [],
            )
            .expect("grow");
        let root = crate::art::art_root(dir.path());
        std::fs::remove_dir_all(&root).expect("clear");
        std::fs::write(&root, b"not a directory").expect("block");

        let outcome = run_j5(&index, 7, 2_000).expect("run");
        assert!(
            matches!(outcome, JobOutcome::TransientFail { .. }),
            "{outcome:?}"
        );
        let row = load_row(index.conn(), 7).expect("l").expect("r");
        assert_eq!(row.fail_count, 1);
        assert_eq!(row.state, ArtState::Failed);
        // §7.5's fallback is the CSS nameplate, which is a legitimate finished card — the row
        // still names the last hash that did render, so nothing points at a hole.
        assert!(crate::art::is_scene_hash(&row.scene_hash));
    }

    #[test]
    fn needs_art_is_true_for_every_state_except_a_ready_row_with_its_file() {
        let (dir, index) = seeded();
        assert!(needs_art(index.conn(), 7).expect("needs"), "no row at all");
        let out = render_card(&index, 7, 1).expect("render");
        assert!(!needs_art(index.conn(), 7).expect("needs"));
        crate::art::store::set_state(index.conn(), 7, ArtState::Stale).expect("stale");
        assert!(needs_art(index.conn(), 7).expect("needs"));
        crate::art::store::set_state(index.conn(), 7, ArtState::Ready).expect("ready");
        crate::art::store::remove_rendition(dir.path(), &out.scene_hash, Rendition::Card)
            .expect("remove");
        // The row alone still says "ready" — §7.5's missing file is only visible to the check
        // that looks at the disk, which is the one the visible-tile hook uses.
        assert!(!needs_art(index.conn(), 7).expect("needs"));
        assert!(
            needs_art_at(index.conn(), dir.path(), 7).expect("needs"),
            "ready but the file is gone"
        );
    }

    #[test]
    fn the_ready_event_names_the_project_the_hash_and_the_rendition() {
        let (_dir, index) = seeded();
        let out = render_card(&index, 7, 1).expect("render");
        let payload = art_ready_payload(&index, 7, Rendition::Card).expect("payload");
        assert_eq!(
            payload.get("projectId").and_then(serde_json::Value::as_i64),
            Some(7)
        );
        assert_eq!(
            payload.get("sceneHash").and_then(serde_json::Value::as_str),
            Some(out.scene_hash.as_str())
        );
        assert_eq!(
            payload.get("rendition").and_then(serde_json::Value::as_str),
            Some("card")
        );
        assert_eq!(
            payload.get("artState").and_then(serde_json::Value::as_str),
            Some("ready")
        );
    }
}
