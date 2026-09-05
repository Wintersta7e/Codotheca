//! AC-P2-23-11's core half: **the blueprint tile is the same machine**.
//!
//! Two claims, and both are constructions rather than coincidences. `scene_hash` does not move
//! when a project is cloned, because `load_inputs` reads `project` columns only — so the card and
//! its blueprint address one scene. And the two passes resolve to **different** addresses and
//! **different bytes**, because §7.6's address is `codotheca://art/<hash>/<rendition>` and one
//! variant cannot address two passes (R47).
//!
//! Compiled only under `testkit`: `art::testsupport::CollectingSink` is gated there.
#![cfg(feature = "testkit")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use codotheca_core::art::commands::blueprint_address;
use codotheca_core::art::generate::{generate, SceneInputs};
use codotheca_core::art::scene::scene_hash;
use codotheca_core::art::store::{put_scene, rendition_exists, write_rendition};
use codotheca_core::art::testsupport::CollectingSink;
use codotheca_core::art::{art_url, rendition_path, ArtCtx};
use codotheca_core::index::Index;
use codotheca_core::protocol::{ArtState, Rendition, SceneHash};

const PROJECT: i64 = 7;

fn seeded(basename: &str) -> (tempfile::TempDir, Index, String) {
    let dir = tempfile::tempdir().expect("tempdir");
    let index = Index::open(dir.path()).expect("open");
    index
        .conn()
        .execute(
            "INSERT INTO project (id, name, seed_basename, created_at, updated_at)
             VALUES (?1, ?2, ?2, 0, 0)",
            rusqlite::params![PROJECT, basename],
        )
        .expect("insert");
    let scene = generate(&SceneInputs {
        seed_basename: basename.to_owned(),
        ..SceneInputs::default()
    });
    let hash = scene_hash(&scene).expect("hash");
    put_scene(
        index.conn(),
        PROJECT,
        &hash,
        &scene,
        ArtState::Ready,
        Some(0),
    )
    .expect("put");
    (dir, index, hash)
}

fn add_location(index: &Index, path: &str) {
    index
        .conn()
        .execute(
            "INSERT INTO location (project_id, kind, distro, path_bytes, path_key, path_display,
                                   volume_key, store_key, presence, repo_kind)
             VALUES (?1, 'linux', '', ?2, ?2, ?3, 'v', 's', 'present', 'worktree')",
            rusqlite::params![PROJECT, path.as_bytes(), path],
        )
        .expect("insert location");
}

/// The scene the generator produces for the project **as the database currently holds it**,
/// re-read through `load_inputs` rather than reconstructed from a constant. That is the whole
/// point: the claim is about which columns the generator reads, so a helper that rebuilt the
/// inputs by hand would assert its own argument.
fn hash_now(index: &Index) -> String {
    let inputs = codotheca_core::art::generate::load_inputs(index.conn(), PROJECT)
        .expect("load")
        .expect("the project row is there");
    scene_hash(&generate(&inputs)).expect("hash")
}

/// AC-P2-23-11's first half.
#[test]
fn the_scene_hash_does_not_move_when_a_project_is_cloned() {
    let (_d, index, hash) = seeded("alpha-tool");
    let before = hash_now(&index);
    assert_eq!(before, hash);

    add_location(&index, "/w/alpha-tool");
    let after_write = hash_now(&index);
    assert_eq!(
        after_write, before,
        "writing a location must move nothing the generator reads"
    );

    index
        .conn()
        .execute("DELETE FROM location WHERE project_id = ?1", [PROJECT])
        .expect("delete location");
    assert_eq!(hash_now(&index), before);

    // Deviation 3, stated as an assertion rather than as prose: what this proves is that the
    // **row write** moves nothing. J2/J3/J4 legitimately re-derive the scene after a clone
    // completes, by filling `size_tracked_bytes`, `language_bytes` and `first_commit_at` — those
    // are `load_inputs` inputs and are supposed to move it.
    let with_inventory = generate(&SceneInputs {
        seed_basename: "alpha-tool".to_owned(),
        size_tracked_bytes: Some(2_000_000),
        ..SceneInputs::default()
    });
    assert_ne!(
        scene_hash(&with_inventory).expect("hash"),
        before,
        "an inventory is an input; only the location row is not"
    );
}

/// AC-P2-23-11's second half: different addresses, different bytes, one hash.
#[test]
fn the_card_and_its_blueprint_share_a_hash_and_share_no_file() {
    let (_d, index, hash) = seeded("alpha-tool");
    let sink = CollectingSink::default();
    let ctx = ArtCtx {
        index: &index,
        events: &sink,
        now: 1_700_000_000,
    };

    let card = art_url(&hash, Rendition::Card).expect("card address");
    let blueprint = art_url(&hash, Rendition::CardBlueprint).expect("blueprint address");
    assert_ne!(card, blueprint, "one hash, two addresses");
    assert!(card.ends_with("/card"));
    assert!(blueprint.ends_with("/card-blueprint"));

    // The request is the demand, exactly as the hero's is.
    assert!(!rendition_exists(
        index.data_dir(),
        &hash,
        Rendition::CardBlueprint
    ));
    let answered = blueprint_address(&ctx, &hash, Rendition::CardBlueprint).expect("blueprint");
    assert_eq!(answered, blueprint);
    assert!(rendition_exists(
        index.data_dir(),
        &hash,
        Rendition::CardBlueprint
    ));

    // And the two files differ. `write_rendition` draws the card here so the comparison is over
    // two files that exist rather than over one that does.
    let scene_json: String = index
        .conn()
        .query_row(
            "SELECT scene_json FROM art_scene WHERE project_id = ?1",
            [PROJECT],
            |r| r.get(0),
        )
        .expect("scene_json");
    let scene = serde_json::from_str(&scene_json).expect("scene parses");
    write_rendition(index.data_dir(), &hash, Rendition::Card, &scene).expect("card raster");

    let card_path = rendition_path(index.data_dir(), &hash, Rendition::Card).expect("card path");
    let blueprint_path =
        rendition_path(index.data_dir(), &hash, Rendition::CardBlueprint).expect("blueprint path");
    assert_ne!(
        card_path, blueprint_path,
        "two passes must not share a file"
    );
    let card_bytes = std::fs::read(&card_path).expect("read card");
    let blueprint_bytes = std::fs::read(&blueprint_path).expect("read blueprint");
    assert_ne!(
        card_bytes, blueprint_bytes,
        "a second pass that writes the first pass's bytes is not a second pass"
    );

    // The hero pair, so all four renditions of one hash are four distinct files.
    blueprint_address(&ctx, &hash, Rendition::HeroBlueprint).expect("hero blueprint");
    write_rendition(index.data_dir(), &hash, Rendition::Hero, &scene).expect("hero raster");
    let paths: std::collections::BTreeSet<_> = [
        Rendition::Card,
        Rendition::Hero,
        Rendition::CardBlueprint,
        Rendition::HeroBlueprint,
    ]
    .into_iter()
    .filter_map(|r| rendition_path(index.data_dir(), &hash, r))
    .collect();
    assert_eq!(paths.len(), 4);
    for path in &paths {
        assert!(path.is_file(), "{} was not written", path.display());
    }
}

/// §23.5 and §7.6: the blueprint is drawn on demand and **tracks no state**. `art_state` and
/// `fail_count` describe the `card` rendition alone, so drawing a blueprint must not move them.
#[test]
fn drawing_a_blueprint_moves_no_art_state_and_no_hero_journal() {
    let (_d, index, hash) = seeded("alpha-tool");
    let sink = CollectingSink::default();
    let ctx = ArtCtx {
        index: &index,
        events: &sink,
        now: 1_700_000_000,
    };
    let before: (String, i64) = index
        .conn()
        .query_row(
            "SELECT art_state, fail_count FROM project p
               JOIN art_scene a ON a.project_id = p.id WHERE p.id = ?1",
            [PROJECT],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("state");

    blueprint_address(&ctx, &hash, Rendition::CardBlueprint).expect("blueprint");
    blueprint_address(&ctx, &hash, Rendition::HeroBlueprint).expect("hero blueprint");

    let after: (String, i64) = index
        .conn()
        .query_row(
            "SELECT art_state, fail_count FROM project p
               JOIN art_scene a ON a.project_id = p.id WHERE p.id = ?1",
            [PROJECT],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("state");
    assert_eq!(before, after, "the card's state describes the card");

    // The hero LRU is the hero's journal; a blueprint has no entry in it.
    let lru = codotheca_core::art::store::read_hero_lru(index.data_dir());
    assert!(
        !lru.iter().any(|e| e == &hash),
        "a blueprint must not be journalled as a hero"
    );

    // No events either: `art_ready` announces the card and the hero, and this pass announces
    // nothing because nothing was waiting on it.
    assert!(sink.events().is_empty(), "the blueprint pass emits nothing");
}

/// §23.3's art row, **consumed** from p2-22's hydration rather than written here (§22.4 owns the
/// writer). When a project's first index is a remote listing, `seed_basename` is the remote's
/// **bare** repository name, never `owner/name` — precisely so local↔remote matching never
/// re-rolls the art.
#[test]
fn a_listing_sourced_project_seeds_on_the_bare_name() {
    use codotheca_core::identity::ingest::ingest_listing;
    use codotheca_core::identity::testutil::{forge_aliases, listing_library};
    use codotheca_core::index::migrate::{apply_all, MIGRATIONS};
    use codotheca_core::index::{open_connection, Index as IndexHandle};

    let dir = tempfile::tempdir().expect("tempdir");
    let mut conn = open_connection(&IndexHandle::db_path(dir.path())).expect("open");
    apply_all(&mut conn, MIGRATIONS).expect("migrate");

    let library = listing_library();
    let aliases = forge_aliases();
    let tx = conn.transaction().expect("tx");
    let mut ingested = 0_usize;
    for entry in &library.listings {
        ingest_listing(&tx, entry, &aliases, 1_700_000_000).expect("ingest");
        ingested += 1;
    }
    tx.commit().expect("commit");
    eprintln!("blueprint_renditions: ingested {ingested} listing entr(ies)");
    assert!(
        ingested > 0,
        "a run that ingested no listing proves nothing"
    );

    let mut stmt = conn
        .prepare("SELECT name, seed_basename FROM project")
        .expect("prepare");
    let rows: Vec<(String, String)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .expect("query")
        .map(|r| r.expect("row"))
        .collect();
    assert!(!rows.is_empty(), "the listing created no project");
    for (name, seed) in rows {
        assert!(
            !seed.contains('/'),
            "{name}: seed_basename carries a slash: {seed}"
        );
    }
}

/// R12, across two languages: `--tier-blue` and `--tier-blue-ink` have one owner (§8.7) and two
/// producers — the raster's ground and ink here, and the renderer's frame token there. A
/// cross-language mirror needs a test reading the other side (R24), and this is it.
#[test]
fn the_blueprint_ink_and_ground_are_the_tokens_the_renderer_declares() {
    use codotheca_core::art::blueprint::{TIER_BLUE, TIER_BLUE_INK};

    let css = include_str!("../../app/src/renderer/styles/tokens.css");
    let hex = |(r, g, b): (u8, u8, u8)| format!("#{r:02x}{g:02x}{b:02x}");
    // `--tier-blue-ink` joins this list in the change that declares it (§23.5's tile), so the
    // test never asserts a token that does not exist yet — and the count below is what stops it
    // from passing over an empty list.
    let mirrored: Vec<(&str, String)> = vec![("--tier-blue", hex(TIER_BLUE))];
    let mut found = 0_usize;
    for (token, value) in &mirrored {
        let needle = format!("{token}: {value}");
        assert!(
            css.contains(&needle),
            "tokens.css does not declare `{needle}`; the raster and the frame would disagree"
        );
        found += 1;
    }
    assert_eq!(
        found,
        mirrored.len(),
        "a run that compared no token proves nothing"
    );
    assert!(found > 0);
    // The ink is declared in Rust already and its own mirror assertion lands with the token.
    assert_eq!(hex(TIER_BLUE_INK), "#9fc2d6");
}

/// A `SceneHash` is what the wire carries; this pins that the address builder takes one and that
/// a non-hash never becomes a path.
#[test]
fn a_blueprint_address_is_never_built_from_a_string_that_is_not_a_hash() {
    let bad = SceneHash("../etc/passwd".to_owned());
    assert!(art_url(&bad.0, Rendition::CardBlueprint).is_none());
    assert!(rendition_path(
        std::path::Path::new("/data"),
        &bad.0,
        Rendition::HeroBlueprint
    )
    .is_none());
}
