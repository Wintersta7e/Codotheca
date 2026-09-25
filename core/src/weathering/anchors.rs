//! §33.3's anchor table, over `Scene` and nothing else.
//!
//! This function takes a stored scene document and returns geometry. It carries no observation
//! time, no debt input and no freeze state, because the anchor set is a pure function of the
//! document — which is what lets §33.1 put the resolver *beside* the debt list rather than inside
//! `core/src/art/`, and what makes Boundary 2's static audit decidable at all.
//!
//! **The lit set is the renderer's.** Nothing here knows what a debt item is.

use crate::art::scene::{Scene, SPACE_W};
use crate::protocol::{DecayLayer, Point, Polyline, Rect, WeatherLayer};

/// The generated enum's own order, iterable.
///
/// `DecayLayer::ALL` is the declaration order, and §33.3's table is written in it — a second
/// ordering here would be a second total order for A3's tie-break to disagree with.
pub const LAYER_ORDER: [DecayLayer; 5] = DecayLayer::ALL;

const fn rect_of(r: [i32; 4]) -> Rect {
    Rect {
        x: r[0],
        y: r[1],
        w: r[2],
        h: r[3],
    }
}

const fn point_of(p: [i32; 2]) -> Point {
    Point { x: p[0], y: p[1] }
}

/// Squared distance from a module's top-right corner to the card's. Squared because the
/// comparison is an ordering and a square root would introduce a float into an integer document.
/// `i64` because the widest legal separation squared overflows nothing at that width.
fn corner_distance_sq(rect: [i32; 4]) -> i64 {
    let dx = i64::from(SPACE_W) - (i64::from(rect[0]) + i64::from(rect[2]));
    let dy = i64::from(rect[1]);
    dx * dx + dy * dy
}

/// §33.3's whole geometry contract.
///
/// Total: a zero count yields an empty vector, never a panic and **never an invented
/// coordinate** (A1b). Every `DecayLayer` variant gets an entry, including one with no anchor —
/// an absent entry and an empty one would be two spellings of the same fact. **At most one** of
/// `rects`, `points` and `paths` is non-empty (R127.1), and the non-empty one is the kind the
/// table gives that layer: on a `Plain` scene with no vents — the majority shape — `dust` and
/// `overgrowth` are correctly all-empty.
#[must_use]
pub fn resolve_anchors(scene: &Scene) -> Vec<WeatherLayer> {
    LAYER_ORDER
        .iter()
        .map(|layer| match *layer {
            DecayLayer::Dust => dust(scene),
            DecayLayer::Cobwebs => cobwebs(scene),
            DecayLayer::Rust => rust(scene),
            DecayLayer::Cracks => cracks(scene),
            DecayLayer::Overgrowth => overgrowth(scene),
        })
        .collect()
}

/// Every face-up module rect, then every vent rect; document order within each group.
///
/// **No per-module filter on the individual document.** `face_up` is a function of `module_kind`
/// alone (`core/src/art/compose.rs:242-246`) and the generator gives every module one kind, so
/// within a scene it is all of them or none. The `filter` below reads the field rather than the
/// kind because the field is what the document stores, but the branch it expresses cannot vary
/// inside one scene — and writing a per-module *rule* is how a later reader concludes it does.
fn dust(scene: &Scene) -> WeatherLayer {
    WeatherLayer {
        layer: DecayLayer::Dust,
        rects: scene
            .modules
            .iter()
            .filter(|m| m.face_up)
            .map(|m| rect_of(m.rect))
            .chain(scene.vents.iter().map(|v| rect_of(v.rect)))
            .collect(),
        points: Vec::new(),
        paths: Vec::new(),
    }
}

/// One anchor: the module whose top-right corner is nearest the card's. Ties break on the lowest
/// index.
///
/// **The rule stays geometric although it resolves to `modules[0]` on every scene the generator
/// writes today** — every module is stacked at one `x` and one width with increasing `y`
/// (`core/src/art/compose.rs:311-318`), so the nearest corner is always the topmost. Writing
/// `modules.first()` would be correct against today's livery and silently wrong against the first
/// one that places two modules side by side.
fn cobwebs(scene: &Scene) -> WeatherLayer {
    let nearest = scene
        .modules
        .iter()
        .enumerate()
        .min_by_key(|(index, m)| (corner_distance_sq(m.rect), *index))
        .map(|(_, m)| rect_of(m.rect));
    WeatherLayer {
        layer: DecayLayer::Cobwebs,
        rects: nearest.into_iter().collect(),
        points: Vec::new(),
        paths: Vec::new(),
    }
}

/// Every fastener centre, in document order.
///
/// **The point alone — no `kind`, no `rot`.** The head is drawn into the bitmap; the layer draws
/// only the stain, and a stain running `+y` under gravity does not read the head's orientation.
/// Two fields nothing would read are two fields that drift.
fn rust(scene: &Scene) -> WeatherLayer {
    WeatherLayer {
        layer: DecayLayer::Rust,
        rects: Vec::new(),
        points: scene.fasteners.iter().map(|f| point_of(f.at)).collect(),
        paths: Vec::new(),
    }
}

/// Every seam polyline, in document order. The livery seam is always drawn, so this is the one
/// layer with a guaranteed anchor.
fn cracks(scene: &Scene) -> WeatherLayer {
    WeatherLayer {
        layer: DecayLayer::Cracks,
        rects: Vec::new(),
        points: Vec::new(),
        paths: scene
            .seams
            .iter()
            .map(|s| Polyline {
                points: s.path.iter().map(|p| point_of(*p)).collect(),
            })
            .collect(),
    }
}

/// Every vent rect, ordered by **descending** `rect[1] + rect[3]` — the lowest vent first,
/// because growth climbs. Ties break on the lowest index, which `sort_by_key` gives for free
/// since it is stable.
fn overgrowth(scene: &Scene) -> WeatherLayer {
    let mut rects: Vec<Rect> = scene.vents.iter().map(|v| rect_of(v.rect)).collect();
    rects.sort_by_key(|r| core::cmp::Reverse(i64::from(r.y) + i64::from(r.h)));
    WeatherLayer {
        layer: DecayLayer::Overgrowth,
        rects,
        points: Vec::new(),
        paths: Vec::new(),
    }
}
