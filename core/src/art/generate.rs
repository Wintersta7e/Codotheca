//! §7.1's pure function: `generate` takes scan-time facts and returns a scene document. It
//! reads no clock, touches no filesystem and issues no query — `load_inputs` does the reading,
//! separately, so the derivation itself can be exercised with nothing but a struct literal.
//!
//! §7.2's signature is `generate(basename, archetype, language_mix, size_bucket, reroll_offset)`
//! and **every input resolves by J3, during the scan.** v2's seed needed J4 outputs that arrive
//! minutes late, so art either waited — destroying the reveal, which *is* the render pass — or
//! re-rolled mid-scan, the exact recognition break §7.4 forbids.

use crate::art::compose::{
    era_of, fastener_for, language_mix_of, layout, module_kind_for, size_bucket_of, LayoutInputs,
    SizeBucket,
};
use crate::art::derive::{
    derive_jewel, derive_plate, designation, jewel_css, jewel_ink, lang_prefix, livery_family,
    panel_family, plate_stop, PlateStop,
};
use crate::art::fade::fade_for;
use crate::art::oklch::css;
use crate::art::scene::{card_space, Palette, Scene};
use crate::art::seed::seed_of;
use crate::art::{ArtError, ART_SCHEMA_VERSION, SCENE_FORMAT_VERSION};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SceneInputs {
    /// §7.4: the directory basename recorded at first index. **Never `project.name`.**
    pub seed_basename: String,
    pub reroll_offset: u32,
    pub is_reference: bool,
    pub is_archived: bool,
    pub archetype: Option<String>,
    pub primary_language: Option<String>,
    /// `project.language_bytes`, a JSON object of name → bytes.
    pub language_bytes_json: Option<String>,
    pub size_tracked_bytes: Option<i64>,
    /// The late-bound era input (§7.2). **Not a seed input.**
    pub first_commit_at: Option<i64>,
    pub first_commit_tz_offset_min: Option<i32>,
}

#[must_use]
pub fn generate(input: &SceneInputs) -> Scene {
    let seed = seed_of(&input.seed_basename, input.reroll_offset);
    // Captured before `seed` moves into the document. The designation's draws come off the same
    // hash as everything else, and taking it from here rather than re-deriving it is what stops
    // the seed being computed twice in one function and drifting.
    let seed_h = seed.h;
    let fade = fade_for(input.is_reference, input.is_archived);
    let jewel = derive_jewel(seed_h, fade);
    let plate = derive_plate(seed_h, fade);
    let panel = panel_family(seed_h);
    let livery = livery_family(seed_h);

    let size_bucket = size_bucket_of(input.size_tracked_bytes);
    let mix = language_mix_of(
        input.language_bytes_json.as_deref(),
        input.primary_language.as_deref(),
    );
    let era = era_of(input.first_commit_at, input.first_commit_tz_offset_min);
    let kind = module_kind_for(input.archetype.as_deref());

    let laid = layout(&LayoutInputs {
        h: seed_h,
        livery_family: livery,
        panel_family: panel,
        module_kind: kind,
        module_count: size_bucket.map_or(1, SizeBucket::module_count),
        vent_count: usize::from(mix.distinct.max(1)),
        fastener: fastener_for(era),
    });

    let ink = jewel_ink(jewel.hue);
    Scene {
        v: SCENE_FORMAT_VERSION,
        schema: ART_SCHEMA_VERSION,
        space: card_space(),
        seed,
        fade,
        jewel,
        jewel_ink: ink.clone(),
        plate,
        panel_family: panel,
        livery_family: livery,
        designation: designation(seed_h, input.primary_language.as_deref()),
        lang_prefix: lang_prefix(input.primary_language.as_deref()).to_owned(),
        era,
        size_bucket,
        archetype: input.archetype.clone(),
        language_mix: mix,
        palette: Palette {
            ground: css(plate_stop(plate, PlateStop::Lo)),
            panel: css(plate_stop(plate, PlateStop::Mid)),
            accent: jewel_css(jewel),
            edge: ink,
        },
        modules: laid.modules,
        fasteners: laid.fasteners,
        seams: laid.seams,
        vents: laid.vents,
    }
}

/// Everything `generate` needs, read in one statement. `None` when the project does not exist.
pub fn load_inputs(
    conn: &rusqlite::Connection,
    project_id: i64,
) -> Result<Option<SceneInputs>, ArtError> {
    let mut stmt = conn.prepare(
        "SELECT seed_basename, reroll_offset, is_reference, is_archived, archetype,
                primary_language, language_bytes, size_tracked_bytes,
                first_commit_at, first_commit_tz_offset_min
           FROM project
          WHERE id = ?1",
    )?;
    let mut rows = stmt.query([project_id])?;
    let Some(row) = rows.next()? else {
        return Ok(None);
    };
    let offset: i64 = row.get(1)?;
    Ok(Some(SceneInputs {
        seed_basename: row.get(0)?,
        reroll_offset: u32::try_from(offset.max(0)).unwrap_or(0),
        is_reference: row.get::<_, i64>(2)? != 0,
        is_archived: row.get::<_, i64>(3)? != 0,
        archetype: row.get(4)?,
        primary_language: row.get(5)?,
        language_bytes_json: row.get(6)?,
        size_tracked_bytes: row.get(7)?,
        first_commit_at: row.get(8)?,
        first_commit_tz_offset_min: row.get(9)?,
    }))
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
    use crate::art::compose::{Era, FastenerKind, SizeBucket};
    use crate::art::scene::scene_hash;

    fn base() -> SceneInputs {
        SceneInputs {
            seed_basename: "alpha-tool".to_owned(),
            reroll_offset: 0,
            is_reference: false,
            is_archived: false,
            archetype: Some("library".to_owned()),
            primary_language: Some("Rust".to_owned()),
            language_bytes_json: Some(r#"{"Rust":900000,"TypeScript":80000}"#.to_owned()),
            size_tracked_bytes: Some(2_000_000),
            first_commit_at: Some(1_600_000_000),
            first_commit_tz_offset_min: Some(0),
        }
    }

    #[test]
    fn the_generator_is_pure() {
        assert_eq!(generate(&base()), generate(&base()));
        assert_eq!(
            scene_hash(&generate(&base())).expect("h"),
            scene_hash(&generate(&base())).expect("h")
        );
    }

    #[test]
    fn the_palette_comes_from_the_basename_and_the_offset_and_from_nothing_else() {
        // §7.3a: "seed_basename (with reroll_offset) drives identity colour, plate and both
        // families, and nothing else may."
        let a = generate(&base());
        let mut moved = base();
        moved.archetype = Some("service".to_owned());
        moved.primary_language = Some("Go".to_owned());
        moved.language_bytes_json = Some(r#"{"Go":1}"#.to_owned());
        moved.size_tracked_bytes = Some(900_000_000);
        moved.first_commit_at = Some(946_684_800);
        let b = generate(&moved);
        assert_eq!(a.jewel, b.jewel);
        assert_eq!(a.plate, b.plate);
        assert_eq!(a.panel_family, b.panel_family);
        assert_eq!(a.livery_family, b.livery_family);
        // …but the document as a whole did move, so the card re-renders.
        assert_ne!(scene_hash(&a).expect("h"), scene_hash(&b).expect("h"));
    }

    #[test]
    fn crossing_a_size_bucket_re_renders_without_moving_the_hue() {
        // §7.4: "Crossing a size_bucket re-renders silently."
        let small = generate(&SceneInputs {
            size_tracked_bytes: Some(1000),
            ..base()
        });
        let huge = generate(&SceneInputs {
            size_tracked_bytes: Some(900_000_000),
            ..base()
        });
        assert_eq!(small.jewel.hue, huge.jewel.hue);
        assert_eq!(small.size_bucket, Some(SizeBucket::Tiny));
        assert_eq!(huge.size_bucket, Some(SizeBucket::Huge));
        assert!(huge.modules.len() > small.modules.len());
        assert_ne!(
            scene_hash(&small).expect("h"),
            scene_hash(&huge).expect("h")
        );
    }

    #[test]
    fn era_is_a_late_bound_render_parameter_not_a_seed_input() {
        // §7.2: "hue, composition and layout are untouched, so it is a refinement, not a re-roll."
        let unknown = generate(&SceneInputs {
            first_commit_at: None,
            ..base()
        });
        let dated = generate(&base());
        assert_eq!(unknown.seed, dated.seed, "the seed is untouched");
        assert_eq!(unknown.jewel, dated.jewel);
        assert_eq!(unknown.plate, dated.plate);
        let unknown_rects: Vec<[i32; 4]> = unknown.modules.iter().map(|m| m.rect).collect();
        let dated_rects: Vec<[i32; 4]> = dated.modules.iter().map(|m| m.rect).collect();
        assert_eq!(unknown_rects, dated_rects, "the layout is untouched");
        // What did move: the era field and the fastener kind.
        assert_eq!(unknown.era, None);
        assert_eq!(dated.era, Some(Era::Modern));
        assert!(unknown
            .fasteners
            .iter()
            .all(|f| f.kind == FastenerKind::Plain));
        assert!(dated.fasteners.iter().all(|f| f.kind == FastenerKind::Torx));
        assert_ne!(
            scene_hash(&unknown).expect("h"),
            scene_hash(&dated).expect("h")
        );
    }

    #[test]
    fn a_first_commit_arriving_on_a_previously_empty_repository_does_not_re_roll() {
        // §7.4 [v2.2]: the seed is seed_basename composed with reroll_offset and nothing else,
        // so a first commit "cannot reach the art at all: not as an empty string, not as a
        // value". What it moves is era detail.
        let empty = generate(&SceneInputs {
            first_commit_at: None,
            ..base()
        });
        let first = generate(&SceneInputs {
            first_commit_at: Some(1_600_000_000),
            ..base()
        });
        assert_eq!(empty.seed.s, "alpha-tool");
        assert_eq!(first.seed, empty.seed);
        assert_eq!(first.livery_family, empty.livery_family);
        assert_eq!(first.panel_family, empty.panel_family);
    }

    #[test]
    fn the_reroll_offset_is_the_one_input_that_moves_the_hue() {
        let zero = generate(&base());
        let one = generate(&SceneInputs {
            reroll_offset: 1,
            ..base()
        });
        assert_ne!(zero.jewel.hue, one.jewel.hue);
        assert_eq!(one.seed.s, "alpha-tool#1");
        // §7.4's walk back: offset n-1 re-derives byte-identically.
        let back = generate(&SceneInputs {
            reroll_offset: 0,
            ..base()
        });
        assert_eq!(scene_hash(&back).expect("h"), scene_hash(&zero).expect("h"));
    }

    #[test]
    fn reference_and_archived_are_the_only_two_flags_that_fade_a_card() {
        assert!((generate(&base()).fade - 0.0).abs() < f64::EPSILON);
        let reference = generate(&SceneInputs {
            is_reference: true,
            ..base()
        });
        let archived = generate(&SceneInputs {
            is_archived: true,
            ..base()
        });
        assert!((reference.fade - 0.25).abs() < f64::EPSILON);
        assert!((archived.fade - 0.25).abs() < f64::EPSILON);
        assert_eq!(
            reference.jewel.hue,
            generate(&base()).jewel.hue,
            "fade never moves the hue"
        );
    }

    #[test]
    fn the_palette_summary_agrees_with_the_fields_it_summarises() {
        let scene = generate(&base());
        assert_eq!(scene.palette.accent, jewel_css(scene.jewel));
        assert_eq!(scene.palette.edge, scene.jewel_ink);
        assert_eq!(scene.palette.edge, jewel_ink(scene.jewel.hue));
    }

    #[test]
    fn an_entirely_uncomputed_project_still_generates_a_finished_document() {
        // J5 can run before J3 and J4 have produced anything at all.
        let bare = SceneInputs {
            seed_basename: "widget".to_owned(),
            ..SceneInputs::default()
        };
        let scene = generate(&bare);
        assert_eq!(scene.era, None);
        assert_eq!(scene.size_bucket, None);
        assert_eq!(scene.archetype, None);
        assert!(!scene.language_mix.computed);
        assert_eq!(scene.lang_prefix, "GN");
        assert_eq!(scene.modules.len(), 1);
        assert_eq!(scene.vents.len(), 1);
        assert!(crate::art::is_scene_hash(&scene_hash(&scene).expect("h")));
    }

    #[test]
    fn the_row_the_generator_reads_is_the_row_it_was_given() {
        let dir = tempfile::tempdir().expect("tempdir");
        let index = crate::index::Index::open(dir.path()).expect("open");
        index
            .conn()
            .execute(
                "INSERT INTO project (id, name, seed_basename, reroll_offset, archetype,
                                      primary_language, language_bytes, size_tracked_bytes,
                                      first_commit_at, first_commit_tz_offset_min,
                                      is_reference, is_archived, created_at, updated_at)
                 VALUES (7, 'alpha tool', 'alpha-tool', 2, 'library', 'Rust',
                         '{\"Rust\":10}', 4096, 1600000000, 60, 0, 1, 0, 0)",
                [],
            )
            .expect("insert");
        let loaded = load_inputs(index.conn(), 7).expect("load").expect("row");
        assert_eq!(loaded.seed_basename, "alpha-tool");
        assert_eq!(loaded.reroll_offset, 2);
        assert!(loaded.is_archived);
        assert!(!loaded.is_reference);
        assert_eq!(loaded.first_commit_tz_offset_min, Some(60));
        assert_eq!(load_inputs(index.conn(), 999).expect("load"), None);
    }
}
