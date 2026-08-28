//! §7.3a's derivation, transcribed. This is the block an implementer copies, and the numbers in
//! it are not to be re-derived by eye.
//!
//! Which seed input drives what: `seed_basename` (with `reroll_offset`) drives identity colour,
//! plate and both families, and **nothing else may**. `archetype`, `language_mix` and
//! `size_bucket` drive module layout and era detail and are **not** hashed into the palette,
//! which is exactly why crossing a `size_bucket` re-renders without moving the hue (§7.4).
//! `primary_language` supplies only the designation prefix.

use crate::art::oklch::{css, Oklch};
use crate::art::seed::draw;

/// Eight jewel bins on black. Full chroma, but only ever on a few percent of the plate.
pub const JEWEL_BINS: [i32; 8] = [26, 58, 96, 148, 188, 232, 284, 328];
/// The four gradient angles the plate's hard two-tone split can take.
pub const JEWEL_ANGLES: [i32; 4] = [148, 32, 118, 62];
pub const ROMAN: [&str; 10] = ["I", "II", "III", "IV", "V", "VI", "VII", "VIII", "IX", "X"];
/// The plate is blackened steel: one hue family, ±4.
pub const PLATE_BASE_HUE: i32 = 76;

/// The ten language codes §7.3a tabulates. Anything else is `GN`.
pub const LANG_CODES: [(&str, &str); 10] = [
    ("Rust", "RS"),
    ("TypeScript", "TS"),
    ("Python", "PY"),
    ("C++", "CP"),
    ("C#", "CS"),
    ("JavaScript", "JS"),
    ("Java", "JV"),
    ("Go", "GO"),
    ("Shell", "SH"),
    ("Lua", "LU"),
];

/// §7.3: lightness and chroma serialise to three decimals. Round once, here, so the value that
/// is hashed is the value that is drawn.
#[must_use]
pub fn round3(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Jewel {
    pub hue: i32,
    #[serde(rename = "L")]
    pub l: f64,
    #[serde(rename = "C")]
    pub c: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Plate {
    pub hue: i32,
    pub c: f64,
    pub hi: f64,
    pub mid: f64,
    pub lo: f64,
    pub ang: i32,
    pub split: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlateStop {
    Hi,
    Mid,
    Lo,
    /// The second stop of the first gradient: `c * 1.06`.
    MidTinted,
}

// The draws are small moduli against a u32, so every product fits an i32 with room to spare.
#[allow(clippy::cast_possible_wrap)]
fn signed(v: u32) -> i32 {
    v as i32
}

/// ```text
/// hue = JEWEL[h % 8] + (((h >>> 5) % 7) - 3)
/// L   = 0.6 - ((h >>> 9) % 3) * 0.035 - fade * 0.12
/// C   = (0.175 - ((h >>> 13) % 3) * 0.02) * (1 - fade * 0.5)
/// ```
#[must_use]
pub fn derive_jewel(h: u32, fade: f64) -> Jewel {
    let bin = JEWEL_BINS
        .get(draw(h, 0, 8) as usize)
        .copied()
        .unwrap_or(JEWEL_BINS[0]);
    let hue = bin + (signed(draw(h, 5, 7)) - 3);
    let l = 0.6 - f64::from(draw(h, 9, 3)) * 0.035 - fade * 0.12;
    let c = (0.175 - f64::from(draw(h, 13, 3)) * 0.02) * (1.0 - fade * 0.5);
    Jewel {
        hue,
        l: round3(l),
        c: round3(c),
    }
}

#[must_use]
pub fn jewel_oklch(j: Jewel) -> Oklch {
    Oklch {
        l: j.l,
        c: j.c,
        h: f64::from(j.hue),
    }
}

#[must_use]
pub fn jewel_css(j: Jewel) -> String {
    css(jewel_oklch(j))
}

/// `jewelInk = oklch(0.87 0.07 hue)` — fade-independent, and the ink the selection ring, the
/// language plate's mono and every jewel-carrying text uses (§7.8, TOKENS.md's contrast floor).
#[must_use]
pub fn jewel_ink(hue: i32) -> String {
    css(Oklch {
        l: 0.87,
        c: 0.07,
        h: f64::from(hue),
    })
}

/// ```text
/// plateHue = 76 + (((h >>> 5) % 9) - 4)
/// c        = (0.005 + ((h >>> 9) % 3) * 0.002) * (1 - fade * 0.5)
/// step     = ((h >>> 21) % 5) * 0.011
/// hi       = 0.185 + step - fade * 0.02
/// mid      = 0.145 + step - fade * 0.016
/// lo       = 0.085 + step * 0.5 + fade * 0.005
/// ang      = [148, 32, 118, 62][(h >>> 13) % 4]
/// split    = 38 + ((h >>> 17) % 24)
/// ```
#[must_use]
pub fn derive_plate(h: u32, fade: f64) -> Plate {
    let hue = PLATE_BASE_HUE + (signed(draw(h, 5, 9)) - 4);
    let c = (0.005 + f64::from(draw(h, 9, 3)) * 0.002) * (1.0 - fade * 0.5);
    let step = f64::from(draw(h, 21, 5)) * 0.011;
    let ang = JEWEL_ANGLES
        .get(draw(h, 13, 4) as usize)
        .copied()
        .unwrap_or(JEWEL_ANGLES[0]);
    Plate {
        hue,
        c: round3(c),
        hi: round3(0.185 + step - fade * 0.02),
        mid: round3(0.145 + step - fade * 0.016),
        lo: round3(0.085 + step * 0.5 + fade * 0.005),
        ang,
        split: 38 + signed(draw(h, 17, 24)),
    }
}

/// The four stops the two stacked gradients use. The `× 1.06` and `× 0.8` multipliers are
/// renderer constants, not document fields, and are covered by `ART_SCHEMA_VERSION` (ruling 2).
#[must_use]
pub fn plate_stop(p: Plate, which: PlateStop) -> Oklch {
    let hue = f64::from(p.hue);
    match which {
        PlateStop::Hi => Oklch {
            l: p.hi,
            c: p.c,
            h: hue,
        },
        PlateStop::Mid => Oklch {
            l: p.mid,
            c: p.c,
            h: hue,
        },
        PlateStop::MidTinted => Oklch {
            l: p.mid,
            c: round3(p.c * 1.06),
            h: hue,
        },
        PlateStop::Lo => Oklch {
            l: p.lo,
            c: round3(p.c * 0.8),
            h: hue,
        },
    }
}

/// The greebling selector, `(h >>> 3) % 4`.
#[allow(clippy::cast_possible_truncation)]
#[must_use]
pub fn panel_family(h: u32) -> u8 {
    draw(h, 3, 4) as u8
}

/// The livery selector, `h % 4`. It varies vertical rhythm and vent direction only, never which
/// side a part sits on.
#[allow(clippy::cast_possible_truncation)]
#[must_use]
pub fn livery_family(h: u32) -> u8 {
    draw(h, 0, 4) as u8
}

#[must_use]
pub fn lang_prefix(primary_language: Option<&str>) -> &'static str {
    let Some(name) = primary_language else {
        return "GN";
    };
    LANG_CODES
        .iter()
        .find(|(key, _)| *key == name)
        .map_or("GN", |(_, code)| *code)
}

/// `designation = <prefix>-<10 + v(11) % 89> / MK-<ROMAN[v(3) % 10]>` where `v(n) = (h >>> n) % 100`.
#[must_use]
pub fn designation(h: u32, primary_language: Option<&str>) -> String {
    let number = 10 + draw(h, 11, 100) % 89;
    let mark = ROMAN
        .get((draw(h, 3, 100) % 10) as usize)
        .copied()
        .unwrap_or(ROMAN[0]);
    format!("{}-{number} / MK-{mark}", lang_prefix(primary_language))
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
    use crate::art::oklch::{hex, to_srgb8};
    use crate::art::seed::seed_hash;

    #[test]
    fn the_jewel_is_a_quantised_bin_with_a_small_jitter() {
        let h = seed_hash("alpha-tool");
        let j = derive_jewel(h, 0.0);
        assert_eq!(j.hue, 330); // bin 328, jitter +2
        assert!((j.l - 0.6).abs() < 1e-9);
        assert!((j.c - 0.155).abs() < 1e-9);
        assert_eq!(hex(to_srgb8(jewel_oklch(j))), "#b259ac");
        assert_eq!(jewel_ink(j.hue), "oklch(0.870 0.070 330)");
    }

    #[test]
    fn the_jitter_never_leaves_the_three_step_window_the_spec_allows() {
        for name in ["alpha-tool", "beta-lib", "widget", "atlas", "zero", "q", ""] {
            let j = derive_jewel(seed_hash(name), 0.0);
            let nearest = JEWEL_BINS
                .iter()
                .map(|bin| (j.hue - bin).abs())
                .min()
                .expect("bins are non-empty");
            assert!(nearest <= 3, "{name} landed {nearest} from a bin");
        }
    }

    #[test]
    fn fade_drains_lightness_and_chroma_by_the_stated_terms() {
        let h = seed_hash("alpha-tool");
        let lit = derive_jewel(h, 0.0);
        let faded = derive_jewel(h, 0.25);
        // §7.3a: L loses `fade * 0.12`, C is scaled by `(1 - fade * 0.5)`.
        assert_eq!(faded.hue, lit.hue, "fade never moves the hue");
        assert!((faded.l - round3(lit.l - 0.25 * 0.12)).abs() < 1e-9);
        assert!((faded.c - round3(lit.c * (1.0 - 0.25 * 0.5))).abs() < 1e-9);
        // The ink is fade-independent — it is what the selection ring is drawn in (§7.8).
        assert_eq!(jewel_ink(faded.hue), jewel_ink(lit.hue));
    }

    #[test]
    fn the_plate_is_blackened_steel_in_one_hue_family() {
        let p = derive_plate(seed_hash("alpha-tool"), 0.0);
        assert_eq!(p.hue, 74);
        assert!((p.c - 0.005).abs() < 1e-9);
        assert!((p.hi - 0.207).abs() < 1e-9);
        assert!((p.mid - 0.167).abs() < 1e-9);
        assert!((p.lo - 0.096).abs() < 1e-9);
        assert_eq!(p.ang, 148);
        assert_eq!(p.split, 40);
        // §7.3a: the plate hue is 76 ± 4, and no draw may take it outside that.
        for name in ["alpha-tool", "beta-lib", "widget", "atlas", "zero"] {
            let q = derive_plate(seed_hash(name), 0.0);
            assert!((q.hue - PLATE_BASE_HUE).abs() <= 4);
            assert!(q.split >= 38 && q.split <= 61);
            assert!(JEWEL_ANGLES.contains(&q.ang));
        }
    }

    #[test]
    fn the_second_plate_stop_is_the_tinted_one_and_the_ground_the_darkened_one() {
        let p = derive_plate(seed_hash("widget"), 0.0);
        let mid = plate_stop(p, PlateStop::Mid);
        let tinted = plate_stop(p, PlateStop::MidTinted);
        let ground = plate_stop(p, PlateStop::Lo);
        assert!((tinted.c - round3(p.c * 1.06)).abs() < 1e-9);
        assert!((ground.c - round3(p.c * 0.8)).abs() < 1e-9);
        assert!((mid.l - p.mid).abs() < 1e-9);
    }

    #[test]
    fn greebling_and_livery_are_two_different_draws_off_one_hash() {
        // §7.3a: TOKENS.md labels the set "chosen by hash % 4"; that is the LIVERY selector.
        // The greebling selector is `(h >>> 3) % 4`.
        let h = seed_hash("alpha-tool");
        assert_eq!(panel_family(h), 0);
        assert_eq!(livery_family(h), 3);
        let mut disagreed = 0;
        for name in [
            "alpha-tool",
            "beta-lib",
            "widget",
            "atlas",
            "zero",
            "q",
            "mm",
        ] {
            let h = seed_hash(name);
            if panel_family(h) != livery_family(h) {
                disagreed += 1;
            }
            assert!(panel_family(h) < 4 && livery_family(h) < 4);
        }
        assert!(disagreed > 0, "the two selectors must not be the same draw");
    }

    #[test]
    fn the_designation_takes_its_prefix_from_the_language_and_gn_when_there_is_none() {
        let h = seed_hash("alpha-tool");
        assert_eq!(designation(h, Some("Rust")), "RS-10 / MK-IX");
        assert_eq!(designation(h, Some("Go")), "GO-10 / MK-IX");
        // Not one of the ten keys, and not computed at all, both fall to GN.
        assert_eq!(designation(h, Some("Fortran")), "GN-10 / MK-IX");
        assert_eq!(designation(h, None), "GN-10 / MK-IX");
        assert_eq!(lang_prefix(Some("TypeScript")), "TS");
        assert_eq!(lang_prefix(None), "GN");
    }

    #[test]
    fn the_designation_number_and_mark_stay_inside_their_ranges() {
        for name in ["alpha-tool", "beta-lib", "widget", "atlas", "zero", "", "x"] {
            let d = designation(seed_hash(name), Some("Rust"));
            let (num, mark) = d.split_once(" / MK-").expect("designation shape");
            let value: i32 = num.trim_start_matches("RS-").parse().expect("number");
            assert!((10..=98).contains(&value), "{d}");
            assert!(ROMAN.contains(&mark), "{d}");
        }
    }

    #[test]
    fn three_decimals_is_a_rounding_not_a_truncation() {
        // §7.3: "A float that round-trips one ulp differently is a different scene_hash and a
        // wasted re-render."
        assert!((round3(0.185_000_000_000_000_02) - 0.185).abs() < f64::EPSILON);
        assert!((round3(0.1859) - 0.186).abs() < f64::EPSILON);
        assert!((round3(0.0) - 0.0).abs() < f64::EPSILON);
    }
}
