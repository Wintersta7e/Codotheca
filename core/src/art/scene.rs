//! §7.3's scene document — which doubles as the phase-3 geometry sidecar.
//!
//! Coordinates are pixels in the 2× space with a top-left origin and y down. `faceUp` is what
//! phase-3 dust settles on; `fasteners` are where rust streaks originate; `seams` are where
//! cracks propagate. Phase 3 consumes **this document**, not a separate mask.
//!
//! **The renderer resolves nothing from a name.** The generator resolves it once, during the
//! scan, and writes the numbers down. Anything the rasterizer varies on and the document omits
//! lets two byte-identical documents render differently, so every such value is a field here —
//! which the test `every_field_the_rasterizer_reads_moves_the_hash` holds to.

use sha2::{Digest, Sha256};

use crate::art::compose::{Era, FastenerKind, LanguageMix, ModuleKind, SizeBucket};
use crate::art::derive::{Jewel, Plate};
use crate::art::seed::Seed;
use crate::art::{ArtError, ART_SCHEMA_VERSION, SCENE_FORMAT_VERSION};

/// The 2× card space §7.6 fixes at `600×900`. The two smaller figures elsewhere in the spec
/// descend from a GPU-texture approximation phase 1 no longer allocates.
pub const SPACE_W: i32 = 600;
/// The height of the same `600×900` space.
pub const SPACE_H: i32 = 900;

/// §7.3's `space`: the coordinate system every other number in the document is written in.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Space {
    /// Width in scene units; `SPACE_W` for every document the generator writes.
    pub w: i32,
    /// Height in scene units; `SPACE_H` for every document the generator writes.
    pub h: i32,
    /// Where `(0, 0)` sits: `top-left`.
    pub origin: String,
    /// Whether y grows downward: `true`.
    pub y_down: bool,
    /// What one unit is: `px@2x`.
    pub units: String,
}

/// The one space the generator writes: `600×900` `px@2x`, top-left origin, y down.
#[must_use]
pub fn card_space() -> Space {
    Space {
        w: SPACE_W,
        h: SPACE_H,
        origin: "top-left".to_owned(),
        y_down: true,
        units: "px@2x".to_owned(),
    }
}

/// The four summary colours §7.3 names. `ground` is the plate's darkest stop, which phase 3's
/// weathering reads; the rasterizer itself draws from `plate` and `jewel`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Palette {
    /// The plate's `lo` stop as an `oklch()` string.
    pub ground: String,
    /// The plate's `mid` stop as an `oklch()` string.
    pub panel: String,
    /// The jewel as an `oklch()` string.
    pub accent: String,
    /// The jewel ink, the same string as `jewelInk`.
    pub edge: String,
}

/// One machined module on the plate.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Module {
    /// `m0`, `m1`, … in stacking order, top first.
    pub id: String,
    /// The face its archetype gives it.
    pub kind: ModuleKind,
    /// `[x, y, w, h]` in the 2× space.
    pub rect: [i32; 4],
    /// Whether the face points up — where phase-3 dust settles (§7.3).
    pub face_up: bool,
}

/// One fastener head, where phase-3 rust streaks originate.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Fastener {
    /// `[x, y]` — the centre.
    pub at: [i32; 2],
    /// The era's cut, or `plain` while the era is uncomputed.
    pub kind: FastenerKind,
    /// Degrees, `0..90`.
    pub rot: i32,
}

/// What kind of break a seam is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SeamKind {
    /// A panel break, and the only kind the generator draws.
    Panel,
}

/// A seam line, where phase-3 cracks propagate.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Seam {
    /// The polyline's vertices, `[x, y]` in the 2× space.
    pub path: Vec<[i32; 2]>,
    /// What kind of break it is.
    pub kind: SeamKind,
}

/// Which way a vent bank's slats run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VentDir {
    /// Horizontal slats — the CSS `0deg` repeating gradient.
    H,
    /// Vertical slats — the CSS `90deg` one.
    V,
}

/// §7.3's scene document: everything the rasterizer draws from, and the bytes `scene_hash`
/// addresses.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Scene {
    /// The document format (§7.3's `"v"`).
    pub v: u32,
    /// The renderer schema (ruling 2). Inside the document, so inside the address.
    pub schema: u32,
    /// The coordinate system every coordinate below is written in.
    pub space: Space,
    /// The string hashed and its `u32` (§7.3a).
    pub seed: Seed,
    /// `0.25` for a reference or archived project, else `0` — never a function of time.
    pub fade: f64,
    /// The identity colour.
    pub jewel: Jewel,
    /// `oklch(0.87 0.07 <hue>)`, the fade-independent ink (§7.8).
    pub jewel_ink: String,
    /// The plate's stops, angle and split.
    pub plate: Plate,
    /// The greebling family, `(h >>> 3) % 4`.
    pub panel_family: u8,
    /// The livery family, `h % 4`: vertical rhythm and vent direction.
    pub livery_family: u8,
    /// `<prefix>-<number> / MK-<numeral>`, drawn off the seed hash.
    pub designation: String,
    /// The designation's two-letter language prefix, `GN` when there is no known language.
    pub lang_prefix: String,
    /// `None` until J4 supplies a first commit date (§7.2).
    pub era: Option<Era>,
    /// `None` until J3 supplies tracked bytes.
    pub size_bucket: Option<SizeBucket>,
    /// `None` until J3 classifies.
    pub archetype: Option<String>,
    /// The language mix that sets the vent-bank count.
    pub language_mix: LanguageMix,
    /// The four summary colours.
    pub palette: Palette,
    /// The machined modules, top to bottom.
    pub modules: Vec<Module>,
    /// The fasteners: the first module's four corners, then any on the seam.
    pub fasteners: Vec<Fastener>,
    /// The seams: the livery seam, then the odd-family cross seam when there is one.
    pub seams: Vec<Seam>,
    /// The vent banks, top to bottom.
    pub vents: Vec<Vent>,
}

/// One vent bank of slats.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Vent {
    /// `[x, y, w, h]` in the 2× space.
    pub rect: [i32; 4],
    /// Which way its slats run.
    pub dir: VentDir,
}

/// The bytes the address is taken over, and the bytes stored in `art_scene.scene_json`.
///
/// Determinism comes from three properties and no others: struct fields serialise in
/// declaration order, there is no map anywhere in the document, and every float was rounded to
/// three decimals by `derive::round3` before it got here.
///
/// # Errors
///
/// [`ArtError::Encode`], carrying `serde_json`'s message, if the document fails to serialise.
pub fn canonical_json(scene: &Scene) -> Result<Vec<u8>, ArtError> {
    serde_json::to_vec(scene).map_err(|e| ArtError::Encode(e.to_string()))
}

/// §7.2: `scene_hash` is content-addressed over the scene document. SHA-256, 64 lowercase hex,
/// untruncated (ruling 3); the first two characters are the on-disk fan-out.
///
/// # Errors
///
/// [`ArtError::Encode`] when [`canonical_json`] fails, or when a digest byte cannot be written
/// into the hex string.
pub fn scene_hash(scene: &Scene) -> Result<String, ArtError> {
    let bytes = canonical_json(scene)?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let mut out = String::with_capacity(64);
    for byte in hasher.finalize() {
        use std::fmt::Write as _;
        write!(out, "{byte:02x}").map_err(|e| ArtError::Encode(e.to_string()))?;
    }
    Ok(out)
}

// Keeps `SCENE_FORMAT_VERSION` and `ART_SCHEMA_VERSION` referenced from this module, so the two
// constants and the two fields that carry them cannot drift apart unnoticed.
const _: () = {
    assert!(SCENE_FORMAT_VERSION >= 1);
    assert!(ART_SCHEMA_VERSION >= 1);
};

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
mod tests {
    use super::*;
    use crate::art::compose::{Era, FastenerKind, LanguageMix, ModuleKind, SizeBucket};
    use crate::art::derive::{Jewel, Plate};
    use crate::art::seed::Seed;
    use sha2::{Digest, Sha256};
    use std::fmt::Write as _;

    /// One named mutation of the document, so the `Vec` below is not a bare complex tuple type.
    type Mutation = (&'static str, Box<dyn Fn(&mut Scene)>);

    fn sample() -> Scene {
        Scene {
            v: SCENE_FORMAT_VERSION,
            schema: ART_SCHEMA_VERSION,
            space: card_space(),
            seed: Seed {
                s: "alpha-tool".to_owned(),
                h: 3_035_956_391,
            },
            fade: 0.0,
            jewel: Jewel {
                hue: 330,
                l: 0.6,
                c: 0.155,
            },
            jewel_ink: "oklch(0.870 0.070 330)".to_owned(),
            plate: Plate {
                hue: 74,
                c: 0.005,
                hi: 0.207,
                mid: 0.167,
                lo: 0.096,
                ang: 148,
                split: 40,
            },
            panel_family: 0,
            livery_family: 3,
            designation: "RS-10 / MK-IX".to_owned(),
            lang_prefix: "RS".to_owned(),
            era: Some(Era::Modern),
            size_bucket: Some(SizeBucket::Medium),
            archetype: Some("library".to_owned()),
            language_mix: LanguageMix {
                primary: Some("Rust".to_owned()),
                distinct: 2,
                computed: true,
            },
            palette: Palette {
                ground: "oklch(0.096 0.004 74)".to_owned(),
                panel: "oklch(0.167 0.005 74)".to_owned(),
                accent: "oklch(0.600 0.155 330)".to_owned(),
                edge: "oklch(0.870 0.070 330)".to_owned(),
            },
            modules: vec![Module {
                id: "m0".to_owned(),
                kind: ModuleKind::Drum,
                rect: [48, 63, 228, 126],
                face_up: false,
            }],
            fasteners: vec![Fastener {
                at: [62, 77],
                kind: FastenerKind::Torx,
                rot: 37,
            }],
            seams: vec![Seam {
                path: vec![[300, 0], [300, 603]],
                kind: SeamKind::Panel,
            }],
            vents: vec![Vent {
                rect: [48, 207, 216, 36],
                dir: VentDir::H,
            }],
        }
    }

    #[test]
    fn the_coordinate_space_is_the_one_the_sidecar_declares() {
        // §7.3: pixels in the 2x space, top-left origin, y down.
        let space = card_space();
        assert_eq!((space.w, space.h), (600, 900));
        assert_eq!(space.origin, "top-left");
        assert!(space.y_down);
        assert_eq!(space.units, "px@2x");
    }

    #[test]
    fn the_document_serialises_the_way_the_spec_writes_it() {
        let json = String::from_utf8(canonical_json(&sample()).expect("json")).expect("utf8");
        assert!(
            json.starts_with(r#"{"v":1,"schema":1,"space":{"w":600,"h":900,"#),
            "{json}"
        );
        assert!(json.contains(r#""origin":"top-left","yDown":true,"units":"px@2x""#));
        assert!(json.contains(r#""seed":{"s":"alpha-tool","h":3035956391}"#));
        assert!(json.contains(r#""jewel":{"hue":330,"L":0.6,"C":0.155}"#));
        assert!(json.contains(r#""jewelInk":"oklch(0.870 0.070 330)""#));
        assert!(json.contains(
            r#""plate":{"hue":74,"c":0.005,"hi":0.207,"mid":0.167,"lo":0.096,"ang":148,"split":40}"#
        ));
        assert!(json.contains(r#""panelFamily":0,"liveryFamily":3"#));
        assert!(json.contains(r#""designation":"RS-10 / MK-IX","langPrefix":"RS""#));
        assert!(json.contains(
            r#""modules":[{"id":"m0","kind":"drum","rect":[48,63,228,126],"faceUp":false}]"#
        ));
        assert!(json.contains(r#""fasteners":[{"at":[62,77],"kind":"torx","rot":37}]"#));
        assert!(json.contains(r#""seams":[{"path":[[300,0],[300,603]],"kind":"panel"}]"#));
        assert!(json.contains(r#""vents":[{"rect":[48,207,216,36],"dir":"h"}]"#));
    }

    #[test]
    fn fade_is_a_field_of_the_document_and_therefore_of_the_address() {
        let json = String::from_utf8(canonical_json(&sample()).expect("json")).expect("utf8");
        assert!(json.contains(r#""fade":0.0"#));
        let lit = scene_hash(&sample()).expect("hash");
        let mut faded = sample();
        faded.fade = 0.25;
        assert_ne!(scene_hash(&faded).expect("hash"), lit);
    }

    #[test]
    fn an_uncomputed_input_is_null_and_not_a_rung() {
        let mut scene = sample();
        scene.era = None;
        scene.size_bucket = None;
        scene.archetype = None;
        let json = String::from_utf8(canonical_json(&scene).expect("json")).expect("utf8");
        assert!(json.contains(r#""era":null"#));
        assert!(json.contains(r#""sizeBucket":null"#));
        assert!(json.contains(r#""archetype":null"#));
    }

    #[test]
    fn the_hash_is_sixty_four_lowercase_hex_over_the_document() {
        let hash = scene_hash(&sample()).expect("hash");
        assert!(crate::art::is_scene_hash(&hash), "{hash}");
        // The hash is the digest of the bytes and nothing else, so it is checkable by hand.
        let mut hasher = Sha256::new();
        hasher.update(canonical_json(&sample()).expect("json"));
        let mut expected = String::new();
        for byte in hasher.finalize() {
            write!(expected, "{byte:02x}").expect("hex");
        }
        assert_eq!(hash, expected);
    }

    #[test]
    fn the_serialisation_is_byte_stable_across_calls() {
        // §7.3: "A float that round-trips one ulp differently is a different scene_hash and a
        // wasted re-render."
        let a = canonical_json(&sample()).expect("json");
        let b = canonical_json(&sample()).expect("json");
        assert_eq!(a, b);
        assert_eq!(
            scene_hash(&sample()).expect("h"),
            scene_hash(&sample()).expect("h")
        );
    }

    #[test]
    fn the_renderer_schema_version_is_inside_the_hash() {
        // §7.2: the address covers the document AND the renderer schema version. Ruling 2 makes
        // that one thing, so a bump has to move the hash.
        let base = scene_hash(&sample()).expect("hash");
        let mut bumped = sample();
        bumped.schema = ART_SCHEMA_VERSION + 1;
        assert_ne!(scene_hash(&bumped).expect("hash"), base);
    }

    #[test]
    fn every_field_the_rasterizer_reads_moves_the_hash() {
        // §7.2: "Everything the rasterizer varies on must be in the document, or the hash
        // collides." One mutation per field, each of which must produce a different address.
        let base = scene_hash(&sample()).expect("hash");
        let mutations: Vec<Mutation> = vec![
            ("jewel", Box::new(|s: &mut Scene| s.jewel.hue += 1)),
            ("plate.split", Box::new(|s: &mut Scene| s.plate.split += 1)),
            ("plate.ang", Box::new(|s: &mut Scene| s.plate.ang = 32)),
            ("panelFamily", Box::new(|s: &mut Scene| s.panel_family = 2)),
            (
                "liveryFamily",
                Box::new(|s: &mut Scene| s.livery_family = 1),
            ),
            ("era", Box::new(|s: &mut Scene| s.era = Some(Era::Early))),
            (
                "sizeBucket",
                Box::new(|s: &mut Scene| s.size_bucket = Some(SizeBucket::Huge)),
            ),
            (
                "languageMix",
                Box::new(|s: &mut Scene| s.language_mix.distinct = 4),
            ),
            ("modules", Box::new(|s: &mut Scene| s.modules.clear())),
            ("vents", Box::new(|s: &mut Scene| s.vents.clear())),
            ("seams", Box::new(|s: &mut Scene| s.seams.clear())),
            ("fasteners", Box::new(|s: &mut Scene| s.fasteners.clear())),
            (
                "designation",
                Box::new(|s: &mut Scene| s.designation = "GN-11 / MK-I".to_owned()),
            ),
        ];
        for (name, mutate) in mutations {
            let mut scene = sample();
            mutate(&mut scene);
            assert_ne!(
                scene_hash(&scene).expect("hash"),
                base,
                "{name} did not move the hash"
            );
        }
    }

    #[test]
    fn the_document_round_trips_because_phase_three_has_to_read_it_back() {
        // §7.3: "Phase 3 consumes this document, not a separate mask."
        let json = canonical_json(&sample()).expect("json");
        let back: Scene = serde_json::from_slice(&json).expect("parse");
        assert_eq!(canonical_json(&back).expect("json"), json);
    }
}
