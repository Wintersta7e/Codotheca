//! The rasterizer. It walks the scene document and resolves **nothing** from a name (§7.3):
//! every value it varies on is a field it was handed.
//!
//! The bitmap carries **no text** (§7.1) — titles, identity lines and completion ticks are DOM —
//! so there is no font stack here, no grapheme segmentation and no deboss. It also carries no
//! band furniture: §7.7's language plate, hairline, rank band, chip column, band-4 vent bank,
//! jewel stripe, hazard tape and scrim are DOM layers composited over this image (ruling 1).
//!
//! What it does paint is the CSS plate, exactly: the two-tone gradient, the greebling family,
//! the static sheen, and then the livery's machined parts. §7.1a requires the two producers to
//! agree — "same hash, same hue bin, same two-tone break. One design, two producers."
//!
//! §7.3a's plate is written as two stacked CSS gradients. The upper one is fully opaque, so in
//! **both** producers the lower one is never composited; its dark stop reaches the document as
//! `palette.ground`, which is what phase-3 weathering reads. Painting it here would be work no
//! pixel shows.

use tiny_skia::{
    Color, FillRule, GradientStop, LinearGradient, Paint, PathBuilder, Pixmap, Point,
    RadialGradient, Rect, SpreadMode, Transform,
};

use crate::art::compose::{FastenerKind, ModuleKind};
use crate::art::derive::{plate_stop, PlateStop};
use crate::art::oklch::{to_srgb8, Oklch};
use crate::art::scene::{Scene, VentDir, SPACE_H, SPACE_W};
use crate::art::ArtError;
use crate::protocol::Rendition;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenderTarget {
    pub w: u32,
    pub h: u32,
}

/// §7.6: the §7.3 scene space at its declared scale.
pub const CARD_TARGET: RenderTarget = RenderTarget { w: 600, h: 900 };
/// §7.6: the design's fixed 268 px hero rail at 2×, at the scene space's own 2:3.
pub const HERO_TARGET: RenderTarget = RenderTarget { w: 536, h: 804 };

#[must_use]
pub fn target_for(rendition: Rendition) -> RenderTarget {
    match rendition {
        Rendition::Card => CARD_TARGET,
        Rendition::Hero => HERO_TARGET,
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: f32,
}

impl Rgba {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    fn color(self) -> Color {
        Color::from_rgba8(
            self.r,
            self.g,
            self.b,
            (self.a * 255.0).round().clamp(0.0, 255.0) as u8,
        )
    }

    const fn black(a: f32) -> Self {
        Self {
            r: 0,
            g: 0,
            b: 0,
            a,
        }
    }

    const fn white(a: f32) -> Self {
        Self {
            r: 255,
            g: 255,
            b: 255,
            a,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StripeDir {
    /// Horizontal bands — CSS `repeating-linear-gradient(0deg, …)`.
    H,
    /// Vertical bands — CSS `90deg`.
    V,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stripe {
    pub dir: StripeDir,
    pub thickness: i32,
    pub period: i32,
    pub color: Rgba,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Highlight {
    pub cx_percent: i32,
    pub cy_percent: i32,
    pub extent_percent: i32,
    pub color: Rgba,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Greebling {
    pub stripes: Vec<Stripe>,
    pub highlight: Option<Highlight>,
}

/// §7.3a's four greebling families, with every CSS length doubled because the scene space is
/// `px@2x`. `TOKENS.md` labels this set "chosen by `hash % 4`"; that is the **livery** selector.
/// The greebling selector is `(h >>> 3) % 4`, and the prototype is authoritative.
#[must_use]
pub fn greebling(family: u8) -> Greebling {
    match family % 4 {
        1 => Greebling {
            stripes: vec![
                Stripe {
                    dir: StripeDir::V,
                    thickness: 4,
                    period: 76,
                    color: Rgba::black(0.15),
                },
                Stripe {
                    dir: StripeDir::H,
                    thickness: 2,
                    period: 56,
                    color: Rgba::white(0.035),
                },
            ],
            highlight: None,
        },
        2 => Greebling {
            stripes: vec![Stripe {
                dir: StripeDir::H,
                thickness: 2,
                period: 30,
                color: Rgba::black(0.12),
            }],
            highlight: Some(Highlight {
                cx_percent: 76,
                cy_percent: 20,
                extent_percent: 40,
                color: Rgba::white(0.055),
            }),
        },
        3 => Greebling {
            stripes: vec![
                Stripe {
                    dir: StripeDir::V,
                    thickness: 2,
                    period: 48,
                    color: Rgba::black(0.13),
                },
                Stripe {
                    dir: StripeDir::H,
                    thickness: 4,
                    period: 84,
                    color: Rgba::black(0.09),
                },
            ],
            highlight: None,
        },
        _ => Greebling {
            stripes: vec![
                Stripe {
                    dir: StripeDir::H,
                    thickness: 2,
                    period: 40,
                    color: Rgba::black(0.13),
                },
                Stripe {
                    dir: StripeDir::V,
                    thickness: 2,
                    period: 64,
                    color: Rgba::black(0.10),
                },
            ],
            highlight: None,
        },
    }
}

/// The CSS gradient line for `linear-gradient(<angle>deg, …)` over a `w × h` box, in a y-down
/// coordinate system. `0deg` runs bottom to top; the line is long enough that the perpendicular
/// through either corner meets it, which is what makes an oblique gradient cover the box.
#[must_use]
pub fn css_gradient_line(angle_deg: f64, w: f32, h: f32) -> (Point, Point) {
    let radians = angle_deg.to_radians();
    #[allow(clippy::cast_possible_truncation)]
    let (dx, dy) = (radians.sin() as f32, -(radians.cos() as f32));
    #[allow(clippy::cast_possible_truncation)]
    let length = (f64::from(w) * radians.sin().abs() + f64::from(h) * radians.cos().abs()) as f32;
    let (cx, cy) = (w / 2.0, h / 2.0);
    (
        Point::from_xy(cx - dx * length / 2.0, cy - dy * length / 2.0),
        Point::from_xy(cx + dx * length / 2.0, cy + dy * length / 2.0),
    )
}

fn solid(color: Color) -> Paint<'static> {
    let mut paint = Paint::default();
    paint.set_color(color);
    paint.anti_alias = true;
    paint
}

fn oklch_color(color: Oklch) -> Color {
    let rgb = to_srgb8(color);
    Color::from_rgba8(rgb.r, rgb.g, rgb.b, 255)
}

fn scene_rect() -> Option<Rect> {
    #[allow(clippy::cast_precision_loss)]
    Rect::from_xywh(0.0, 0.0, SPACE_W as f32, SPACE_H as f32)
}

/// The hard two-tone split. Both bands are flat: `hi 0 <split>%` then `midTinted <split>% 100%`.
pub fn paint_ground(pixmap: &mut Pixmap, scene: &Scene, transform: Transform) {
    let Some(rect) = scene_rect() else { return };
    let hi = oklch_color(plate_stop(scene.plate, PlateStop::Hi));
    let mid = oklch_color(plate_stop(scene.plate, PlateStop::MidTinted));
    // A ground fill first, so a degenerate gradient can never leave a transparent card.
    pixmap.fill(hi);
    #[allow(clippy::cast_precision_loss)]
    let (start, end) =
        css_gradient_line(f64::from(scene.plate.ang), SPACE_W as f32, SPACE_H as f32);
    #[allow(clippy::cast_precision_loss)]
    let split = (scene.plate.split.clamp(0, 100) as f32) / 100.0;
    // Two stops at (almost) the same offset is how a hard stop is expressed.
    let stops = vec![
        GradientStop::new(0.0, hi),
        GradientStop::new(split, hi),
        GradientStop::new((split + 0.0001).min(1.0), mid),
        GradientStop::new(1.0, mid),
    ];
    let Some(shader) =
        LinearGradient::new(start, end, stops, SpreadMode::Pad, Transform::identity())
    else {
        return;
    };
    let paint = Paint {
        shader,
        ..Paint::default()
    };
    pixmap.fill_rect(rect, &paint, transform, None);
}

/// The panel greebling: two repeating stripe sets, and for family 2 a radial highlight.
pub fn paint_greebling(pixmap: &mut Pixmap, family: u8, transform: Transform) {
    let set = greebling(family);
    for stripe in &set.stripes {
        if stripe.period <= 0 || stripe.thickness <= 0 {
            continue;
        }
        let paint = solid(stripe.color.color());
        let mut at = 0_i32;
        let limit = match stripe.dir {
            StripeDir::H => SPACE_H,
            StripeDir::V => SPACE_W,
        };
        while at < limit {
            #[allow(clippy::cast_precision_loss)]
            let rect = match stripe.dir {
                StripeDir::H => {
                    Rect::from_xywh(0.0, at as f32, SPACE_W as f32, stripe.thickness as f32)
                }
                StripeDir::V => {
                    Rect::from_xywh(at as f32, 0.0, stripe.thickness as f32, SPACE_H as f32)
                }
            };
            if let Some(rect) = rect {
                pixmap.fill_rect(rect, &paint, transform, None);
            }
            at += stripe.period;
        }
    }
    let (Some(highlight), Some(rect)) = (set.highlight, scene_rect()) else {
        return;
    };
    #[allow(clippy::cast_precision_loss)]
    let centre = Point::from_xy(
        SPACE_W as f32 * (highlight.cx_percent as f32 / 100.0),
        SPACE_H as f32 * (highlight.cy_percent as f32 / 100.0),
    );
    // CSS sizes a circle radial gradient to the farthest corner by default; the `40%` stop is
    // that fraction of the ray.
    #[allow(clippy::cast_precision_loss)]
    let farthest = {
        let dx = (centre.x).max(SPACE_W as f32 - centre.x);
        let dy = (centre.y).max(SPACE_H as f32 - centre.y);
        (dx * dx + dy * dy).sqrt()
    };
    #[allow(clippy::cast_precision_loss)]
    let radius = farthest * (highlight.extent_percent as f32 / 100.0);
    let inner = highlight.color;
    let mut outer = highlight.color;
    outer.a = 0.0;
    let Some(shader) = RadialGradient::new(
        centre,
        0.0,
        centre,
        radius.max(1.0),
        vec![
            GradientStop::new(0.0, inner.color()),
            GradientStop::new(1.0, outer.color()),
        ],
        SpreadMode::Pad,
        Transform::identity(),
    ) else {
        return;
    };
    let paint = Paint {
        shader,
        ..Paint::default()
    };
    pixmap.fill_rect(rect, &paint, transform, None);
}

/// The static sheen: `linear-gradient(157deg, rgb(255 255 255 / .09), transparent 44%)`.
pub fn paint_sheen(pixmap: &mut Pixmap, transform: Transform, w: f32, h: f32) {
    let Some(rect) = scene_rect() else { return };
    let (start, end) = css_gradient_line(157.0, w, h);
    let Some(shader) = LinearGradient::new(
        start,
        end,
        vec![
            GradientStop::new(0.0, Rgba::white(0.09).color()),
            GradientStop::new(0.44, Rgba::white(0.0).color()),
            GradientStop::new(1.0, Rgba::white(0.0).color()),
        ],
        SpreadMode::Pad,
        Transform::identity(),
    ) else {
        return;
    };
    let paint = Paint {
        shader,
        ..Paint::default()
    };
    pixmap.fill_rect(rect, &paint, transform, None);
}

pub const VENT_SLAT_PITCH: i32 = 18;
pub const VENT_SLAT_THICKNESS: i32 = 6;
pub const FASTENER_RADIUS: f32 = 7.0;
pub const MODULE_EDGE_WIDTH: i32 = 4;

#[allow(clippy::cast_precision_loss)]
fn rect_of(rect: [i32; 4]) -> Option<Rect> {
    let [x, y, w, h] = rect;
    if w <= 0 || h <= 0 {
        return None;
    }
    Rect::from_xywh(x as f32, y as f32, w as f32, h as f32)
}

fn fill(pixmap: &mut Pixmap, rect: Option<Rect>, color: Rgba, transform: Transform) {
    let Some(rect) = rect else { return };
    pixmap.fill_rect(rect, &solid(color.color()), transform, None);
}

/// One machined module per `modules` entry: a raised face, a dark rebate, the project's jewel
/// on the leading edge, and a per-kind detail.
pub fn paint_modules(pixmap: &mut Pixmap, scene: &Scene, transform: Transform) {
    let jewel = to_srgb8(crate::art::derive::jewel_oklch(scene.jewel));
    let edge = Rgba {
        r: jewel.r,
        g: jewel.g,
        b: jewel.b,
        a: 0.85,
    };
    for module in &scene.modules {
        let [x, y, w, h] = module.rect;
        if w <= 0 || h <= 0 {
            continue;
        }
        fill(pixmap, rect_of([x, y, w, h]), Rgba::black(0.30), transform);
        fill(
            pixmap,
            rect_of([x + 2, y + 2, w - 4, h - 4]),
            Rgba::white(0.055),
            transform,
        );
        fill(
            pixmap,
            rect_of([x, y, MODULE_EDGE_WIDTH, h]),
            edge,
            transform,
        );
        paint_module_detail(pixmap, module.kind, module.rect, transform);
    }
}

/// The face a module's archetype gives it. Split out of [`paint_modules`] so the shared
/// rebate-and-edge treatment stays readable beside the eight per-kind cases.
fn paint_module_detail(
    pixmap: &mut Pixmap,
    kind: ModuleKind,
    rect: [i32; 4],
    transform: Transform,
) {
    let [x, y, w, h] = rect;
    match kind {
        ModuleKind::Screen => {
            fill(
                pixmap,
                rect_of([x + 20, y + 18, w - 40, h - 36]),
                Rgba::black(0.34),
                transform,
            );
            fill(
                pixmap,
                rect_of([x + 20, y + 18, w - 40, 3]),
                Rgba::white(0.10),
                transform,
            );
        }
        ModuleKind::Grille => {
            let mut at = y + 14;
            while at < y + h - 10 {
                fill(
                    pixmap,
                    rect_of([x + 18, at, w - 36, 4]),
                    Rgba::black(0.28),
                    transform,
                );
                at += 14;
            }
        }
        ModuleKind::Port => {
            for index in 0..3_i32 {
                let px = x + 24 + index * 34;
                fill(
                    pixmap,
                    rect_of([px, y + h / 2 - 12, 22, 24]),
                    Rgba::black(0.36),
                    transform,
                );
                fill(
                    pixmap,
                    rect_of([px + 3, y + h / 2 - 9, 16, 4]),
                    Rgba::white(0.09),
                    transform,
                );
            }
        }
        ModuleKind::Drum => {
            fill(
                pixmap,
                rect_of([x + 26, y + 20, w - 52, h - 40]),
                Rgba::black(0.24),
                transform,
            );
            fill(
                pixmap,
                rect_of([x + 44, y + 34, w - 88, h - 68]),
                Rgba::white(0.06),
                transform,
            );
        }
        ModuleKind::Hatch => {
            fill(
                pixmap,
                rect_of([x + 16, y + h / 2 - 2, w - 32, 4]),
                Rgba::black(0.30),
                transform,
            );
            fill(
                pixmap,
                rect_of([x + w / 2 - 2, y + 12, 4, h - 24]),
                Rgba::black(0.30),
                transform,
            );
        }
        ModuleKind::Slab => {
            fill(
                pixmap,
                rect_of([x + 14, y + 14, w - 28, 6]),
                Rgba::white(0.07),
                transform,
            );
        }
        // `blank` is a deliberate empty face; `plain` is the uncomputed archetype and takes
        // a face that no archetype produces — a bare rebate with no detail at all.
        ModuleKind::Blank => {
            fill(
                pixmap,
                rect_of([x + 10, y + 10, w - 20, h - 20]),
                Rgba::black(0.10),
                transform,
            );
        }
        ModuleKind::Plain => {}
    }
}

/// A vent bank is slats inside its rect, in the direction the document records.
pub fn paint_vents(pixmap: &mut Pixmap, scene: &Scene, transform: Transform) {
    for vent in &scene.vents {
        let [x, y, w, h] = vent.rect;
        if w <= 0 || h <= 0 {
            continue;
        }
        fill(pixmap, rect_of([x, y, w, h]), Rgba::black(0.18), transform);
        let (limit, mut at) = match vent.dir {
            VentDir::H => (h, 0),
            VentDir::V => (w, 0),
        };
        while at < limit {
            let slat = match vent.dir {
                VentDir::H => [x, y + at, w, VENT_SLAT_THICKNESS],
                VentDir::V => [x + at, y, VENT_SLAT_THICKNESS, h],
            };
            let lip = match vent.dir {
                VentDir::H => [x, y + at + VENT_SLAT_THICKNESS, w, 2],
                VentDir::V => [x + at + VENT_SLAT_THICKNESS, y, 2, h],
            };
            fill(pixmap, rect_of(slat), Rgba::black(0.32), transform);
            fill(pixmap, rect_of(lip), Rgba::white(0.06), transform);
            at += VENT_SLAT_PITCH;
        }
    }
}

/// A seam is a dark score with a bright lip beside it — where phase-3 cracks propagate.
pub fn paint_seams(pixmap: &mut Pixmap, scene: &Scene, transform: Transform) {
    for seam in &scene.seams {
        let mut points = seam.path.iter();
        let Some(&[first_x, first_y]) = points.next() else {
            continue;
        };
        let mut builder = PathBuilder::new();
        #[allow(clippy::cast_precision_loss)]
        builder.move_to(first_x as f32, first_y as f32);
        let mut any = false;
        for &[px, py] in points {
            #[allow(clippy::cast_precision_loss)]
            builder.line_to(px as f32, py as f32);
            any = true;
        }
        if !any {
            continue;
        }
        let Some(path) = builder.finish() else {
            continue;
        };
        let stroke = tiny_skia::Stroke {
            width: 2.0,
            ..tiny_skia::Stroke::default()
        };
        pixmap.stroke_path(
            &path,
            &solid(Rgba::black(0.42).color()),
            &stroke,
            transform,
            None,
        );
        let lit = transform.pre_translate(1.0, 1.0);
        pixmap.stroke_path(&path, &solid(Rgba::white(0.05).color()), &stroke, lit, None);
    }
}

/// A fastener is a recessed head with a per-era cut, rotated by the angle the document records
/// — and it is where phase-3 rust streaks originate.
pub fn paint_fasteners(pixmap: &mut Pixmap, scene: &Scene, transform: Transform) {
    for fastener in &scene.fasteners {
        let [at_x, at_y] = fastener.at;
        #[allow(clippy::cast_precision_loss)]
        let (cx, cy) = (at_x as f32, at_y as f32);
        let Some(head) = PathBuilder::from_circle(cx, cy, FASTENER_RADIUS) else {
            continue;
        };
        pixmap.fill_path(
            &head,
            &solid(Rgba::black(0.40).color()),
            FillRule::Winding,
            transform,
            None,
        );
        if let Some(rim) = PathBuilder::from_circle(cx, cy - 1.0, FASTENER_RADIUS - 1.0) {
            pixmap.fill_path(
                &rim,
                &solid(Rgba::white(0.07).color()),
                FillRule::Winding,
                transform,
                None,
            );
        }
        let bars: &[(f32, f32, f32, f32)] = match fastener.kind {
            FastenerKind::Slotted => &[(-5.0, -1.0, 10.0, 2.0)],
            FastenerKind::Hex => &[(-5.0, -1.0, 10.0, 2.0), (-1.0, -5.0, 2.0, 10.0)],
            FastenerKind::Torx => &[
                (-5.0, -1.0, 10.0, 2.0),
                (-1.0, -5.0, 2.0, 10.0),
                (-4.0, -4.0, 8.0, 2.0),
            ],
            // Era not computed: a plain head with no cut at all, which is a mark no era emits.
            FastenerKind::Plain => &[],
        };
        #[allow(clippy::cast_precision_loss)]
        let spin = transform.pre_concat(Transform::from_rotate_at(fastener.rot as f32, cx, cy));
        for (ox, oy, bar_w, bar_h) in bars.iter().copied() {
            if let Some(rect) = Rect::from_xywh(cx + ox, cy + oy, bar_w, bar_h) {
                pixmap.fill_rect(rect, &solid(Rgba::black(0.55).color()), spin, None);
            }
        }
    }
}

/// Walk the document once, into a pixmap of the requested size.
pub fn render(scene: &Scene, target: RenderTarget) -> Result<Pixmap, ArtError> {
    if target.w == 0 || target.h == 0 {
        return Err(ArtError::Encode(format!(
            "degenerate target {}x{}",
            target.w, target.h
        )));
    }
    let mut pixmap = Pixmap::new(target.w, target.h)
        .ok_or_else(|| ArtError::Encode(format!("pixmap {}x{}", target.w, target.h)))?;
    #[allow(clippy::cast_precision_loss)]
    let transform = Transform::from_scale(
        target.w as f32 / scene.space.w.max(1) as f32,
        target.h as f32 / scene.space.h.max(1) as f32,
    );
    paint_ground(&mut pixmap, scene, transform);
    paint_greebling(&mut pixmap, scene.panel_family, transform);
    #[allow(clippy::cast_precision_loss)]
    paint_sheen(&mut pixmap, transform, SPACE_W as f32, SPACE_H as f32);
    paint_modules(&mut pixmap, scene, transform);
    paint_vents(&mut pixmap, scene, transform);
    paint_seams(&mut pixmap, scene, transform);
    paint_fasteners(&mut pixmap, scene, transform);
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
    use crate::protocol::Rendition;

    fn scene() -> crate::art::scene::Scene {
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

    fn channels(pm: &tiny_skia::Pixmap, x: u32, y: u32) -> (u8, u8, u8, u8) {
        let p = pm.pixel(x, y).expect("in bounds");
        (p.red(), p.green(), p.blue(), p.alpha())
    }

    fn close(a: u8, b: u8) -> bool {
        u16::from(a).abs_diff(u16::from(b)) <= 2
    }

    #[test]
    fn the_two_renditions_are_the_sizes_the_spec_fixes() {
        // §7.6: card 600x900 — the scene space at its declared scale, unrounded. Hero 536x804,
        // a display box and not a scale of the card.
        assert_eq!(target_for(Rendition::Card), CARD_TARGET);
        assert_eq!(target_for(Rendition::Hero), HERO_TARGET);
        assert_eq!((CARD_TARGET.w, CARD_TARGET.h), (600, 900));
        assert_eq!((HERO_TARGET.w, HERO_TARGET.h), (536, 804));
        let card = render(&scene(), CARD_TARGET).expect("card");
        let hero = render(&scene(), HERO_TARGET).expect("hero");
        assert_eq!((card.width(), card.height()), (600, 900));
        assert_eq!((hero.width(), hero.height()), (536, 804));
    }

    #[test]
    fn the_card_is_fully_opaque_because_the_encoder_writes_straight_bytes() {
        // Task 12 encodes `pixmap.data()`, which tiny-skia keeps premultiplied. That is only
        // safe while every pixel is opaque, so the precondition is asserted here.
        let card = render(&scene(), CARD_TARGET).expect("card");
        assert!(card.pixels().iter().all(|p| p.alpha() == 255));
    }

    #[test]
    fn the_same_scene_rasterizes_to_the_same_bytes() {
        // §7.4's walk back re-derives byte-identically, which is only true end to end if the
        // rasterizer is deterministic as well as the generator.
        let a = render(&scene(), CARD_TARGET).expect("a");
        let b = render(&scene(), CARD_TARGET).expect("b");
        assert_eq!(a.data(), b.data());
    }

    #[test]
    fn the_css_gradient_line_matches_the_four_cardinal_angles() {
        // CSS: 0deg runs bottom to top, 90deg left to right.
        let (start, end) = css_gradient_line(0.0, 600.0, 900.0);
        assert!((start.x - 300.0).abs() < 0.01 && (start.y - 900.0).abs() < 0.01);
        assert!((end.x - 300.0).abs() < 0.01 && (end.y - 0.0).abs() < 0.01);
        let (start, end) = css_gradient_line(90.0, 600.0, 900.0);
        assert!((start.x - 0.0).abs() < 0.01 && (start.y - 450.0).abs() < 0.01);
        assert!((end.x - 600.0).abs() < 0.01);
        let (start, end) = css_gradient_line(180.0, 600.0, 900.0);
        assert!((start.y - 0.0).abs() < 0.01 && (end.y - 900.0).abs() < 0.01);
        // The line is long enough to cover the box at an oblique angle.
        let (start, end) = css_gradient_line(148.0, 600.0, 900.0);
        let length = ((end.x - start.x).powi(2) + (end.y - start.y).powi(2)).sqrt();
        assert!(length > 900.0, "{length}");
    }

    #[test]
    fn the_plate_is_a_hard_two_tone_split_not_a_wash() {
        // §7.3a: "a hard two-tone split so every plate has a composition rather than a single
        // wash". Sample well inside each band, before greebling and sheen.
        let scene = scene();
        let mut pm = tiny_skia::Pixmap::new(600, 900).expect("pixmap");
        paint_ground(&mut pm, &scene, tiny_skia::Transform::identity());
        let hi = to_srgb8(plate_stop(scene.plate, PlateStop::Hi));
        let mid = to_srgb8(plate_stop(scene.plate, PlateStop::MidTinted));
        assert_ne!(hi, mid, "the two stops must actually differ");
        // ang 148 runs down-right, split 40%: the top-left corner is band one.
        let (r, g, b, a) = channels(&pm, 4, 4);
        assert!(
            close(r, hi.r) && close(g, hi.g) && close(b, hi.b),
            "{r},{g},{b} vs {hi:?}"
        );
        assert_eq!(a, 255);
        let (r, g, b, _) = channels(&pm, 596, 896);
        assert!(
            close(r, mid.r) && close(g, mid.g) && close(b, mid.b),
            "{r},{g},{b} vs {mid:?}"
        );

        // **The corners alone cannot tell a split from a wash.** They sit at gradient parameter
        // t ~= 0.005 and t ~= 0.995, where a two-stop wash is already within one 8-bit level of
        // each end — the plan's own "replace the stops with a wash" check passes against them.
        // What "hard split" actually claims is that **each band is flat**, so sample two points
        // well inside each band and require them identical. Under a wash they ramp and differ.
        // Points computed from the ang-148 gradient line over 600x900 at t = .10/.35/.45/.90.
        let band_one_a = channels(&pm, 71, 83);
        let band_one_b = channels(&pm, 214, 312);
        let band_two_a = channels(&pm, 271, 404);
        let band_two_b = channels(&pm, 529, 817);
        assert_eq!(band_one_a, band_one_b, "band one must be flat, not a ramp");
        assert_eq!(band_two_a, band_two_b, "band two must be flat, not a ramp");
        assert_ne!(band_one_a, band_two_a, "and the two bands must differ");
        assert_eq!(band_one_a, (hi.r, hi.g, hi.b, 255));
        assert_eq!(band_two_a, (mid.r, mid.g, mid.b, 255));
    }

    #[test]
    fn the_greebling_families_are_four_and_they_are_visibly_different() {
        let base = scene();
        let mut rendered = Vec::new();
        for family in 0_u8..4 {
            let mut s = base.clone();
            s.panel_family = family;
            rendered.push(render(&s, CARD_TARGET).expect("render").data().to_vec());
        }
        for i in 0..rendered.len() {
            for j in (i + 1)..rendered.len() {
                assert_ne!(
                    rendered.get(i),
                    rendered.get(j),
                    "families {i} and {j} match"
                );
            }
        }
        // An unknown family index must not panic; it falls to family 0's table.
        let mut odd = base.clone();
        odd.panel_family = 9;
        assert_eq!(greebling(9).stripes.len(), greebling(0).stripes.len());
        let _ = render(&odd, CARD_TARGET).expect("render");
    }

    #[test]
    fn only_family_two_carries_a_radial_highlight() {
        assert!(greebling(2).highlight.is_some());
        for family in [0_u8, 1, 3] {
            assert!(greebling(family).highlight.is_none(), "{family}");
        }
    }

    #[test]
    fn the_sheen_lightens_the_top_left_and_leaves_the_bottom_right_alone() {
        // The static sheen is `linear-gradient(157deg, rgb(255 255 255 / .09), transparent 44%)`.
        let scene = scene();
        let mut without = tiny_skia::Pixmap::new(600, 900).expect("pixmap");
        paint_ground(&mut without, &scene, tiny_skia::Transform::identity());
        let mut with = without.clone();
        paint_sheen(&mut with, tiny_skia::Transform::identity(), 600.0, 900.0);
        let (r0, _, _, _) = channels(&without, 20, 20);
        let (r1, _, _, _) = channels(&with, 20, 20);
        assert!(r1 > r0, "the sheen must lighten the top-left: {r0} -> {r1}");
        let (rb0, _, _, _) = channels(&without, 580, 880);
        let (rb1, _, _, _) = channels(&with, 580, 880);
        assert_eq!(rb0, rb1, "and stop before the bottom-right");
    }

    #[test]
    fn a_faded_card_is_darker_than_the_same_card_lit() {
        let lit = render(&scene(), CARD_TARGET).expect("lit");
        let mut faded_scene = scene();
        faded_scene.plate = crate::art::derive::derive_plate(faded_scene.seed.h, 0.25);
        faded_scene.fade = 0.25;
        let faded = render(&faded_scene, CARD_TARGET).expect("faded");
        let mean = |pm: &tiny_skia::Pixmap| -> f64 {
            let sum: u64 = pm.pixels().iter().map(|p| u64::from(p.red())).sum();
            #[allow(clippy::cast_precision_loss)]
            {
                sum as f64 / pm.pixels().len() as f64
            }
        };
        assert!(mean(&faded) < mean(&lit));
    }

    #[test]
    fn the_jewel_reaches_the_bitmap_so_a_hue_change_moves_the_pixels() {
        let base = scene();
        let mut other = base.clone();
        other.jewel.hue = (base.jewel.hue + 120) % 360;
        let a = render(&base, CARD_TARGET).expect("a");
        let b = render(&other, CARD_TARGET).expect("b");
        assert_ne!(a.data(), b.data());
    }

    #[test]
    fn every_module_kind_draws_something_of_its_own() {
        use crate::art::compose::ModuleKind;
        let base = scene();
        let mut seen: Vec<Vec<u8>> = Vec::new();
        for kind in [
            ModuleKind::Port,
            ModuleKind::Drum,
            ModuleKind::Grille,
            ModuleKind::Screen,
            ModuleKind::Slab,
            ModuleKind::Hatch,
        ] {
            let mut s = base.clone();
            for m in &mut s.modules {
                m.kind = kind;
            }
            seen.push(render(&s, CARD_TARGET).expect("render").data().to_vec());
        }
        for i in 0..seen.len() {
            for j in (i + 1)..seen.len() {
                assert_ne!(seen.get(i), seen.get(j), "kinds {i} and {j} render alike");
            }
        }
    }

    #[test]
    fn the_two_uncomputed_marks_render_and_are_not_the_first_rung() {
        use crate::art::compose::{FastenerKind, ModuleKind};
        let base = scene();
        let mut plain = base.clone();
        for m in &mut plain.modules {
            m.kind = ModuleKind::Plain;
        }
        for f in &mut plain.fasteners {
            f.kind = FastenerKind::Plain;
        }
        let mut earliest = base.clone();
        for m in &mut earliest.modules {
            m.kind = ModuleKind::Port;
        }
        for f in &mut earliest.fasteners {
            f.kind = FastenerKind::Slotted;
        }
        let a = render(&plain, CARD_TARGET).expect("plain");
        let b = render(&earliest, CARD_TARGET).expect("earliest");
        assert_ne!(
            a.data(),
            b.data(),
            "uncomputed must not look like the earliest era"
        );
    }

    #[test]
    fn the_vent_direction_changes_the_pixels() {
        use crate::art::scene::VentDir;
        let mut horizontal = scene();
        for v in &mut horizontal.vents {
            v.dir = VentDir::H;
        }
        let mut vertical = horizontal.clone();
        for v in &mut vertical.vents {
            v.dir = VentDir::V;
        }
        assert_ne!(
            render(&horizontal, CARD_TARGET).expect("h").data(),
            render(&vertical, CARD_TARGET).expect("v").data()
        );
    }

    #[test]
    fn a_fasteners_rotation_is_drawn_and_not_ignored() {
        let mut a = scene();
        for f in &mut a.fasteners {
            f.rot = 0;
        }
        let mut b = a.clone();
        for f in &mut b.fasteners {
            f.rot = 45;
        }
        assert_ne!(
            render(&a, CARD_TARGET).expect("a").data(),
            render(&b, CARD_TARGET).expect("b").data()
        );
    }

    #[test]
    fn a_scene_with_no_livery_at_all_still_renders_an_opaque_card() {
        let mut bare = scene();
        bare.modules.clear();
        bare.vents.clear();
        bare.seams.clear();
        bare.fasteners.clear();
        let pm = render(&bare, CARD_TARGET).expect("render");
        assert!(pm.pixels().iter().all(|p| p.alpha() == 255));
    }

    #[test]
    fn a_rect_that_runs_off_the_card_is_clipped_rather_than_refused() {
        let mut wild = scene();
        if let Some(m) = wild.modules.first_mut() {
            m.rect = [-40, -40, 2000, 4000];
        }
        if let Some(v) = wild.vents.first_mut() {
            v.rect = [500, 850, 400, 400];
        }
        let pm = render(&wild, CARD_TARGET).expect("render");
        assert!(pm.pixels().iter().all(|p| p.alpha() == 255));
    }

    #[test]
    fn the_hero_draws_the_same_livery_at_its_own_scale() {
        // §7.6: a second output size driven from the same scene document — the first evidence
        // for the wrap-export guarantee.
        let s = scene();
        let card = render(&s, CARD_TARGET).expect("card");
        let hero = render(&s, HERO_TARGET).expect("hero");
        // The module's jewel edge sits at x = 48 in scene units; scaled, both must be lighter
        // there than four units to its left.
        let sample = |pm: &tiny_skia::Pixmap, sx: f64| -> u32 {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let x = (48.0_f64 * sx + 1.0) as u32;
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let y = (110.0_f64 * sx) as u32;
            let p = pm.pixel(x, y).expect("in bounds");
            u32::from(p.red()) + u32::from(p.green()) + u32::from(p.blue())
        };
        let left = |pm: &tiny_skia::Pixmap, sx: f64| -> u32 {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let x = (40.0_f64 * sx) as u32;
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let y = (110.0_f64 * sx) as u32;
            let p = pm.pixel(x, y).expect("in bounds");
            u32::from(p.red()) + u32::from(p.green()) + u32::from(p.blue())
        };
        assert!(sample(&card, 1.0) > left(&card, 1.0));
        assert!(sample(&hero, 536.0 / 600.0) > left(&hero, 536.0 / 600.0));
    }

    #[test]
    fn a_zero_sized_target_is_refused_rather_than_panicking() {
        assert!(render(&scene(), RenderTarget { w: 0, h: 900 }).is_err());
        assert!(render(&scene(), RenderTarget { w: 600, h: 0 }).is_err());
    }
}
