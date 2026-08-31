//! Acceptance: §7.3a, §7.4, §7.5, §7.6 — criteria 56 and 62, core side.
//!
//! Test names are the ids `acceptance/criteria.json` carries. Renaming one silently detaches a
//! criterion from its proof.
//!
//! Compiled only under `testkit`: `art::testsupport::CollectingSink` is gated there, so without
//! the feature this fails to compile rather than skipping silently.
#![cfg(feature = "testkit")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use codotheca_core::art::commands::rerender;
use codotheca_core::art::fade::{fade_for, FADE_FLAT, FADE_NONE};
use codotheca_core::art::generate::{generate, SceneInputs};
use codotheca_core::art::scene::scene_hash;
use codotheca_core::art::store::put_scene;
use codotheca_core::art::testsupport::CollectingSink;
use codotheca_core::art::{rendition_path, ArtCtx};
use codotheca_core::index::Index;
use codotheca_core::protocol::{ArtState, Rendition, SceneHash};

fn seeded(basename: &str) -> (tempfile::TempDir, Index, String) {
    let dir = tempfile::tempdir().expect("tempdir");
    let index = Index::open(dir.path()).expect("open");
    index
        .conn()
        .execute(
            "INSERT INTO project (id, name, seed_basename, created_at, updated_at)
             VALUES (7, ?1, ?1, 0, 0)",
            [basename],
        )
        .expect("insert");
    let scene = generate(&SceneInputs {
        seed_basename: basename.to_owned(),
        ..SceneInputs::default()
    });
    let hash = scene_hash(&scene).expect("hash");
    put_scene(index.conn(), 7, &hash, &scene, ArtState::Ready, Some(0)).expect("put");
    codotheca_core::art::store::write_rendition(index.data_dir(), &hash, Rendition::Card, &scene)
        .expect("card");
    (dir, index, hash)
}

#[test]
fn ac_56_reroll_offset_is_absolute() {
    let (_d, index, original) = seeded("alpha-tool");
    let sink = CollectingSink::default();
    let ctx = ArtCtx {
        index: &index,
        events: &sink,
        now: 1_700_000_000,
    };

    // Absolute, never an increment: a replayed message writes the same integer.
    let first = rerender(&ctx, 7, 1).expect("first");
    let replay = rerender(&ctx, 7, 1).expect("replay");
    assert_eq!(first.offset, 1);
    assert_eq!(first.scene_hash, replay.scene_hash);
    assert_ne!(first.scene_hash, Some(SceneHash(original.clone())));

    // More than one step away is rejected, with the stored value returned so a rail resyncs.
    let far = rerender(&ctx, 7, 9).expect("a reply, not a failure");
    assert!(far.rejected);
    assert_eq!(far.offset, 1);

    // Floored at 0 and never a wrap: the wire type is u32 and the column CHECKs it, so the step
    // down from 0 is unrepresentable rather than defended against.
    let back = rerender(&ctx, 7, 0).expect("back");
    assert!(!back.rejected);
    // Offset n-1 re-derives byte-identically: offset 0 IS the original card.
    assert_eq!(back.scene_hash, Some(SceneHash(original)));

    // One projectId per call — no batch exists to test, so the absence is asserted structurally:
    // nothing else in the project table moved.
    let touched: i64 = index
        .conn()
        .query_row(
            "SELECT count(*) FROM project WHERE reroll_offset != 0",
            [],
            |r| r.get(0),
        )
        .expect("count");
    assert_eq!(touched, 0);

    // §7.4: no ledger row, ever.
    let xp: i64 = index
        .conn()
        .query_row("SELECT count(*) FROM xp_events", [], |r| r.get(0))
        .expect("xp");
    assert_eq!(xp, 0);
}

#[test]
fn ac_62_fade_is_reference_or_archived_only() {
    // Exactly two values in phase 1, regardless of last_commit_at — which is not an input at all.
    // Compared by epsilon, as `core::art::fade`'s own tests do: `==` on f64 is a denied lint.
    assert!((fade_for(false, false) - FADE_NONE).abs() < f64::EPSILON);
    assert!((fade_for(true, false) - FADE_FLAT).abs() < f64::EPSILON);
    assert!((fade_for(false, true) - FADE_FLAT).abs() < f64::EPSILON);
    assert!((fade_for(true, true) - FADE_FLAT).abs() < f64::EPSILON);
    assert!((FADE_FLAT - 0.25).abs() < f64::EPSILON);
    assert!((FADE_NONE - 0.0).abs() < f64::EPSILON);

    // And no other value reaches the document.
    for (is_ref, is_arc) in [(false, false), (true, false), (false, true), (true, true)] {
        let scene = generate(&SceneInputs {
            seed_basename: "alpha-tool".to_owned(),
            is_reference: is_ref,
            is_archived: is_arc,
            ..SceneInputs::default()
        });
        let json = serde_json::to_value(&scene).expect("scene json");
        let fade = json["fade"].as_f64().expect("fade is a number");
        assert!(
            (fade - FADE_NONE).abs() < f64::EPSILON || (fade - FADE_FLAT).abs() < f64::EPSILON,
            "fade was {fade}"
        );
    }
}

#[test]
fn ac_62_scene_hash_is_not_a_function_of_wall_clock() {
    let (_d, index, hash) = seeded("alpha-tool");
    let sink = CollectingSink::default();

    // Fixtures whose last_commit_at spans years, and a clock advanced by any amount.
    for (now, last_commit) in [
        (0_i64, 0_i64),
        (1_700_000_000, 1_000_000_000),
        (2_000_000_000, 1),
    ] {
        index
            .conn()
            .execute(
                "UPDATE project SET last_commit_at = ?1 WHERE id = 7",
                [last_commit],
            )
            .expect("update");
        let ctx = ArtCtx {
            index: &index,
            events: &sink,
            now,
        };
        let outcome = codotheca_core::art::job::render_card(ctx.index, 7, ctx.now).expect("render");
        assert_eq!(outcome.scene_hash, hash, "the hash moved with the clock");
        // §7.1a / plan 10 ruling 7: nothing moved, so nothing was re-rendered and nothing emitted.
        assert!(!outcome.rendered);
    }
    assert!(sink.named("projects", "art_ready").is_empty());
    assert!(rendition_path(index.data_dir(), &hash, Rendition::Card)
        .expect("path")
        .exists());
}

#[test]
fn ac_62_seed_inputs_are_basename_and_offset() {
    // The seed is seed_basename composed with reroll_offset and nothing else. first_commit_sha
    // is not an input, so its empty-repository "" case has nothing to test.
    let base = SceneInputs {
        seed_basename: "alpha-tool".to_owned(),
        ..SceneInputs::default()
    };

    // A first commit arriving on a previously-empty repository changes era detail only: hue,
    // composition and layout inputs are untouched (§7.2, §7.4).
    let with_first = SceneInputs {
        first_commit_at: Some(1_000_000_000),
        ..base.clone()
    };
    let a = serde_json::to_value(generate(&base)).expect("a");
    let b = serde_json::to_value(generate(&with_first)).expect("b");
    assert_eq!(a["palette"], b["palette"], "hue is not an era function");
    assert_eq!(
        a["modules"], b["modules"],
        "composition is not an era function"
    );
    assert_eq!(a["seed"], b["seed"], "the seed does not see a commit");
    assert_ne!(
        a["fasteners"], b["fasteners"],
        "era detail is exactly what moves"
    );

    // The offset is a suffix on the hashed string, never a rewrite of the stored basename.
    let rolled = SceneInputs {
        reroll_offset: 1,
        ..base.clone()
    };
    assert_ne!(
        scene_hash(&generate(&base)).expect("h0"),
        scene_hash(&generate(&rolled)).expect("h1")
    );
    let back = SceneInputs {
        reroll_offset: 0,
        ..base.clone()
    };
    assert_eq!(
        scene_hash(&generate(&base)).expect("h0"),
        scene_hash(&generate(&back)).expect("h0'")
    );

    // Ruling 4: the on-disk grammar the shell mirrors, pinned against the same literal
    // app/test/artProtocol.test.ts asserts.
    let h = "0123456789abcdef".repeat(4);
    let card = rendition_path(std::path::Path::new("/data"), &h, Rendition::Card).expect("card");
    assert_eq!(
        card,
        std::path::Path::new("/data")
            .join("art")
            .join("01")
            .join(format!("{h}.card.webp"))
    );
}
