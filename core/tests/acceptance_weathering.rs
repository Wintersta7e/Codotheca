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
