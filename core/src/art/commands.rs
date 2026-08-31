//! §2.4's two art commands. Everything they do is already implemented under `crate::art`; a
//! handler here is argument parsing, one guard, and one existing call. Nothing in this file
//! reads a clock or a path from the wire: `now` arrives on `ArtCtx`, and the only string the
//! renderer may send is a 64-hex scene hash, which names no file until `rendition_path` builds
//! one from it.

use serde_json::Value;

use crate::art::job::art_ready_payload;
use crate::art::scene::Scene;
use crate::art::store::{
    find_project_by_hash, load_row, rendition_exists, set_state, touch_hero, write_rendition,
};
use crate::art::{art_url, is_scene_hash, ArtCtx, ArtError};
use crate::proto::dispatch::{parse_args, CommandFailure};
use crate::protocol::{ArtState, ArtUrlArgs, ErrorCode, Rendition};

/// `ArtError` already carries the closed code; this is the only place it becomes a wire failure.
/// The core's `message` is diagnostic and is never shown raw (§2.4).
fn failure(err: &ArtError) -> CommandFailure {
    match err.code() {
        ErrorCode::Protocol => CommandFailure::protocol(err.to_string()),
        _ => CommandFailure::internal(err.to_string()),
    }
}

/// §7.6: the address of one rendition of one scene. Answers `""` when there is no address
/// (ruling 9) and `PROTOCOL` when the hash is not a hash (ruling 1).
pub fn handle_url(ctx: &ArtCtx<'_>, args: Value) -> Result<Value, CommandFailure> {
    let args: ArtUrlArgs = parse_args(args)?;
    let hash = args.hash.0.as_str();
    if !is_scene_hash(hash) {
        return Err(failure(&ArtError::BadHash(hash.to_owned())));
    }
    let address = match args.rendition {
        Rendition::Card => card_address(ctx, hash),
        Rendition::Hero => hero_address(ctx, hash),
    }
    .map_err(|e| failure(&e))?;
    Ok(Value::String(address))
}

/// The card is drawn during the scan, at J3 (§7.6), so by the time anything asks for its address
/// the file is either there or has been swept from under a live row. The second case is a hole:
/// mark it `stale` and let J5 redraw, and answer `""` so §7.5's nameplate stands in meanwhile.
pub fn card_address(ctx: &ArtCtx<'_>, hash: &str) -> Result<String, ArtError> {
    if rendition_exists(ctx.index.data_dir(), hash, Rendition::Card) {
        // `is_scene_hash` has already passed, so `art_url` is `Some`; `unwrap_or_default` is the
        // same answer ruling 9 gives for absence and keeps `unwrap` out of the crate.
        return Ok(art_url(hash, Rendition::Card).unwrap_or_default());
    }
    if let Some(project_id) = find_project_by_hash(ctx.index.conn(), hash)? {
        set_state(ctx.index.conn(), project_id, ArtState::Stale)?;
    }
    Ok(String::new())
}

/// Plan 10 ruling 8: the hero renders lazily "on first demand" (§7.2) and the core cannot observe
/// an open page, so this request is the demand. `touch_hero` owns the journal and the eviction —
/// it returns the hashes it evicted and has already removed their files.
pub fn hero_address(ctx: &ArtCtx<'_>, hash: &str) -> Result<String, ArtError> {
    let data_dir = ctx.index.data_dir();
    if rendition_exists(data_dir, hash, Rendition::Hero) {
        touch_hero(data_dir, hash)?;
        return Ok(art_url(hash, Rendition::Hero).unwrap_or_default());
    }

    let Some(project_id) = find_project_by_hash(ctx.index.conn(), hash)? else {
        return Ok(String::new());
    };
    let Some(row) = load_row(ctx.index.conn(), project_id)? else {
        return Ok(String::new());
    };
    let scene: Scene = serde_json::from_str(&row.scene_json)
        .map_err(|e| ArtError::Encode(format!("scene_json for project {project_id}: {e}")))?;

    write_rendition(data_dir, hash, Rendition::Hero, &scene)?;
    touch_hero(data_dir, hash)?;

    // Ruling 12: no transaction is open here, so the emit is already outside one.
    if let Some(payload) = art_ready_payload(ctx.index, project_id, Rendition::Hero) {
        ctx.events.emit("projects", "art_ready", payload);
    }
    Ok(art_url(hash, Rendition::Hero).unwrap_or_default())
}

pub fn handle_rerender(
    _ctx: &ArtCtx<'_>,
    _args: serde_json::Value,
) -> Result<serde_json::Value, CommandFailure> {
    Err(CommandFailure::internal("art.rerender not yet implemented"))
}

#[cfg(all(test, feature = "testkit"))]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
mod tests {
    use super::*;
    use crate::art::generate::{generate, SceneInputs};
    use crate::art::scene::scene_hash;
    use crate::art::store::{put_scene, rendition_exists, write_rendition};
    use crate::art::testsupport::CollectingSink;
    use crate::index::Index;
    use crate::protocol::{ArtState, Rendition};
    use serde_json::json;

    /// A project with a persisted scene and a card on disk, exactly as J5 would leave it.
    fn seeded() -> (tempfile::TempDir, Index, CollectingSink, String) {
        let dir = tempfile::tempdir().expect("tempdir");
        let index = Index::open(dir.path()).expect("open");
        index
            .conn()
            .execute(
                "INSERT INTO project (id, name, seed_basename, created_at, updated_at)
                 VALUES (7, 'alpha tool', 'alpha-tool', 0, 0)",
                [],
            )
            .expect("insert");
        let inputs = SceneInputs {
            seed_basename: "alpha-tool".to_owned(),
            ..SceneInputs::default()
        };
        let scene = generate(&inputs);
        let hash = scene_hash(&scene).expect("hash");
        put_scene(index.conn(), 7, &hash, &scene, ArtState::Ready, Some(0)).expect("put");
        write_rendition(index.data_dir(), &hash, Rendition::Card, &scene).expect("card");
        (dir, index, CollectingSink::default(), hash)
    }

    fn ctx<'a>(index: &'a Index, sink: &'a CollectingSink) -> ArtCtx<'a> {
        ArtCtx {
            index,
            events: sink,
            now: 1_700_000_000,
        }
    }

    #[test]
    fn a_card_that_exists_answers_its_two_segment_address() {
        let (_d, index, sink, hash) = seeded();
        let out = handle_url(
            &ctx(&index, &sink),
            json!({ "hash": hash, "rendition": "card" }),
        )
        .expect("ok");
        assert_eq!(out, json!(format!("codotheca://art/{hash}/card")));
    }

    #[test]
    fn a_hash_that_is_not_a_hash_is_a_protocol_failure_not_an_empty_string() {
        // Ruling 1: absence is "", but a malformed hash is a renderer defect and must show.
        let (_d, index, sink, _h) = seeded();
        let err = handle_url(
            &ctx(&index, &sink),
            json!({ "hash": "../etc", "rendition": "card" }),
        )
        .expect_err("protocol");
        assert_eq!(err.code, ErrorCode::Protocol);
    }

    #[test]
    fn a_missing_card_answers_empty_and_marks_the_row_stale_so_j5_redraws_it() {
        // §7.5: art.url returning a path to a deleted file is detected and demoted to the
        // nameplate rather than rendering a hole.
        let (_d, index, sink, hash) = seeded();
        let path =
            crate::art::rendition_path(index.data_dir(), &hash, Rendition::Card).expect("path");
        std::fs::remove_file(&path).expect("remove");

        let out = handle_url(
            &ctx(&index, &sink),
            json!({ "hash": hash, "rendition": "card" }),
        )
        .expect("ok");
        assert_eq!(out, json!(""));

        let state: String = index
            .conn()
            .query_row(
                "SELECT state FROM art_scene WHERE project_id = 7",
                [],
                |r| r.get(0),
            )
            .expect("state");
        assert_eq!(state, "stale");
    }

    #[test]
    fn asking_for_the_hero_is_the_demand_that_renders_it() {
        // Plan 10 ruling 8: the core cannot see an open page, so the request IS the open.
        let (_d, index, sink, hash) = seeded();
        assert!(!rendition_exists(index.data_dir(), &hash, Rendition::Hero));

        let out = handle_url(
            &ctx(&index, &sink),
            json!({ "hash": hash, "rendition": "hero" }),
        )
        .expect("ok");

        assert_eq!(out, json!(format!("codotheca://art/{hash}/hero")));
        assert!(rendition_exists(index.data_dir(), &hash, Rendition::Hero));
        assert_eq!(
            crate::art::store::read_hero_lru(index.data_dir()),
            vec![hash.clone()]
        );
        assert_eq!(sink.named("projects", "art_ready").len(), 1);
    }

    #[test]
    fn a_second_open_of_the_same_hero_touches_the_journal_and_renders_nothing() {
        let (_d, index, sink, hash) = seeded();
        handle_url(
            &ctx(&index, &sink),
            json!({ "hash": hash, "rendition": "hero" }),
        )
        .expect("ok");
        sink.clear();
        handle_url(
            &ctx(&index, &sink),
            json!({ "hash": hash, "rendition": "hero" }),
        )
        .expect("ok");
        // §7.1a: nothing changes appearance, so nothing is announced.
        assert!(sink.named("projects", "art_ready").is_empty());
        assert_eq!(
            crate::art::store::read_hero_lru(index.data_dir()),
            vec![hash]
        );
    }

    #[test]
    fn a_hash_no_project_owns_answers_empty_rather_than_erroring() {
        let (_d, index, sink, _h) = seeded();
        let orphan = "f".repeat(64);
        for rendition in ["card", "hero"] {
            let out = handle_url(
                &ctx(&index, &sink),
                json!({ "hash": orphan, "rendition": rendition }),
            )
            .expect("ok");
            assert_eq!(out, json!(""));
        }
    }
}
