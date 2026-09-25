#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! §33.3's anchor table, over the generator's own geometry.
//!
//! The fixtures are built by calling `layout()` — the function `generate()` calls — rather than
//! by writing rectangles down here, so a change to the livery cannot leave this file asserting a
//! shape the product no longer produces. The one hand-built scene exists because the generator
//! stacks every module at a single `x` (`core/src/art/compose.rs:311-318`), so cobwebs' tie-break
//! never fires against a generated document and a test over one would prove the generator rather
//! than the rule.

use codotheca_core::art::compose::{layout, FastenerKind, LayoutInputs, ModuleKind};
use codotheca_core::art::generate::{generate, SceneInputs};
use codotheca_core::art::scene::{Module, Scene, SPACE_H, SPACE_W};
use codotheca_core::protocol::{DecayLayer, WeatherLayer};
use codotheca_core::weathering::anchors::{resolve_anchors, LAYER_ORDER};

/// A production scene with its machined parts replaced by one `layout()` call. Every other field
/// is what `generate` wrote, so the document stays a real one.
fn scene_with(kind: ModuleKind, module_count: usize, vent_count: usize, livery: u8) -> Scene {
    let mut scene = generate(&SceneInputs {
        seed_basename: "fixture".to_owned(),
        ..SceneInputs::default()
    });
    let laid = layout(&LayoutInputs {
        h: scene.seed.h,
        livery_family: livery,
        panel_family: scene.panel_family,
        module_kind: kind,
        module_count,
        vent_count,
        fastener: FastenerKind::Hex,
    });
    scene.livery_family = livery;
    scene.modules = laid.modules;
    scene.vents = laid.vents;
    scene.seams = laid.seams;
    scene.fasteners = laid.fasteners;
    scene
}

/// Two modules side by side at different `x`, which the generator never produces.
fn side_by_side_scene() -> Scene {
    let mut scene = scene_with(ModuleKind::Screen, 2, 2, 0);
    scene.modules = vec![
        Module {
            id: "m0".to_owned(),
            kind: ModuleKind::Screen,
            rect: [48, 72, 228, 126],
            face_up: ModuleKind::Screen.face_up(),
        },
        Module {
            id: "m1".to_owned(),
            kind: ModuleKind::Screen,
            rect: [320, 72, 228, 126],
            face_up: ModuleKind::Screen.face_up(),
        },
    ];
    scene
}

fn layer(layers: &[WeatherLayer], which: DecayLayer) -> &WeatherLayer {
    layers
        .iter()
        .find(|l| l.layer == which)
        .unwrap_or_else(|| panic!("{which:?} has no entry"))
}

fn fixtures() -> Vec<(&'static str, Scene)> {
    vec![
        ("screen with vents", scene_with(ModuleKind::Screen, 3, 2, 0)),
        (
            "plain with no vents",
            scene_with(ModuleKind::Plain, 2, 0, 0),
        ),
        ("zero modules", scene_with(ModuleKind::Screen, 0, 2, 1)),
        (
            "zero modules and zero vents",
            scene_with(ModuleKind::Plain, 0, 0, 1),
        ),
        ("side by side", side_by_side_scene()),
        ("odd livery", scene_with(ModuleKind::Hatch, 2, 3, 3)),
    ]
}

#[test]
fn every_layer_has_an_entry_in_the_enums_declared_order() {
    let all = fixtures();
    assert!(!all.is_empty(), "scanned no fixture at all");
    for (name, scene) in &all {
        let layers = resolve_anchors(scene);
        let order: Vec<DecayLayer> = layers.iter().map(|l| l.layer).collect();
        assert_eq!(
            order,
            LAYER_ORDER.to_vec(),
            "{name}: an absent entry and an empty one would be two spellings of one fact"
        );
    }
    eprintln!("§33.3: {} scenes scanned", all.len());
}

#[test]
fn at_most_one_array_is_populated_and_it_is_the_kind_the_table_gives() {
    let all = fixtures();
    assert!(!all.is_empty(), "scanned no fixture at all");
    for (name, scene) in &all {
        for entry in resolve_anchors(scene) {
            let populated = usize::from(!entry.rects.is_empty())
                + usize::from(!entry.points.is_empty())
                + usize::from(!entry.paths.is_empty());
            assert!(
                populated <= 1,
                "{name}/{:?}: at most one of rects, points, paths is non-empty (R127.1)",
                entry.layer
            );
            // The kind is fixed by §33.3 and does not vary with the scene.
            match entry.layer {
                DecayLayer::Dust | DecayLayer::Cobwebs | DecayLayer::Overgrowth => {
                    assert!(entry.points.is_empty() && entry.paths.is_empty());
                }
                DecayLayer::Rust => {
                    assert!(entry.rects.is_empty() && entry.paths.is_empty());
                }
                DecayLayer::Cracks => {
                    assert!(entry.rects.is_empty() && entry.points.is_empty());
                }
            }
        }
    }
}

#[test]
fn dust_takes_face_up_modules_then_vents_in_document_order() {
    let scene = scene_with(ModuleKind::Screen, 3, 2, 0);
    let layers = resolve_anchors(&scene);
    let dust = layer(&layers, DecayLayer::Dust);
    let expected: Vec<[i32; 4]> = scene
        .modules
        .iter()
        .filter(|m| m.face_up)
        .map(|m| m.rect)
        .chain(scene.vents.iter().map(|v| v.rect))
        .collect();
    let got: Vec<[i32; 4]> = dust.rects.iter().map(|r| [r.x, r.y, r.w, r.h]).collect();
    assert_eq!(
        got, expected,
        "modules before vents, document order within each"
    );
    assert_eq!(got.len(), 5);
}

#[test]
fn dust_and_overgrowth_are_empty_on_a_plain_scene_with_no_vents() {
    // The majority shape: `face_up()` matches three of eight `ModuleKind` variants and `Plain` is
    // *archetype not computed*, so a card whose classifier never ran has no dust surface at all.
    let scene = scene_with(ModuleKind::Plain, 2, 0, 0);
    let layers = resolve_anchors(&scene);

    let dust = layer(&layers, DecayLayer::Dust);
    assert!(dust.rects.is_empty() && dust.points.is_empty() && dust.paths.is_empty());
    let overgrowth = layer(&layers, DecayLayer::Overgrowth);
    assert!(
        overgrowth.rects.is_empty() && overgrowth.points.is_empty() && overgrowth.paths.is_empty()
    );

    // The livery seam is always drawn (`compose.rs:398-402`), so cracks is the one layer with a
    // guaranteed anchor and the empty case above is not the resolver returning nothing at all.
    let cracks = layer(&layers, DecayLayer::Cracks);
    assert!(!cracks.paths.is_empty(), "the livery seam is always drawn");
}

#[test]
fn cobwebs_takes_the_module_nearest_the_cards_top_right_corner() {
    let scene = side_by_side_scene();
    let layers = resolve_anchors(&scene);
    let cobwebs = layer(&layers, DecayLayer::Cobwebs);
    assert_eq!(cobwebs.rects.len(), 1, "cobwebs is one anchor or none");
    // The card's top-right is (SPACE_W, 0); `m1`'s top-right is 52px from it and `m0`'s is 324.
    assert_eq!(
        [
            cobwebs.rects[0].x,
            cobwebs.rects[0].y,
            cobwebs.rects[0].w,
            cobwebs.rects[0].h
        ],
        scene.modules[1].rect,
        "the nearest corner, not index 0"
    );
    assert_eq!(SPACE_W, 600);
    assert_eq!(SPACE_H, 900);
}

#[test]
fn cobwebs_breaks_a_tie_on_the_lowest_index() {
    let mut scene = side_by_side_scene();
    scene.modules[0].rect = [320, 72, 228, 126];
    let layers = resolve_anchors(&scene);
    let cobwebs = layer(&layers, DecayLayer::Cobwebs);
    assert_eq!(cobwebs.rects.len(), 1);
    assert_eq!(cobwebs.rects[0].y, 72);
    // Both corners are identical, so the answer is the first module and nothing else.
    assert_eq!(cobwebs.rects[0].x, 320);
}

#[test]
fn cobwebs_has_no_anchor_on_a_scene_with_no_modules() {
    let scene = scene_with(ModuleKind::Screen, 0, 2, 1);
    let layers = resolve_anchors(&scene);
    let cobwebs = layer(&layers, DecayLayer::Cobwebs);
    assert!(
        cobwebs.rects.is_empty(),
        "no module is no anchor, never an invented rectangle"
    );
}

#[test]
fn rust_takes_every_fastener_centre_in_document_order() {
    let scene = scene_with(ModuleKind::Screen, 2, 2, 0);
    let layers = resolve_anchors(&scene);
    let rust = layer(&layers, DecayLayer::Rust);
    let expected: Vec<[i32; 2]> = scene.fasteners.iter().map(|f| f.at).collect();
    let got: Vec<[i32; 2]> = rust.points.iter().map(|p| [p.x, p.y]).collect();
    assert_eq!(got, expected);
    assert!(!got.is_empty(), "the fixture must carry fasteners");
}

#[test]
fn cracks_takes_every_seam_path_in_document_order() {
    let scene = scene_with(ModuleKind::Screen, 2, 2, 1);
    let layers = resolve_anchors(&scene);
    let cracks = layer(&layers, DecayLayer::Cracks);
    let expected: Vec<Vec<[i32; 2]>> = scene.seams.iter().map(|s| s.path.clone()).collect();
    let got: Vec<Vec<[i32; 2]>> = cracks
        .paths
        .iter()
        .map(|p| p.points.iter().map(|pt| [pt.x, pt.y]).collect())
        .collect();
    assert_eq!(got, expected);
    assert_eq!(got.len(), 2, "an odd greebling family adds a second seam");
}

#[test]
fn overgrowth_orders_vents_by_descending_bottom_edge() {
    let scene = scene_with(ModuleKind::Hatch, 2, 3, 3);
    let layers = resolve_anchors(&scene);
    let overgrowth = layer(&layers, DecayLayer::Overgrowth);
    assert_eq!(overgrowth.rects.len(), scene.vents.len());

    let bottoms: Vec<i32> = overgrowth.rects.iter().map(|r| r.y + r.h).collect();
    let mut sorted = bottoms.clone();
    sorted.sort_unstable_by(|a, b| b.cmp(a));
    assert_eq!(
        bottoms, sorted,
        "lowest vent first: descending rect[1]+rect[3]"
    );

    // The same set, not a different one.
    let mut got: Vec<[i32; 4]> = overgrowth
        .rects
        .iter()
        .map(|r| [r.x, r.y, r.w, r.h])
        .collect();
    let mut expected: Vec<[i32; 4]> = scene.vents.iter().map(|v| v.rect).collect();
    got.sort_unstable();
    expected.sort_unstable();
    assert_eq!(got, expected);
}

#[test]
fn overgrowth_breaks_a_tie_on_the_lowest_index() {
    let mut scene = scene_with(ModuleKind::Hatch, 1, 3, 2);
    // Three vents sharing one bottom edge: the order is the document's.
    let first = scene.vents[0].rect;
    for vent in &mut scene.vents {
        vent.rect = first;
    }
    scene.vents[0].rect[0] = 10;
    scene.vents[1].rect[0] = 20;
    scene.vents[2].rect[0] = 30;
    let layers = resolve_anchors(&scene);
    let overgrowth = layer(&layers, DecayLayer::Overgrowth);
    let xs: Vec<i32> = overgrowth.rects.iter().map(|r| r.x).collect();
    assert_eq!(xs, vec![10, 20, 30], "ties break on the lowest index");
}

#[test]
fn resolve_anchors_is_total_over_a_scene_with_nothing_laid_out() {
    let mut scene = scene_with(ModuleKind::Plain, 0, 0, 0);
    scene.seams.clear();
    scene.fasteners.clear();
    let layers = resolve_anchors(&scene);
    assert_eq!(layers.len(), 5);
    for entry in &layers {
        assert!(entry.rects.is_empty() && entry.points.is_empty() && entry.paths.is_empty());
    }
}
