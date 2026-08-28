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
pub const SPACE_H: i32 = 900;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Space {
    pub w: i32,
    pub h: i32,
    pub origin: String,
    pub y_down: bool,
    pub units: String,
}

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
    pub ground: String,
    pub panel: String,
    pub accent: String,
    pub edge: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Module {
    pub id: String,
    pub kind: ModuleKind,
    /// `[x, y, w, h]` in the 2× space.
    pub rect: [i32; 4],
    pub face_up: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Fastener {
    /// `[x, y]` — the centre.
    pub at: [i32; 2],
    pub kind: FastenerKind,
    /// Degrees, `0..90`.
    pub rot: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SeamKind {
    Panel,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Seam {
    pub path: Vec<[i32; 2]>,
    pub kind: SeamKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VentDir {
    /// Horizontal slats — the CSS `0deg` repeating gradient.
    H,
    /// Vertical slats — the CSS `90deg` one.
    V,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Scene {
    /// The document format (§7.3's `"v"`).
    pub v: u32,
    /// The renderer schema (ruling 2). Inside the document, so inside the address.
    pub schema: u32,
    pub space: Space,
    pub seed: Seed,
    pub fade: f64,
    pub jewel: Jewel,
    pub jewel_ink: String,
    pub plate: Plate,
    pub panel_family: u8,
    pub livery_family: u8,
    pub designation: String,
    pub lang_prefix: String,
    /// `None` until J4 supplies a first commit date (§7.2).
    pub era: Option<Era>,
    /// `None` until J3 supplies tracked bytes.
    pub size_bucket: Option<SizeBucket>,
    /// `None` until J3 classifies.
    pub archetype: Option<String>,
    pub language_mix: LanguageMix,
    pub palette: Palette,
    pub modules: Vec<Module>,
    pub fasteners: Vec<Fastener>,
    pub seams: Vec<Seam>,
    pub vents: Vec<Vent>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Vent {
    pub rect: [i32; 4],
    pub dir: VentDir,
}

/// The bytes the address is taken over, and the bytes stored in `art_scene.scene_json`.
///
/// Determinism comes from three properties and no others: struct fields serialise in
/// declaration order, there is no map anywhere in the document, and every float was rounded to
/// three decimals by `derive::round3` before it got here.
pub fn canonical_json(scene: &Scene) -> Result<Vec<u8>, ArtError> {
    serde_json::to_vec(scene).map_err(|e| ArtError::Encode(e.to_string()))
}

/// §7.2: `scene_hash` is content-addressed over the scene document. SHA-256, 64 lowercase hex,
/// untruncated (ruling 3); the first two characters are the on-disk fan-out.
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
