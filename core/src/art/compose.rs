//! §7.2's non-palette seed inputs, and the late-bound era.
//!
//! These drive **module layout and era detail** and are deliberately *not* hashed into the
//! palette (§7.3a), which is why crossing a `size_bucket` re-renders without moving the hue, and
//! why a first commit arriving on a previously-empty repository refines the card rather than
//! re-rolling it.
//!
//! Each of them can be uncomputed, because J5 may run before J3 or J4 has produced anything.
//! Uncomputed is `None` in the document and a mark **outside** the ladder on the card —
//! §7.7a's rule, applied to the only other ladder the bitmap carries.

use std::collections::BTreeMap;

use crate::art::scene::{Fastener, Module, Seam, SeamKind, Vent, VentDir, SPACE_H, SPACE_W};
use crate::art::seed::draw;

/// Languages below this share of total tracked bytes do not count towards the mix.
pub const MIX_SHARE_FLOOR_PERCENT: u64 = 5;
/// More than this many and the card would grow a fifth vent bank it has no room for.
pub const MIX_DISTINCT_CAP: u8 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SizeBucket {
    Tiny,
    Small,
    Medium,
    Large,
    Huge,
}

impl SizeBucket {
    #[must_use]
    pub fn slug(self) -> &'static str {
        match self {
            Self::Tiny => "tiny",
            Self::Small => "small",
            Self::Medium => "medium",
            Self::Large => "large",
            Self::Huge => "huge",
        }
    }

    /// How many machined modules the plate carries. Ruling 5: a composition parameter, not a
    /// readout — nothing the bitmap paints states a size.
    #[must_use]
    pub fn module_count(self) -> usize {
        match self {
            Self::Tiny | Self::Small => 1,
            Self::Medium | Self::Large => 2,
            Self::Huge => 3,
        }
    }
}

/// Buckets on `project.size_tracked_bytes` (J3). `None` is "J3 has not run", which is not the
/// smallest bucket: `Tiny` is a measurement of a small repository.
#[must_use]
pub fn size_bucket_of(size_tracked_bytes: Option<i64>) -> Option<SizeBucket> {
    let bytes = size_tracked_bytes?;
    if bytes < 0 {
        return None;
    }
    Some(match bytes {
        0..=65_535 => SizeBucket::Tiny,
        65_536..=1_048_575 => SizeBucket::Small,
        1_048_576..=16_777_215 => SizeBucket::Medium,
        16_777_216..=268_435_455 => SizeBucket::Large,
        _ => SizeBucket::Huge,
    })
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LanguageMix {
    pub primary: Option<String>,
    pub distinct: u8,
    /// False when `language_bytes` is absent, unparsable or sums to zero. The card still draws
    /// one vent bank; the document records that nothing was measured.
    pub computed: bool,
}

/// `project.language_bytes` is a JSON object of language name → bytes (J3).
#[must_use]
pub fn language_mix_of(
    language_bytes_json: Option<&str>,
    primary_language: Option<&str>,
) -> LanguageMix {
    let primary = primary_language.map(str::to_owned);
    let parsed: Option<BTreeMap<String, u64>> =
        language_bytes_json.and_then(|raw| serde_json::from_str(raw).ok());
    let Some(map) = parsed else {
        return LanguageMix {
            primary,
            distinct: 0,
            computed: false,
        };
    };
    let total: u64 = map.values().copied().sum();
    if total == 0 {
        return LanguageMix {
            primary,
            distinct: 0,
            computed: false,
        };
    }
    let counted = map
        .values()
        .filter(|bytes| **bytes * 100 >= total * MIX_SHARE_FLOOR_PERCENT)
        .count();
    let distinct = u8::try_from(counted)
        .unwrap_or(MIX_DISTINCT_CAP)
        .min(MIX_DISTINCT_CAP);
    LanguageMix {
        primary,
        distinct,
        computed: true,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Era {
    Early,
    Middle,
    Modern,
}

impl Era {
    #[must_use]
    pub fn slug(self) -> &'static str {
        match self {
            Self::Early => "early",
            Self::Middle => "middle",
            Self::Modern => "modern",
        }
    }

    #[must_use]
    pub fn fastener(self) -> FastenerKind {
        match self {
            Self::Early => FastenerKind::Slotted,
            Self::Middle => FastenerKind::Hex,
            Self::Modern => FastenerKind::Torx,
        }
    }
}

/// The proleptic Gregorian year of a unix timestamp at a fixed offset. Hinnant's
/// `civil_from_days`, kept here rather than taken from a job module so this file has no
/// cross-plan dependency at all — and so a reviewer can see that no clock is read.
#[must_use]
pub fn local_year(unix_secs: i64, tz_offset_min: i32) -> i32 {
    let shifted = unix_secs.saturating_add(i64::from(tz_offset_min).saturating_mul(60));
    let days = shifted.div_euclid(86_400);
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    if month <= 2 {
        year += 1;
    }
    i32::try_from(year).unwrap_or(i32::MAX)
}

/// §7.2: "When J4 later supplies the first commit date, the card re-renders once with
/// period-appropriate fastener and plate detail — hue, composition and layout are untouched."
/// Three bands, on the committer's own calendar year.
#[must_use]
pub fn era_of(first_commit_at: Option<i64>, tz_offset_min: Option<i32>) -> Option<Era> {
    let at = first_commit_at?;
    let year = local_year(at, tz_offset_min.unwrap_or(0));
    Some(match year {
        ..=2009 => Era::Early,
        2010..=2017 => Era::Middle,
        _ => Era::Modern,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FastenerKind {
    Slotted,
    Hex,
    Torx,
    /// Era not computed. A kind no era can produce, so an uncomputed birth year never reads as
    /// the earliest one (ruling 4).
    Plain,
}

impl FastenerKind {
    #[must_use]
    pub fn slug(self) -> &'static str {
        match self {
            Self::Slotted => "slotted",
            Self::Hex => "hex",
            Self::Torx => "torx",
            Self::Plain => "plain",
        }
    }
}

#[must_use]
pub fn fastener_for(era: Option<Era>) -> FastenerKind {
    era.map_or(FastenerKind::Plain, Era::fastener)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModuleKind {
    Port,
    Drum,
    Grille,
    Screen,
    Slab,
    Hatch,
    Blank,
    /// Archetype not computed — outside the set the classifier can produce.
    Plain,
}

impl ModuleKind {
    #[must_use]
    pub fn slug(self) -> &'static str {
        match self {
            Self::Port => "port",
            Self::Drum => "drum",
            Self::Grille => "grille",
            Self::Screen => "screen",
            Self::Slab => "slab",
            Self::Hatch => "hatch",
            Self::Blank => "blank",
            Self::Plain => "plain",
        }
    }

    /// §7.3: `faceUp` is what phase-3 dust settles on.
    #[must_use]
    pub fn face_up(self) -> bool {
        matches!(self, Self::Screen | Self::Slab | Self::Hatch)
    }
}

/// The eight verdicts plan 09's classifier can produce, plus the absence.
#[must_use]
pub fn module_kind_for(archetype: Option<&str>) -> ModuleKind {
    match archetype {
        Some("cli") => ModuleKind::Port,
        Some("library") => ModuleKind::Drum,
        Some("service") => ModuleKind::Grille,
        Some("site") => ModuleKind::Screen,
        Some("docs") => ModuleKind::Hatch,
        Some("config") => ModuleKind::Blank,
        // Two verdicts, one face. `unclassified` means the classifier ran and reached no
        // verdict, which is still a reading — it shares `Slab` with `notebook` rather than
        // taking `Plain`, because `Plain` says J3 never ran at all.
        Some("notebook" | "unclassified") => ModuleKind::Slab,
        _ => ModuleKind::Plain,
    }
}

/// The prototype's livery percentages, resolved into the 2× space once.
pub const MODULE_X: i32 = 48; // modLeft 8%
pub const MODULE_W: i32 = 228; // modW 38%
pub const MODULE_H: i32 = 126; // modH 14%
pub const MODULE_GAP: i32 = 18;
/// `modTop`: `8%` for families 0 and 1, `7%` for 2 and 3.
pub const MODULE_Y_SHALLOW: i32 = 72;
pub const MODULE_Y_DEEP: i32 = 63;
pub const VENT_X: i32 = 48;
pub const VENT_W: i32 = 216; // ventW 36%
pub const VENT_BANK_H: i32 = 36;
pub const VENT_GAP: i32 = 12;
/// `seamStyle: left:50%; top:0; bottom:33%`.
pub const SEAM_X: i32 = SPACE_W / 2;
pub const SEAM_Y_END: i32 = SPACE_H - (SPACE_H * 33 / 100);
pub const FASTENER_INSET: i32 = 14;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    pub modules: Vec<Module>,
    pub vents: Vec<Vent>,
    pub seams: Vec<Seam>,
    pub fasteners: Vec<Fastener>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayoutInputs {
    pub h: u32,
    pub livery_family: u8,
    pub panel_family: u8,
    pub module_kind: ModuleKind,
    pub module_count: usize,
    pub vent_count: usize,
    pub fastener: FastenerKind,
}

// The draws are `% 90`, so the product always fits.
#[allow(clippy::cast_possible_wrap, clippy::cast_possible_truncation)]
fn rotation(h: u32, index: usize) -> i32 {
    let shift = 7 + (index as u32 % 8) * 3;
    draw(h, shift, 90) as i32
}

/// Where the machined parts sit. Pure, and total: a zero count produces an empty vector rather
/// than a panic, because the counts come from composition parameters that can be uncomputed.
#[must_use]
pub fn layout(input: &LayoutInputs) -> Layout {
    let module_y0 = if input.livery_family < 2 {
        MODULE_Y_SHALLOW
    } else {
        MODULE_Y_DEEP
    };
    let face_up = input.module_kind.face_up();

    let mut modules = Vec::with_capacity(input.module_count);
    for index in 0..input.module_count {
        let step = i32::try_from(index).unwrap_or(0) * (MODULE_H + MODULE_GAP);
        modules.push(Module {
            id: format!("m{index}"),
            kind: input.module_kind,
            rect: [MODULE_X, module_y0 + step, MODULE_W, MODULE_H],
            face_up,
        });
    }

    let modules_deep = i32::try_from(input.module_count).unwrap_or(0) * (MODULE_H + MODULE_GAP);
    let vent_y0 = module_y0 + modules_deep;
    let dir = if input.livery_family % 2 == 1 {
        VentDir::V
    } else {
        VentDir::H
    };
    let mut vents = Vec::with_capacity(input.vent_count);
    for index in 0..input.vent_count {
        let step = i32::try_from(index).unwrap_or(0) * (VENT_BANK_H + VENT_GAP);
        vents.push(Vent {
            rect: [VENT_X, vent_y0 + step, VENT_W, VENT_BANK_H],
            dir,
        });
    }

    // The livery seam is always drawn; an odd greebling family adds one across the plate just
    // above the vent bank, which is what gives families 1 and 3 their harder panel break.
    let mut seams = vec![Seam {
        path: vec![[SEAM_X, 0], [SEAM_X, SEAM_Y_END]],
        kind: SeamKind::Panel,
    }];
    if input.panel_family % 2 == 1 {
        let y = (vent_y0 - VENT_GAP).clamp(0, SPACE_H);
        seams.push(Seam {
            path: vec![[0, y], [SPACE_W, y]],
            kind: SeamKind::Panel,
        });
    }

    let mut fasteners = Vec::new();
    if let Some(first) = modules.first() {
        let [x, y, w, h] = first.rect;
        let corners = [
            [x + FASTENER_INSET, y + FASTENER_INSET],
            [x + w - FASTENER_INSET, y + FASTENER_INSET],
            [x + FASTENER_INSET, y + h - FASTENER_INSET],
            [x + w - FASTENER_INSET, y + h - FASTENER_INSET],
        ];
        for (index, at) in corners.into_iter().enumerate() {
            fasteners.push(Fastener {
                at,
                kind: input.fastener,
                rot: rotation(input.h, index),
            });
        }
        // The deeper rhythm exposes the seam, so it gets two fasteners of its own.
        if input.livery_family >= 2 {
            for (index, y) in [120_i32, 480].into_iter().enumerate() {
                fasteners.push(Fastener {
                    at: [SEAM_X, y],
                    kind: input.fastener,
                    rot: rotation(input.h, index + 4),
                });
            }
        }
    }

    Layout {
        modules,
        vents,
        seams,
        fasteners,
    }
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
    use crate::art::scene::{SeamKind, VentDir, SPACE_H, SPACE_W};

    fn inputs(livery: u8, panel: u8, modules: usize, vents: usize) -> LayoutInputs {
        LayoutInputs {
            h: crate::art::seed::seed_hash("alpha-tool"),
            livery_family: livery,
            panel_family: panel,
            module_kind: ModuleKind::Drum,
            module_count: modules,
            vent_count: vents,
            fastener: FastenerKind::Torx,
        }
    }

    #[test]
    fn the_livery_family_varies_vertical_rhythm_and_never_which_side_a_part_sits_on() {
        // §7.3a: "liveryFamily varies vertical rhythm and vent direction only, never which side
        // a part sits on."
        let early = layout(&inputs(0, 0, 1, 1));
        let late = layout(&inputs(2, 0, 1, 1));
        let [ex, ey, ew, eh] = early.modules.first().expect("module").rect;
        let [lx, ly, lw, lh] = late.modules.first().expect("module").rect;
        assert_eq!(
            (ex, ew, eh),
            (lx, lw, lh),
            "the side and the size are fixed"
        );
        assert_ne!(ey, ly, "the vertical rhythm is not");
        assert_eq!((ex, ew), (MODULE_X, MODULE_W));
        assert_eq!(ey, 72);
        assert_eq!(ly, 63);
    }

    #[test]
    fn the_vent_direction_alternates_with_the_family_parity() {
        assert_eq!(
            layout(&inputs(0, 0, 1, 1)).vents.first().expect("vent").dir,
            VentDir::H
        );
        assert_eq!(
            layout(&inputs(1, 0, 1, 1)).vents.first().expect("vent").dir,
            VentDir::V
        );
        assert_eq!(
            layout(&inputs(2, 0, 1, 1)).vents.first().expect("vent").dir,
            VentDir::H
        );
        assert_eq!(
            layout(&inputs(3, 0, 1, 1)).vents.first().expect("vent").dir,
            VentDir::V
        );
    }

    #[test]
    fn modules_stack_and_the_vents_start_below_the_last_one() {
        let l = layout(&inputs(0, 0, 3, 2));
        assert_eq!(l.modules.len(), 3);
        assert_eq!(l.vents.len(), 2);
        let ids: Vec<&str> = l.modules.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["m0", "m1", "m2"]);
        let last = l.modules.last().expect("module").rect;
        let [_, mod_y, _, mod_h] = last;
        let [_, vent_y, _, _] = l.vents.first().expect("vent").rect;
        assert!(vent_y >= mod_y + mod_h, "a vent must not sit on a module");
    }

    #[test]
    fn nothing_the_layout_emits_leaves_the_declared_space() {
        // The worst case: the deepest module rhythm, the most modules and the most vents.
        for livery in 0_u8..4 {
            let laid = layout(&inputs(livery, 0, 3, 4));
            for module in &laid.modules {
                let [x, y, w, h] = module.rect;
                assert!(
                    x >= 0 && y >= 0 && x + w <= SPACE_W && y + h <= SPACE_H,
                    "{:?}",
                    module.rect
                );
            }
            for vent in &laid.vents {
                let [x, y, w, h] = vent.rect;
                assert!(
                    x >= 0 && y >= 0 && x + w <= SPACE_W && y + h <= SPACE_H,
                    "{:?}",
                    vent.rect
                );
            }
            for fastener in &laid.fasteners {
                let [x, y] = fastener.at;
                assert!((0..=SPACE_W).contains(&x) && (0..=SPACE_H).contains(&y));
                assert!((0..90).contains(&fastener.rot));
            }
            for seam in &laid.seams {
                for point in &seam.path {
                    let [x, y] = *point;
                    assert!((0..=SPACE_W).contains(&x) && (0..=SPACE_H).contains(&y));
                }
            }
        }
    }

    #[test]
    fn the_panel_seam_is_always_there_and_an_odd_greebling_family_adds_a_cross_seam() {
        let even = layout(&inputs(0, 0, 1, 1));
        let odd = layout(&inputs(0, 1, 1, 1));
        assert_eq!(even.seams.len(), 1);
        assert_eq!(odd.seams.len(), 2);
        assert_eq!(even.seams.first().expect("seam").kind, SeamKind::Panel);
        assert_eq!(
            even.seams.first().expect("seam").path,
            vec![[SEAM_X, 0], [SEAM_X, SEAM_Y_END]]
        );
    }

    #[test]
    fn every_fastener_takes_the_eras_kind_and_a_seeded_rotation() {
        let l = layout(&inputs(3, 0, 1, 1));
        assert!(l.fasteners.len() >= 4, "the first module gets four corners");
        assert!(l.fasteners.iter().all(|f| f.kind == FastenerKind::Torx));
        let rots: std::collections::BTreeSet<i32> = l.fasteners.iter().map(|f| f.rot).collect();
        assert!(rots.len() > 1, "the rotations are drawn, not constant");
    }

    #[test]
    fn a_layout_with_no_modules_still_produces_a_valid_document() {
        // module_count is never zero in practice, but a zero must not panic or index off the end.
        let l = layout(&inputs(0, 0, 0, 0));
        assert!(l.modules.is_empty());
        assert!(l.vents.is_empty());
        assert!(l.fasteners.is_empty());
        assert_eq!(l.seams.len(), 1);
    }

    #[test]
    fn the_layout_is_a_pure_function_of_its_inputs() {
        assert_eq!(layout(&inputs(2, 1, 2, 3)), layout(&inputs(2, 1, 2, 3)));
    }

    #[test]
    fn the_size_bucket_is_a_log_ladder_over_tracked_bytes() {
        assert_eq!(size_bucket_of(Some(0)), Some(SizeBucket::Tiny));
        assert_eq!(size_bucket_of(Some(65_535)), Some(SizeBucket::Tiny));
        assert_eq!(size_bucket_of(Some(65_536)), Some(SizeBucket::Small));
        assert_eq!(size_bucket_of(Some(1_048_575)), Some(SizeBucket::Small));
        assert_eq!(size_bucket_of(Some(1_048_576)), Some(SizeBucket::Medium));
        assert_eq!(size_bucket_of(Some(16_777_216)), Some(SizeBucket::Large));
        assert_eq!(size_bucket_of(Some(268_435_456)), Some(SizeBucket::Huge));
    }

    #[test]
    fn an_unmeasured_size_is_none_and_never_the_smallest_bucket() {
        // §1.10: NULL is "not computed", not "zero bytes". Tiny is a measurement.
        assert_eq!(size_bucket_of(None), None);
        // A negative count is not a measurement either.
        assert_eq!(size_bucket_of(Some(-1)), None);
        // Ruling 5: the count is a composition parameter, so the uncomputed case still draws
        // something — it just records that it measured nothing.
        assert_eq!(SizeBucket::Tiny.module_count(), 1);
        assert_eq!(SizeBucket::Huge.module_count(), 3);
    }

    #[test]
    fn the_language_mix_counts_languages_that_hold_a_real_share() {
        let json = r#"{"Rust":900000,"TypeScript":80000,"Shell":19000,"Makefile":1000}"#;
        let mix = language_mix_of(Some(json), Some("Rust"));
        assert!(mix.computed);
        assert_eq!(mix.primary.as_deref(), Some("Rust"));
        // Rust 90%, TypeScript 8%, Shell 1.9%, Makefile 0.1% -> two clear the 5% floor.
        assert_eq!(mix.distinct, 2);
    }

    #[test]
    fn the_mix_is_capped_so_a_polyglot_repository_does_not_grow_a_fifth_vent() {
        let json = r#"{"a":10,"b":10,"c":10,"d":10,"e":10,"f":10,"g":10,"h":10,"i":10,"j":10}"#;
        assert_eq!(language_mix_of(Some(json), Some("a")).distinct, 4);
    }

    #[test]
    fn an_unparsed_or_absent_language_map_is_uncomputed_not_empty() {
        for input in [None, Some("{}"), Some("not json"), Some(r#"{"Rust":0}"#)] {
            let mix = language_mix_of(input, None);
            assert!(!mix.computed, "{input:?}");
            assert_eq!(mix.distinct, 0);
        }
    }

    #[test]
    fn the_local_year_is_the_committers_year_not_the_machines() {
        // 2018-01-01T00:30:00Z is still 2017 for a committer an hour west.
        let at = 1_514_766_600_i64;
        assert_eq!(local_year(at, 0), 2018);
        assert_eq!(local_year(at, -60), 2017);
        assert_eq!(local_year(0, 0), 1970);
        assert_eq!(local_year(0, -13 * 60), 1969);
    }

    #[test]
    fn the_era_bands_are_three_and_an_uncomputed_first_commit_is_none() {
        assert_eq!(era_of(Some(946_684_800), Some(0)), Some(Era::Early)); // 2000
        assert_eq!(era_of(Some(1_262_304_000), Some(0)), Some(Era::Middle)); // 2010
        assert_eq!(era_of(Some(1_514_764_800), Some(0)), Some(Era::Modern)); // 2018
        assert_eq!(era_of(Some(1_262_303_999), Some(0)), Some(Era::Early)); // 2009-12-31
                                                                            // §7.2: era is a late-bound render parameter. Before J4 there is nothing to bind.
        assert_eq!(era_of(None, Some(0)), None);
        // A missing offset is not a claim that the committer was at UTC; treat it as UTC only
        // because the timestamp itself is UTC, and record the era all the same.
        assert_eq!(era_of(Some(1_514_764_800), None), Some(Era::Modern));
    }

    #[test]
    fn an_uncomputed_era_draws_a_fastener_no_era_can_produce() {
        // Ruling 4 / §7.7a's rule applied to the card's other ladder: unknown is not the
        // earliest rung, it is outside the ladder.
        assert_eq!(fastener_for(None), FastenerKind::Plain);
        assert_eq!(fastener_for(Some(Era::Early)), FastenerKind::Slotted);
        assert_eq!(fastener_for(Some(Era::Middle)), FastenerKind::Hex);
        assert_eq!(fastener_for(Some(Era::Modern)), FastenerKind::Torx);
        for era in [Era::Early, Era::Middle, Era::Modern] {
            assert_ne!(fastener_for(Some(era)), FastenerKind::Plain);
        }
    }

    #[test]
    fn every_archetype_the_classifier_can_produce_has_a_module_kind() {
        for archetype in [
            "cli",
            "library",
            "service",
            "site",
            "notebook",
            "docs",
            "config",
            "unclassified",
        ] {
            let kind = module_kind_for(Some(archetype));
            assert_ne!(
                kind,
                ModuleKind::Plain,
                "{archetype} is a verdict, not an absence"
            );
        }
        // NULL archetype is "J3 has not run", which is outside the set.
        assert_eq!(module_kind_for(None), ModuleKind::Plain);
        assert_eq!(module_kind_for(Some("something-new")), ModuleKind::Plain);
        assert!(ModuleKind::Screen.face_up());
        assert!(!ModuleKind::Drum.face_up());
    }

    #[test]
    fn every_slug_is_stable_because_it_is_serialised_into_the_hashed_document() {
        assert_eq!(SizeBucket::Medium.slug(), "medium");
        assert_eq!(Era::Middle.slug(), "middle");
        assert_eq!(FastenerKind::Torx.slug(), "torx");
        assert_eq!(ModuleKind::Grille.slug(), "grille");
        // Renaming one of these changes every scene_hash in the library.
        assert_eq!(
            serde_json::to_string(&SizeBucket::Huge).expect("json"),
            "\"huge\""
        );
        assert_eq!(
            serde_json::to_string(&Era::Modern).expect("json"),
            "\"modern\""
        );
    }
}
