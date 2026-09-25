//! §2.4's two art commands. Everything they do is already implemented under `crate::art`; a
//! handler here is argument parsing, one guard, and one existing call. Nothing in this file
//! reads a clock or a path from the wire: `now` arrives on `ArtCtx`, and the only string the
//! renderer may send is a 64-hex scene hash, which names no file until `rendition_path` builds
//! one from it.

use serde_json::Value;

use crate::art::job::{art_ready_payload, render_card};
use crate::art::scene::Scene;
use crate::art::store::{
    find_project_by_hash, load_row, rendition_exists, set_state, touch_hero, write_rendition,
};
use crate::art::{art_url, is_scene_hash, ArtCtx, ArtError};
use crate::identity::redirect::resolve_project_id;
use crate::proto::dispatch::{parse_args, CommandFailure};
use crate::proto::txguard::TxGuard;
use crate::protocol::{
    ArtRerender, ArtRerenderArgs, ArtState, ArtUrlArgs, CoreError, ErrorCode, ProjectId, Rendition,
    SceneHash,
};

/// `ArtError` already carries the closed code; this is the only place it becomes a wire failure.
/// The core's `message` is diagnostic and is never shown raw (§2.4).
fn failure(err: &ArtError) -> CommandFailure {
    CommandFailure {
        code: err.code(),
        message: err.to_string(),
        outcome: None,
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
        // §23.5: **the request is the demand**, exactly as the hero's is. J5 keeps rendering the
        // `card` rendition only, and `art_state` / `fail_count` keep tracking `card` alone (§7.6)
        // — a blueprint that fails to draw must not mark the project's art failed, because the
        // card is what that state describes.
        r @ (Rendition::CardBlueprint | Rendition::HeroBlueprint) => {
            blueprint_address(ctx, hash, r)
        }
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

/// §23.5's second pass, answered the way `hero_address` answers the hero's: the request **is**
/// the demand, because the core cannot observe an open tile.
///
/// Two things it deliberately does not do. It does **not** touch the hero LRU — only the `hero`
/// rendition is journalled, and a blueprint has no entry there — and it does **not** write
/// `art_state` or `fail_count`, which track the `card` rendition only (§7.6): a blueprint that
/// fails to draw must not mark the project's art failed, because the card is what that state
/// describes. A project with no scene answers `""`, ruling 9's *no address*, and §7.5's
/// nameplate stands.
pub fn blueprint_address(
    ctx: &ArtCtx<'_>,
    hash: &str,
    rendition: Rendition,
) -> Result<String, ArtError> {
    let data_dir = ctx.index.data_dir();
    if rendition_exists(data_dir, hash, rendition) {
        return Ok(art_url(hash, rendition).unwrap_or_default());
    }
    let Some(project_id) = find_project_by_hash(ctx.index.conn(), hash)? else {
        return Ok(String::new());
    };
    let Some(row) = load_row(ctx.index.conn(), project_id)? else {
        return Ok(String::new());
    };
    let scene: Scene = serde_json::from_str(&row.scene_json)
        .map_err(|e| ArtError::Encode(format!("scene_json for project {project_id}: {e}")))?;

    write_rendition(data_dir, hash, rendition, &scene)?;
    Ok(art_url(hash, rendition).unwrap_or_default())
}

/// §7.4: "±1 per press. Never a random draw, never a wrap, floored at 0 and unbounded above."
/// The floor is the `u32` on the wire and the column's `CHECK`; this is the step.
pub const MAX_OFFSET_STEP: u32 = 1;

/// §7.4's stepper. The renderer sends the **absolute** target offset it computed, so a retried,
/// replayed or double-delivered message writes the same integer and lands on the same card.
pub fn handle_rerender(ctx: &ArtCtx<'_>, args: Value) -> Result<Value, CommandFailure> {
    let args: ArtRerenderArgs = parse_args(args)?;
    let reply = rerender(ctx, args.project_id.0, args.offset).map_err(|e| failure(&e))?;

    if reply.rejected {
        // Plan 02: the reply carries the stored value so a stale rail resyncs, and the rejection
        // itself is announced separately — one reply cannot be both a value and a failure.
        let payload = serde_json::to_value(&CoreError {
            code: ErrorCode::Protocol,
            message: "art.rerender: offset is more than one step from the stored value".to_owned(),
            project_id: Some(reply.project_id),
        })
        .map_err(|e| CommandFailure::internal(e.to_string()))?;
        ctx.events.emit("core", "error", payload);
    }
    serde_json::to_value(&reply).map_err(|e| CommandFailure::internal(e.to_string()))
}

/// One project, never a batch (§7.4). Ruling 2: the superseded renditions are left for §7.5's
/// sweep — two deleters for one file is how a cache grows a dangling journal entry.
pub fn rerender(ctx: &ArtCtx<'_>, requested: i64, offset: u32) -> Result<ArtRerender, ArtError> {
    let conn = ctx.index.conn();

    let guard = TxGuard::enter();
    let tx = conn.unchecked_transaction()?;
    // §1.6: one hop. A rail holding a pre-merge id rerolls the survivor, not nothing.
    let project_id = resolve_project_id(&tx, requested)?;
    let stored: i64 = tx.query_row(
        "SELECT reroll_offset FROM project WHERE id = ?1",
        [project_id],
        |r| r.get(0),
    )?;
    let stored = u32::try_from(stored).unwrap_or(0);

    let rejected = offset.abs_diff(stored) > MAX_OFFSET_STEP;
    if !rejected {
        tx.execute(
            "UPDATE project SET reroll_offset = ?2, updated_at = ?3 WHERE id = ?1",
            rusqlite::params![project_id, i64::from(offset), ctx.now],
        )?;
    }
    tx.commit()?;
    drop(guard);

    if rejected {
        return current(ctx, project_id, stored, true);
    }

    // §7.4 "Priority": a user is waiting on this one render, so it happens on the command's own
    // thread rather than joining the background queue behind J5's other work.
    render_card(ctx.index, project_id, ctx.now)?;

    if let Some(payload) = art_ready_payload(ctx.index, project_id, Rendition::Card) {
        ctx.events.emit("projects", "art_ready", payload);
    }
    current(ctx, project_id, offset, false)
}

/// The project's art state as it now stands, which is what both the accepted and the rejected
/// reply carry.
fn current(
    ctx: &ArtCtx<'_>,
    project_id: i64,
    offset: u32,
    rejected: bool,
) -> Result<ArtRerender, ArtError> {
    let row = load_row(ctx.index.conn(), project_id)?;
    Ok(ArtRerender {
        project_id: ProjectId(project_id),
        offset,
        rejected,
        scene_hash: row.as_ref().map(|r| SceneHash(r.scene_hash.clone())),
        art_state: row.as_ref().map_or(ArtState::Pending, |r| r.state),
    })
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
    use crate::protocol::{ArtState, Rendition, SceneHash};
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
            vec![hash]
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

    fn offset_of(index: &Index, project_id: i64) -> i64 {
        index
            .conn()
            .query_row(
                "SELECT reroll_offset FROM project WHERE id = ?1",
                [project_id],
                |r| r.get(0),
            )
            .expect("offset")
    }

    fn insert_survivor(index: &Index) -> i64 {
        const SURVIVOR_ID: i64 = 8;
        index
            .conn()
            .execute(
                "INSERT INTO project (id, name, seed_basename, created_at, updated_at)
                 VALUES (?1, 'beta tool', 'beta-tool', 0, 0)",
                [SURVIVOR_ID],
            )
            .expect("insert survivor");
        SURVIVOR_ID
    }

    #[test]
    fn a_pre_merge_id_rerolls_the_survivor_and_leaves_the_tombstone_unchanged() {
        let (_d, index, sink, _h) = seeded();
        let survivor_id = insert_survivor(&index);
        index
            .conn()
            .execute(
                "UPDATE project SET merged_into = ?2 WHERE id = ?1",
                rusqlite::params![7, survivor_id],
            )
            .expect("tombstone");
        index
            .conn()
            .execute(
                "INSERT INTO project_redirect (old_project_id, new_project_id, merged_at)
                 VALUES (?1, ?2, ?3)",
                rusqlite::params![7, survivor_id, 1_700_000_000_i64],
            )
            .expect("redirect");

        let reply = rerender(&ctx(&index, &sink), 7, 1).expect("rerender survivor");

        assert_eq!(reply.project_id.0, survivor_id);
        assert_eq!(offset_of(&index, survivor_id), 1);
        let tombstone: (i64, i64) = index
            .conn()
            .query_row(
                "SELECT reroll_offset, updated_at FROM project WHERE id = 7",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .expect("tombstone state");
        assert_eq!(tombstone, (0, 0), "the pre-merge row was not written");
    }

    #[test]
    fn a_tombstoned_id_without_a_redirect_surfaces_project_merged_at_the_wire_edge() {
        let (_d, index, sink, _h) = seeded();
        let survivor_id = insert_survivor(&index);
        index
            .conn()
            .execute(
                "UPDATE project SET merged_into = ?2 WHERE id = ?1",
                rusqlite::params![7, survivor_id],
            )
            .expect("tombstone");

        let failure = handle_rerender(&ctx(&index, &sink), json!({ "projectId": 7, "offset": 1 }))
            .expect_err("a tombstone without its redirect is stale");

        assert_eq!(failure.code, ErrorCode::ProjectMerged);
    }

    #[test]
    fn a_replayed_message_writes_the_same_integer_and_lands_on_the_same_card() {
        let (_d, index, sink, _h) = seeded();
        let first = rerender(&ctx(&index, &sink), 7, 1).expect("first");
        let replay = rerender(&ctx(&index, &sink), 7, 1).expect("replay");
        assert!(!first.rejected);
        assert!(!replay.rejected);
        assert_eq!(first.offset, 1);
        assert_eq!(first.scene_hash, replay.scene_hash);
        assert_eq!(offset_of(&index, 7), 1);
    }

    #[test]
    fn offset_n_minus_one_re_derives_byte_identically() {
        // §7.4's walk back: the derivation is pure, so offset 0 is the original card and not a
        // reconstruction of it.
        let (_d, index, sink, original) = seeded();
        rerender(&ctx(&index, &sink), 7, 1).expect("forward");
        let back = rerender(&ctx(&index, &sink), 7, 0).expect("back");
        assert_eq!(back.scene_hash, Some(SceneHash(original)));
        assert_eq!(offset_of(&index, 7), 0);
    }

    #[test]
    fn an_offset_more_than_one_step_away_is_rejected_and_the_stored_value_comes_back() {
        let (_d, index, sink, _h) = seeded();
        let reply = rerender(&ctx(&index, &sink), 7, 2).expect("reply, not an error");
        assert!(reply.rejected);
        assert_eq!(reply.offset, 0, "the stored value, so a stale rail resyncs");
        assert_eq!(offset_of(&index, 7), 0, "and nothing was written");
    }

    #[test]
    fn a_rejection_is_announced_as_protocol_on_the_core_topic() {
        let (_d, index, sink, _h) = seeded();
        let out = handle_rerender(&ctx(&index, &sink), json!({ "projectId": 7, "offset": 5 }))
            .expect("a reply, not a failure");
        assert_eq!(out["rejected"], json!(true));
        let errors = sink.named("core", "error");
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0]["code"], json!("PROTOCOL"));
        assert_eq!(errors[0]["projectId"], json!(7));
    }

    #[test]
    fn a_reroll_moves_no_identity_field_and_writes_no_ledger_row() {
        let (_d, index, sink, _h) = seeded();
        let before: (String, String) = index
            .conn()
            .query_row(
                "SELECT name, seed_basename FROM project WHERE id = 7",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .expect("before");
        rerender(&ctx(&index, &sink), 7, 1).expect("reroll");
        let after: (String, String) = index
            .conn()
            .query_row(
                "SELECT name, seed_basename FROM project WHERE id = 7",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .expect("after");
        assert_eq!(before, after);
        let xp: i64 = index
            .conn()
            .query_row("SELECT count(*) FROM xp_events", [], |r| r.get(0))
            .expect("xp");
        assert_eq!(xp, 0, "§7.4: a reroll writes no xp_events row");
    }

    #[test]
    fn a_reroll_announces_the_card_once_and_only_after_the_write() {
        let (_d, index, sink, _h) = seeded();
        rerender(&ctx(&index, &sink), 7, 1).expect("reroll");
        let ready = sink.named("projects", "art_ready");
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0]["rendition"], json!("card"));
    }
}
