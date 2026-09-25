//! §23.5's second render pass: the line-art engineering drawing of **the same seeded machine**.
//!
//! Same seed, same scene document, different pass. It is *a different kind of tile, not a
//! degraded one* — so nothing here is a filter over `render`'s output, and `fade` is untouched
//! (§7.3a's two-valued expression stands; line art is a render mode, not drained chroma).
//!
//! What it paints is the geometry the scene already carries, as **strokes** rather than fills:
//! the plate boundary, the module outlines, the vent slats, the fastener rings, the seam paths,
//! and the drafting title block's rules and frame — in `--tier-blue-ink` over the `--tier-blue`
//! ground.
//!
//! **The title block carries no text, and that is not a deferral.** §7.1: the bitmap carries no
//! text at all — titles, identity lines and completion ticks are DOM. §23.5 defers *the title
//! block's content and type, the line weights of the second pass, and the assembly animation* to
//! §24 with the design handoff; what lands here is the block's **geometry** in the art layer,
//! crossing no band, and the line work of the machine itself.

use tiny_skia::{Color, FillRule, Paint, PathBuilder, Pixmap, Rect, Stroke, Transform};

use crate::art::cast::{f32_to_u8, i32_to_f32, u32_to_f32};
use crate::art::raster::{RenderTarget, FASTENER_RADIUS};
use crate::art::scene::{Scene, VentDir, SPACE_H, SPACE_W};
use crate::art::ArtError;
use crate::protocol::Rendition;

/// §8.7's `--tier-blue`, the blueprint ground.
///
/// Stated here because the raster needs the value and the renderer's frame token needs it too;
/// `core/tests/blueprint_renditions.rs` reads `app/src/renderer/styles/tokens.css` and asserts
/// the two agree, so the mirror has a test reading the other side rather than two independent
/// copies (R12, R24).
pub const TIER_BLUE: (u8, u8, u8) = (0x2f, 0x4a, 0x5c);
/// §8.7's `--tier-blue-ink`, the readable variant and the only ink this pass draws in.
pub const TIER_BLUE_INK: (u8, u8, u8) = (0x9f, 0xc2, 0xd6);

/// Total over the four variants, so a fifth rendition is a compile error rather than a silent
/// `false`.
#[must_use]
pub const fn is_blueprint(rendition: Rendition) -> bool {
    match rendition {
        Rendition::Card | Rendition::Hero => false,
        Rendition::CardBlueprint | Rendition::HeroBlueprint => true,
    }
}

fn solid_tier_blue() -> Color {
    Color::from_rgba8(TIER_BLUE.0, TIER_BLUE.1, TIER_BLUE.2, 255)
}

fn ink(alpha: f32) -> Paint<'static> {
    let mut paint = Paint::default();
    paint.set_color(Color::from_rgba8(
        TIER_BLUE_INK.0,
        TIER_BLUE_INK.1,
        TIER_BLUE_INK.2,
        alpha_byte(alpha),
    ));
    paint.anti_alias = true;
    paint
}

fn alpha_byte(alpha: f32) -> u8 {
    f32_to_u8((alpha * 255.0).round().clamp(0.0, 255.0))
}

fn hairline(width: f32) -> Stroke {
    Stroke {
        width,
        ..Stroke::default()
    }
}

fn rect_of(r: [i32; 4]) -> Option<Rect> {
    Rect::from_xywh(
        i32_to_f32(r[0]),
        i32_to_f32(r[1]),
        i32_to_f32((r[2]).max(1)),
        i32_to_f32((r[3]).max(1)),
    )
}

fn stroke_rect(pixmap: &mut Pixmap, rect: Rect, paint: &Paint<'_>, stroke: &Stroke, t: Transform) {
    let mut builder = PathBuilder::new();
    builder.push_rect(rect);
    if let Some(path) = builder.finish() {
        pixmap.stroke_path(&path, paint, stroke, t, None);
    }
}

/// The plate boundary, inset so the stroke sits inside the raster rather than half outside it.
fn paint_plate_boundary(pixmap: &mut Pixmap, t: Transform) {
    let outer = Rect::from_xywh(6.0, 6.0, i32_to_f32(SPACE_W - 12), i32_to_f32(SPACE_H - 12));
    if let Some(rect) = outer {
        stroke_rect(pixmap, rect, &ink(0.55), &hairline(2.0), t);
    }
    let inner = Rect::from_xywh(
        16.0,
        16.0,
        i32_to_f32(SPACE_W - 32),
        i32_to_f32(SPACE_H - 32),
    );
    if let Some(rect) = inner {
        stroke_rect(pixmap, rect, &ink(0.28), &hairline(1.0), t);
    }
}

fn paint_module_outlines(pixmap: &mut Pixmap, scene: &Scene, t: Transform) {
    for module in &scene.modules {
        let Some(rect) = rect_of(module.rect) else {
            continue;
        };
        stroke_rect(pixmap, rect, &ink(0.85), &hairline(2.0), t);
        // A drafting hatch on the face the module presents, so the two orientations the scene
        // records are still distinguishable in line art.
        let inset = if module.face_up { 6.0 } else { 10.0 };
        if let Some(inner) = Rect::from_xywh(
            rect.x() + inset,
            rect.y() + inset,
            (rect.width() - inset * 2.0).max(1.0),
            (rect.height() - inset * 2.0).max(1.0),
        ) {
            stroke_rect(pixmap, inner, &ink(0.35), &hairline(1.0), t);
        }
    }
}

fn paint_vent_slats(pixmap: &mut Pixmap, scene: &Scene, t: Transform) {
    const PITCH: f32 = 9.0;
    for vent in &scene.vents {
        let Some(rect) = rect_of(vent.rect) else {
            continue;
        };
        let mut builder = PathBuilder::new();
        // The slats step across tiny-skia's `f32` rect edges. An integer step count would match
        // this loop only while every edge is an exact `f32` integer, and a scene document read
        // back from the index carries no such bound, so the float walk stays as drawn.
        #[allow(clippy::while_float)]
        match vent.dir {
            VentDir::H => {
                let mut y = rect.y() + PITCH;
                while y < rect.bottom() {
                    builder.move_to(rect.x(), y);
                    builder.line_to(rect.right(), y);
                    y += PITCH;
                }
            }
            VentDir::V => {
                let mut x = rect.x() + PITCH;
                while x < rect.right() {
                    builder.move_to(x, rect.y());
                    builder.line_to(x, rect.bottom());
                    x += PITCH;
                }
            }
        }
        if let Some(path) = builder.finish() {
            pixmap.stroke_path(&path, &ink(0.5), &hairline(1.0), t, None);
        }
        stroke_rect(pixmap, rect, &ink(0.6), &hairline(1.5), t);
    }
}

fn paint_fastener_rings(pixmap: &mut Pixmap, scene: &Scene, t: Transform) {
    for fastener in &scene.fasteners {
        let (cx, cy) = (i32_to_f32(fastener.at[0]), i32_to_f32(fastener.at[1]));
        if let Some(ring) = PathBuilder::from_circle(cx, cy, FASTENER_RADIUS) {
            pixmap.stroke_path(&ring, &ink(0.8), &hairline(1.5), t, None);
        }
        // A centre mark, the drafting convention for a hole.
        let mut cross = PathBuilder::new();
        cross.move_to(cx - FASTENER_RADIUS - 2.0, cy);
        cross.line_to(cx + FASTENER_RADIUS + 2.0, cy);
        cross.move_to(cx, cy - FASTENER_RADIUS - 2.0);
        cross.line_to(cx, cy + FASTENER_RADIUS + 2.0);
        if let Some(path) = cross.finish() {
            pixmap.stroke_path(&path, &ink(0.4), &hairline(1.0), t, None);
        }
    }
}

fn paint_seam_lines(pixmap: &mut Pixmap, scene: &Scene, t: Transform) {
    for seam in &scene.seams {
        let mut points = seam.path.iter();
        let Some(&[first_x, first_y]) = points.next() else {
            continue;
        };
        let mut builder = PathBuilder::new();
        builder.move_to(i32_to_f32(first_x), i32_to_f32(first_y));
        let mut any = false;
        for &[px, py] in points {
            builder.line_to(i32_to_f32(px), i32_to_f32(py));
            any = true;
        }
        if !any {
            continue;
        }
        if let Some(path) = builder.finish() {
            pixmap.stroke_path(&path, &ink(0.45), &hairline(1.0), t, None);
        }
    }
}

/// The drafting title block: **rules and a frame, and no text at all** (§7.1). Its content, type
/// and line weights are §24's with the design handoff; the geometry is this task's.
///
/// Lower-right, inside the inner plate rule, in the drafting convention.
fn paint_title_block(pixmap: &mut Pixmap, transform: Transform) {
    const W: f32 = 236.0;
    const H: f32 = 84.0;
    let left = i32_to_f32(SPACE_W - 16) - W;
    let top = i32_to_f32(SPACE_H - 16) - H;
    let Some(frame) = Rect::from_xywh(left, top, W, H) else {
        return;
    };
    // A solid ground behind the block, so the machine's line work does not read through it.
    let mut ground = Paint::default();
    ground.set_color(solid_tier_blue());
    let mut filled = PathBuilder::new();
    filled.push_rect(frame);
    if let Some(path) = filled.finish() {
        pixmap.fill_path(&path, &ground, FillRule::Winding, transform, None);
    }
    stroke_rect(pixmap, frame, &ink(0.9), &hairline(2.0), transform);

    let mut rules = PathBuilder::new();
    for share in [0.34_f32, 0.67] {
        let rule_y = H.mul_add(share, top);
        rules.move_to(left, rule_y);
        rules.line_to(left + W, rule_y);
    }
    // One vertical division, so the block reads as a title block rather than as a ruled box.
    rules.move_to(W.mul_add(0.62, left), H.mul_add(0.34, top));
    rules.line_to(W.mul_add(0.62, left), top + H);
    if let Some(path) = rules.finish() {
        pixmap.stroke_path(&path, &ink(0.7), &hairline(1.0), transform, None);
    }
}

/// The second pass. **Pure and deterministic**: it reads the scene and nothing else — no clock,
/// no filesystem, no random source — so two renders of one scene are byte-identical.
///
/// # Errors
///
/// [`ArtError::Encode`] when either side of `target` is zero, or when tiny-skia cannot allocate
/// a pixmap that size.
pub fn render_blueprint(scene: &Scene, target: RenderTarget) -> Result<Pixmap, ArtError> {
    if target.w == 0 || target.h == 0 {
        return Err(ArtError::Encode(format!(
            "degenerate target {}x{}",
            target.w, target.h
        )));
    }
    let mut pixmap = Pixmap::new(target.w, target.h)
        .ok_or_else(|| ArtError::Encode(format!("pixmap {}x{}", target.w, target.h)))?;
    pixmap.fill(solid_tier_blue());

    let transform = Transform::from_scale(
        u32_to_f32(target.w) / i32_to_f32(scene.space.w.max(1)),
        u32_to_f32(target.h) / i32_to_f32(scene.space.h.max(1)),
    );
    paint_plate_boundary(&mut pixmap, transform);
    paint_module_outlines(&mut pixmap, scene, transform);
    paint_vent_slats(&mut pixmap, scene, transform);
    paint_seam_lines(&mut pixmap, scene, transform);
    paint_fastener_rings(&mut pixmap, scene, transform);
    paint_title_block(&mut pixmap, transform);
    Ok(pixmap)
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
    use crate::art::generate::{generate, SceneInputs};
    use crate::art::raster::{render, CARD_TARGET, HERO_TARGET};

    fn scene() -> Scene {
        generate(&SceneInputs {
            seed_basename: "alpha-tool".to_owned(),
            archetype: Some("library".to_owned()),
            primary_language: Some("Rust".to_owned()),
            size_tracked_bytes: Some(2_000_000),
            first_commit_at: Some(1_600_000_000),
            first_commit_tz_offset_min: Some(0),
            ..SceneInputs::default()
        })
    }

    #[test]
    fn is_blueprint_is_total_over_the_four_variants() {
        assert!(!is_blueprint(Rendition::Card));
        assert!(!is_blueprint(Rendition::Hero));
        assert!(is_blueprint(Rendition::CardBlueprint));
        assert!(is_blueprint(Rendition::HeroBlueprint));
    }

    /// The shape `the_generator_is_pure` establishes, applied to the second pass.
    #[test]
    fn the_pass_is_pure_and_deterministic() {
        let s = scene();
        let a = render_blueprint(&s, CARD_TARGET).expect("first");
        let b = render_blueprint(&s, CARD_TARGET).expect("second");
        assert_eq!(a.data(), b.data(), "two renders of one scene must agree");
    }

    #[test]
    fn the_two_passes_draw_different_pixels_at_the_same_target() {
        let s = scene();
        let card = render(&s, CARD_TARGET).expect("card");
        let blueprint = render_blueprint(&s, CARD_TARGET).expect("blueprint");
        assert_eq!(
            (card.width(), card.height()),
            (blueprint.width(), blueprint.height())
        );
        assert_ne!(
            card.data(),
            blueprint.data(),
            "a second pass that draws the first pass is not a second pass"
        );
    }

    #[test]
    fn the_pass_changes_the_ink_and_never_the_geometry() {
        let s = scene();
        let card = render_blueprint(&s, CARD_TARGET).expect("card");
        let hero = render_blueprint(&s, HERO_TARGET).expect("hero");
        assert_eq!(
            (card.width(), card.height()),
            (CARD_TARGET.w, CARD_TARGET.h)
        );
        assert_eq!(
            (hero.width(), hero.height()),
            (HERO_TARGET.w, HERO_TARGET.h)
        );
    }

    #[test]
    fn a_degenerate_target_is_refused_rather_than_drawn() {
        let s = scene();
        assert!(render_blueprint(&s, RenderTarget { w: 0, h: 10 }).is_err());
        assert!(render_blueprint(&s, RenderTarget { w: 10, h: 0 }).is_err());
    }
}
