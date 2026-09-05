//! Acceptance: §7.5, §7.6 — criterion 22's art-cache half.
//!
//! `ac_22_database_and_wal_under_100mb` is criterion 22's other half and belongs to plan 09,
//! which does not yet write it. It is deliberately absent rather than stubbed: a stub would
//! report `passed` for a budget nothing measured.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use codotheca_core::art::generate::{generate, SceneInputs};
use codotheca_core::art::raster::{render, target_for, CARD_TARGET, HERO_TARGET};
use codotheca_core::art::scene::scene_hash;
use codotheca_core::art::store::{read_hero_lru, rendition_exists, touch_hero, write_rendition};
use codotheca_core::art::{rendition_path, HERO_CACHE_MAX};
use codotheca_core::protocol::Rendition;

/// §7.6's re-derived total, and criterion 22's budget.
const ART_CACHE_BUDGET_BYTES: u64 = 50 * 1024 * 1024;
/// The shelf the budget is stated against.
const PROJECTS: u64 = 1_000;
/// Enough distinct scenes to see the spread without rasterizing a shelf.
const SAMPLE: u32 = 24;
/// More opens than the cap, so the eviction order is observed and not inferred.
const OPENS: u32 = 250;

#[test]
fn ac_22_art_cache_under_50mb() {
    let dir = tempfile::tempdir().expect("tempdir");
    let data_dir = dir.path();

    // The card rendition is 600x900 (§7.6) — not the 520x780 v2.1 approximated for a GPU
    // texture phase 1 no longer allocates.
    assert_eq!((CARD_TARGET.w, CARD_TARGET.h), (600, 900));
    assert_eq!((HERO_TARGET.w, HERO_TARGET.h), (536, 804));
    assert_eq!(target_for(Rendition::Card), CARD_TARGET);
    assert_eq!(target_for(Rendition::Hero), HERO_TARGET);

    let mut worst_card = 0_u64;
    let mut worst_hero = 0_u64;
    let mut hashes = Vec::new();

    for n in 0..SAMPLE {
        let scene = generate(&SceneInputs {
            seed_basename: format!("sample-project-{n}"),
            ..SceneInputs::default()
        });
        let hash = scene_hash(&scene).expect("hash");

        let pixmap = render(&scene, CARD_TARGET).expect("card raster");
        assert_eq!(
            (pixmap.width(), pixmap.height()),
            (CARD_TARGET.w, CARD_TARGET.h)
        );

        for rendition in [Rendition::Card, Rendition::Hero] {
            let path = write_rendition(data_dir, &hash, rendition, &scene).expect("write");
            let bytes = std::fs::metadata(&path).expect("stat").len();
            match rendition {
                Rendition::Card => worst_card = worst_card.max(bytes),
                Rendition::Hero => worst_hero = worst_hero.max(bytes),
                // §7.6 budgets the two rasters the cache holds. §23.5's blueprint passes are
                // rendered on demand like the hero and take the same targets, so they add no
                // new worst case here — and the loop above does not produce one.
                Rendition::CardBlueprint | Rendition::HeroBlueprint => {
                    panic!("the loop iterates Card and Hero only")
                }
            }
        }
        hashes.push(hash);
    }

    // Measured per-rendition maxima, extrapolated to the shelf the criterion names. Stated as an
    // extrapolation because it is one: the alternative is a thousand rasters in a unit-test gate.
    let heroes = u64::try_from(HERO_CACHE_MAX).expect("the hero cap fits a u64");
    let projected = PROJECTS * worst_card + heroes * worst_hero;
    // stderr, never stdout: `print_stdout` is denied crate-wide because stdout carries protocol
    // frames. These are the first measured numbers behind §7.6's re-derived ~27 MB and ~4 MB.
    eprintln!(
        "worst card {worst_card} bytes, worst hero {worst_hero} bytes, projected {projected}"
    );
    assert!(
        projected < ART_CACHE_BUDGET_BYTES,
        "projected art cache {projected} bytes exceeds {ART_CACHE_BUDGET_BYTES} \
         (worst card {worst_card}, worst hero {worst_hero})",
    );

    // The cap and its order are exact, and cost nothing: the journal decides, not the rasterizer.
    let opened: Vec<String> = (0..OPENS).map(|n| format!("{n:064x}")).collect();
    for hash in &opened {
        let path = rendition_path(data_dir, hash, Rendition::Hero).expect("path");
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("mkdir");
        }
        std::fs::write(&path, b"placeholder").expect("write");
        touch_hero(data_dir, hash).expect("touch");
    }

    let journal = read_hero_lru(data_dir);
    assert_eq!(
        journal.len(),
        HERO_CACHE_MAX,
        "the hero cache is bounded at 200 files"
    );

    // Least-recently-opened evicted first: the survivors are the last 200 opened, in order.
    assert_eq!(journal, opened[opened.len() - HERO_CACHE_MAX..].to_vec());
    assert!(
        !rendition_exists(data_dir, &opened[0], Rendition::Hero),
        "the oldest went"
    );
    assert!(
        rendition_exists(data_dir, &opened[opened.len() - 1], Rendition::Hero),
        "the newest stayed"
    );

    // Cards are never evicted: they are what the shelf paints.
    for hash in &hashes {
        assert!(rendition_exists(data_dir, hash, Rendition::Card));
    }
}
