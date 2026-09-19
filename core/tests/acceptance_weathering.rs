#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! The `AC-P3-33-*` criteria that live in the core.
//!
//! `AC-P3-33-3` (the anchor table), `AC-P3-33-9` (Boundary 2) and `AC-P3-33-12` (`fade` is pinned
//! for ever). The rest are the renderer's and carry their tags in their own files.

use codotheca_core::art::fade::{fade_for, FADE_FLAT, FADE_NONE};
use codotheca_core::art::generate::{generate, SceneInputs};
use codotheca_core::art::scene::{canonical_json, scene_hash};

/// One year in seconds, near enough for a fixture that only has to span years.
const YEAR: i64 = 365 * 24 * 60 * 60;

fn inputs(first_commit_at: Option<i64>, is_reference: bool, is_archived: bool) -> SceneInputs {
    SceneInputs {
        seed_basename: "pinned".to_owned(),
        reroll_offset: 0,
        is_reference,
        is_archived,
        archetype: Some("site".to_owned()),
        primary_language: Some("Rust".to_owned()),
        language_bytes_json: None,
        size_tracked_bytes: Some(4_000_000),
        first_commit_at,
        first_commit_tz_offset_min: Some(0),
    }
}

/// **`AC-P3-33-12`.** `fade` stays `(is_reference || is_archived) ? 0.25 : 0` **permanently**.
///
/// §7.3a:222's phase-3 row is deleted rather than amended, and §7.3a's argument is promoted from
/// a phase gate to an invariant: a time-driven `fade` is staleness-driven decay, and it makes a
/// content address a function of wall-clock time — which does not weaken with the phase number.
/// Under §33.1 it would also be **the last route by which decay could re-address a card**.
#[test]
fn ac_p3_33_12_fade_is_pinned_for_ever_over_fixtures_spanning_years() {
    let mut scanned = 0_usize;
    // The commit clock spans a decade. `fade` does not move with it, at any flag combination.
    for years_ago in 0..10 {
        let at = Some(1_700_000_000 - i64::from(years_ago) * YEAR);
        for (is_reference, is_archived) in
            [(false, false), (true, false), (false, true), (true, true)]
        {
            let scene = generate(&inputs(at, is_reference, is_archived));
            let expected = if is_reference || is_archived {
                FADE_FLAT
            } else {
                FADE_NONE
            };
            assert!(
                (scene.fade - expected).abs() < f64::EPSILON,
                "fade moved with the commit clock: {years_ago} years, {is_reference}/{is_archived}"
            );
            scanned += 1;
        }
    }
    assert!(scanned > 0, "scanned no fixture at all");
    eprintln!("AC-P3-33-12: {scanned} fixtures scanned across a decade of commit clocks");
}

/// The address does not move because time passed. `generate` takes no clock, so two calls at any
/// two instants produce byte-identical documents — and that is what makes it safe to say
/// `scene_hash` is content-addressed rather than time-addressed.
#[test]
fn ac_p3_33_12_the_scene_hash_is_byte_identical_however_much_time_passes() {
    let first = generate(&inputs(Some(1_400_000_000), false, false));
    let later = generate(&inputs(Some(1_400_000_000), false, false));
    assert_eq!(
        canonical_json(&first).unwrap(),
        canonical_json(&later).unwrap()
    );
    assert_eq!(scene_hash(&first).unwrap(), scene_hash(&later).unwrap());
}

/// **The signature is the gate.** `fade_for` takes two booleans and nothing else: no timestamp,
/// no `Clock`. A phase-3 author who wanted a clock would have to change this line, which is what
/// makes the pin one line rather than a fork in the renderer.
#[test]
fn ac_p3_33_12_fade_for_takes_no_clock() {
    let signature: fn(bool, bool) -> f64 = fade_for;
    assert!((signature(false, false) - FADE_NONE).abs() < f64::EPSILON);
    assert!((signature(true, true) - FADE_FLAT).abs() < f64::EPSILON);
}

/// **Both producers, read as source.** §7.1a requires the two to agree, and the clause that told
/// the next author to change them is what expires here.
///
/// The scan reads the raw text **including comments**, deliberately and unlike
/// `AC-P3-33-9`'s module audit: the clock this criterion retires lived in a doc comment, and a
/// comment instructing a future author to build it is exactly what §27.6 supersedes.
#[test]
fn ac_p3_33_12_no_producer_of_fade_names_a_commit_clock() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let producers = [
        root.join("src/art/fade.rs"),
        root.join("../app/src/renderer/art/appearance.ts"),
    ];
    // The clamp §7.3a:222 proposed, spelled the way it proposed it.
    let forbidden = ["days_since_last_commit", "/ 900"];
    let mut scanned = 0_usize;
    for path in &producers {
        let text = std::fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("{} is readable: {e}", path.display()));
        assert!(!text.is_empty(), "{} read as empty", path.display());
        scanned += 1;
        for needle in forbidden {
            assert!(
                !text.contains(needle),
                "{} still names {needle}: the phase-3 fade clock is retired, not scheduled",
                path.display()
            );
        }
    }
    assert!(
        scanned >= 2,
        "both producers must be read, scanned {scanned}"
    );
    eprintln!("AC-P3-33-12: {scanned} fade producers scanned");
}

// ---------------------------------------------------------------------------------------------
// AC-P3-33-9 — Boundary 2 (§27.4, §33.6). **The audit is a byte-identity criterion, not a code
// review**, plus a static half that compares shapes rather than strings.
// ---------------------------------------------------------------------------------------------

/// Every `DebtSource`, so the fixture carries an open item on all five layers at once.
const ALL_SOURCES: [&str; 8] = [
    "todo_marker",
    "missing_readme",
    "missing_license",
    "missing_tests",
    "no_release",
    "unpushed_commits",
    "ci_red",
    "dependency_advisory",
];

/// **`AC-P3-33-9`, the byte-identity half.**
///
/// Over a fixture where **every open debt item is closed**, `art_scene.scene_hash` and the bytes
/// of **both** rendition files are identical before and after, and no re-render is enqueued.
/// That is criterion 62's existing shape pointed at a different input, and it is the one check
/// that stops a later author *"simplifying"* the layers into the scene document.
#[test]
fn ac_p3_33_9_the_bitmap_is_byte_identical_lit_or_clean() {
    use codotheca_core::art::job::needs_art_at;
    use codotheca_core::art::store::{put_scene, write_rendition};
    use codotheca_core::art::{rendition_path, scene::scene_hash as hash_of};
    use codotheca_core::index::Index;
    use codotheca_core::protocol::{ArtState, Rendition};

    let dir = tempfile::tempdir().unwrap();
    let index = Index::open(dir.path()).unwrap();
    index
        .conn()
        .execute(
            "INSERT INTO project (id, name, seed_basename, created_at, updated_at)
             VALUES (7, 'weathered', 'weathered', 0, 0)",
            [],
        )
        .unwrap();

    let scene = generate(&inputs(Some(1_500_000_000), false, false));
    let hash = hash_of(&scene).unwrap();
    put_scene(index.conn(), 7, &hash, &scene, ArtState::Ready, Some(0)).unwrap();
    write_rendition(index.data_dir(), &hash, Rendition::Card, &scene).unwrap();
    write_rendition(index.data_dir(), &hash, Rendition::Hero, &scene).unwrap();

    // Open debt on every source, which is every layer.
    for (n, source) in ALL_SOURCES.iter().enumerate() {
        index
            .conn()
            .execute(
                "INSERT INTO debt_item
                   (project_id, subject_key, source, fingerprint, state, scoring,
                    first_seen_at, last_seen_at)
                 VALUES (7, ?1, ?2, ?1, 'open', 'scored', 1, 2)",
                rusqlite::params![format!("subject-{n}"), source],
            )
            .unwrap();
    }

    let read = |r: Rendition| -> Vec<u8> {
        let path = rendition_path(index.data_dir(), &hash, r).expect("a path for a real hash");
        // Bytes, never a size: do not pipe `stat` through anything — an 860 KB file has read as
        // "86 bytes" in this repository before.
        std::fs::read(&path).unwrap_or_else(|e| panic!("{} is readable: {e}", path.display()))
    };
    let stored_hash = |conn: &rusqlite::Connection| -> String {
        conn.query_row(
            "SELECT scene_hash FROM art_scene WHERE project_id = 7",
            [],
            |r| r.get(0),
        )
        .unwrap()
    };

    let lit_open: i64 = index
        .conn()
        .query_row(
            "SELECT count(*) FROM debt_item WHERE state = 'open'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(lit_open, 8, "the fixture must actually carry open debt");

    let before_hash = stored_hash(index.conn());
    let before_card = read(Rendition::Card);
    let before_hero = read(Rendition::Hero);
    assert!(!before_card.is_empty() && !before_hero.is_empty());
    assert!(!needs_art_at(index.conn(), index.data_dir(), 7).unwrap());

    // Close every open item. *Closed* is an event, never a state: the row is deleted.
    index
        .conn()
        .execute("DELETE FROM debt_item WHERE project_id = 7", [])
        .unwrap();
    let after_open: i64 = index
        .conn()
        .query_row(
            "SELECT count(*) FROM debt_item WHERE state = 'open'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(after_open, 0, "the fixture must actually have been cleaned");

    assert_eq!(
        stored_hash(index.conn()),
        before_hash,
        "scene_hash moved when the debt list did: decay reached the bitmap"
    );
    assert_eq!(
        read(Rendition::Card),
        before_card,
        "the card rendition moved"
    );
    assert_eq!(
        read(Rendition::Hero),
        before_hero,
        "the hero rendition moved"
    );
    assert!(
        !needs_art_at(index.conn(), index.data_dir(), 7).unwrap(),
        "closing debt enqueued a re-render"
    );
    eprintln!("AC-P3-33-9: 2 rendition files compared byte for byte across 8 closed items");
}

/// **`AC-P3-33-9`, the static half.** `core/src/art/` contains no reference to `DecayLayer`, to
/// `debt`, or to any weathering symbol.
///
/// **Comments and doc comments are stripped before the scan, and this matters here
/// specifically.** `core/src/art/scene.rs` names dust, rust and cracks in prose **on purpose** —
/// it documents the three geometry joins — and calls `ground` *the weathering tint's ground*. A
/// text grep fails a correct tree, which is the recorded *grepping a declaration matches prose
/// about it* trap that has already produced two wrong rulings. What is asserted is the absence of
/// an **identifier**.
#[test]
fn ac_p3_33_9_the_art_module_knows_nothing_about_decay() {
    let art = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/art");
    let (scanned, offenders) = scan_for_decay_identifiers(&art);
    eprintln!("AC-P3-33-9: {scanned} files scanned under core/src/art/");
    // The same floor `core/tests/git_readonly.rs` uses, for the same reason: a scan that reads
    // almost nothing is a gate reporting on a tree it did not see.
    assert!(
        scanned >= 10,
        "the art module scan read {scanned} files, which is not the module"
    );
    assert!(
        offenders.is_empty(),
        "core/src/art/ names a decay symbol, so Boundary 2 has been crossed: {offenders:?}"
    );
}

/// Returns the number of `.rs` files read and every `(file, identifier)` that breaches
/// Boundary 2. Separate from the test so the floor can be exercised against an empty directory.
fn scan_for_decay_identifiers(root: &std::path::Path) -> (usize, Vec<String>) {
    let mut scanned = 0_usize;
    let mut offenders = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let Ok(text) = std::fs::read_to_string(&path) else {
                    continue;
                };
                scanned += 1;
                for ident in identifiers(&strip_rust_comments(&text)) {
                    let lower = ident.to_lowercase();
                    if lower.contains("debt")
                        || lower.contains("decaylayer")
                        || lower.contains("weathering")
                        || lower.contains("weatherlayer")
                        || ident == "resolve_anchors"
                    {
                        offenders.push(format!("{}: {ident}", path.display()));
                    }
                }
            }
        }
    }
    (scanned, offenders)
}

/// Line and block comments, removed. Doc comments are line comments and go with them.
fn strip_rust_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let bytes: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == '/' && i + 1 < bytes.len() && bytes[i + 1] == '/' {
            while i < bytes.len() && bytes[i] != '\n' {
                i += 1;
            }
        } else if bytes[i] == '/' && i + 1 < bytes.len() && bytes[i + 1] == '*' {
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == '*' && bytes[i + 1] == '/') {
                i += 1;
            }
            i = (i + 2).min(bytes.len());
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    out
}

fn identifiers(code: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    for ch in code.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            current.push(ch);
        } else if !current.is_empty() {
            out.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

/// The floor is not decoration: a scan that reads nothing reports `0 offenders` and looks clean.
#[test]
fn ac_p3_33_9_the_scan_floor_refuses_an_empty_directory() {
    let empty = tempfile::tempdir().unwrap();
    let (scanned, offenders) = scan_for_decay_identifiers(empty.path());
    assert_eq!(scanned, 0);
    assert!(
        offenders.is_empty(),
        "an empty scan finds nothing, which is the problem"
    );
    assert!(
        scanned < 10,
        "the floor the sibling test asserts would have passed here"
    );
}

// ---------------------------------------------------------------------------------------------
// AC-P3-33-3 — the sidecar's anchors, resolved by the core. `core/tests/weathering_anchors.rs`
// holds §33.3's full table; this is the criterion's own statement of the well-formedness rule.
// ---------------------------------------------------------------------------------------------

/// **`AC-P3-33-3`.** **At most one** of `rects`, `points` and `paths` is non-empty per layer, and
/// the non-empty one is the kind §33.3's table gives that layer (R127.1).
///
/// *Exactly one* is false on the majority shape — a `Plain` scene with no vents, where `dust` and
/// `overgrowth` resolve to zero anchors — and **the fix that presents itself, building the
/// fixture out of `Screen` modules, retires the majority case from the bar.** So both shapes are
/// here, and the `Plain` one is asserted to be all-empty on those two layers rather than skipped.
#[test]
fn ac_p3_33_3_at_most_one_anchor_array_is_populated_per_layer() {
    use codotheca_core::art::compose::{layout, FastenerKind, LayoutInputs, ModuleKind};
    use codotheca_core::protocol::DecayLayer;
    use codotheca_core::weathering::anchors::resolve_anchors;

    let build = |kind: ModuleKind, modules: usize, vents: usize| {
        let mut scene = generate(&inputs(Some(1_500_000_000), false, false));
        let laid = layout(&LayoutInputs {
            h: scene.seed.h,
            livery_family: scene.livery_family,
            panel_family: scene.panel_family,
            module_kind: kind,
            module_count: modules,
            vent_count: vents,
            fastener: FastenerKind::Hex,
        });
        scene.modules = laid.modules;
        scene.vents = laid.vents;
        scene.seams = laid.seams;
        scene.fasteners = laid.fasteners;
        scene
    };

    let fixtures = [
        ("screen with vents", build(ModuleKind::Screen, 2, 2)),
        ("plain with no vents", build(ModuleKind::Plain, 2, 0)),
        ("nothing laid out", build(ModuleKind::Plain, 0, 0)),
    ];
    let mut scanned = 0_usize;
    for (name, scene) in &fixtures {
        let layers = resolve_anchors(scene);
        assert_eq!(layers.len(), 5, "{name}: every variant gets an entry");
        for entry in &layers {
            let populated = usize::from(!entry.rects.is_empty())
                + usize::from(!entry.points.is_empty())
                + usize::from(!entry.paths.is_empty());
            assert!(
                populated <= 1,
                "{name}/{:?}: at most one array is non-empty",
                entry.layer
            );
            match entry.layer {
                DecayLayer::Rust => assert!(entry.rects.is_empty() && entry.paths.is_empty()),
                DecayLayer::Cracks => assert!(entry.rects.is_empty() && entry.points.is_empty()),
                _ => assert!(entry.points.is_empty() && entry.paths.is_empty()),
            }
        }
        scanned += 1;
    }

    // The majority shape, asserted rather than avoided.
    let plain = resolve_anchors(&build(ModuleKind::Plain, 2, 0));
    for want in [DecayLayer::Dust, DecayLayer::Overgrowth] {
        let entry = plain
            .iter()
            .find(|l| l.layer == want)
            .unwrap_or_else(|| panic!("{want:?} has no entry"));
        assert!(
            entry.rects.is_empty() && entry.points.is_empty() && entry.paths.is_empty(),
            "{want:?} must be all-empty on a Plain scene with no vents"
        );
    }

    assert!(scanned > 0, "scanned no scene at all");
    eprintln!("AC-P3-33-3: {scanned} scenes scanned for anchor well-formedness");
}
