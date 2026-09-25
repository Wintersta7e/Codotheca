//! §8.2's `projects.list`: era sections and their coverage-carrying aggregates first, rows
//! second, and the `ProjectPage` R37 found nothing anywhere produced.
#![cfg(feature = "testkit")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use codotheca_core::art::testsupport::CollectingSink;
use codotheca_core::index::Index;
use codotheca_core::projects::list::{
    era_section_id_for, era_section_order, order_key_of, rank_of, sort_rows, ERA_NAMED_YEARS,
};
use codotheca_core::projects::rows::{LoadedRow, RowFacts};
use codotheca_core::projects::{dispatch_projects_command, ProjectsCtx};
use codotheca_core::protocol::{HealthState, HealthSummary, ProjectLifecycle, ProjectRow, SortKey};

/// 2026-06-11T12:00:00Z, so the cut year is 2026 and the ten named years run 2025 down to 2016.
const NOW: i64 = 1_781_179_200;
const DAY: i64 = 86_400;

fn at(secs: i64) -> ProjectRow {
    let mut row = ProjectRow::for_test(1);
    row.last_touched_at = secs;
    row
}

const fn loaded(row: ProjectRow) -> LoadedRow {
    LoadedRow {
        row,
        facts: RowFacts {
            authored_by_user: None,
            location_kind: None,
            distro: None,
            has_remote: false,
            has_submodules: false,
            has_readme: None,
            content_presence: None,
        },
    }
}

/// A row in **one** era section — every fixture below sits a day back, so `era:live` holds all of
/// them and *after every ranked row within its own section* is the whole list. The bucketing is
/// `AC-P3-35-9`'s claim and is not restated here.
fn with_health(id: i64, state: HealthState, scored_open: Option<u32>) -> ProjectRow {
    let mut row = ProjectRow::for_test(id);
    row.last_touched_at = NOW - DAY;
    row.health_summary = HealthSummary {
        state,
        scored_open,
        unverified: None,
        unknown_checks: None,
        observed_at: None,
    };
    row
}

/// Moves a fixture row back by whole days, so one fixture can span four era sections.
const fn touched(mut row: ProjectRow, days_ago: i64) -> ProjectRow {
    row.last_touched_at = NOW - days_ago * DAY;
    row
}

/// `SortKey`'s variants off the **tracked** §2.4 contract, the same source
/// `app/src/renderer/shelf/viewState.test.ts` derives from. A count a human maintains is a defect
/// with a delay.
fn schema_sort_variants() -> Vec<String> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../protocol/schema/protocol.json"
    );
    let text = std::fs::read_to_string(path).expect("protocol.json is readable");
    let doc: serde_json::Value = serde_json::from_str(&text).expect("protocol.json is JSON");
    doc["types"]["SortKey"]["variants"]
        .as_array()
        .expect("SortKey declares variants")
        .iter()
        .map(|v| v.as_str().expect("a variant").to_owned())
        .collect()
}

fn sorted_ids(rows: &[LoadedRow], sort: SortKey) -> Vec<i64> {
    let mut refs: Vec<&LoadedRow> = rows.iter().collect();
    sort_rows(&mut refs, sort);
    refs.iter().map(|r| r.row.id.0).collect()
}

#[test]
fn the_id_and_order_tables_are_section_eight_ones_verbatim() {
    assert_eq!(era_section_id_for(&at(NOW - DAY), NOW, 0), "era:live");
    assert_eq!(era_section_id_for(&at(NOW - 20 * DAY), NOW, 0), "era:month");
    assert_eq!(era_section_id_for(&at(NOW - 60 * DAY), NOW, 0), "era:q");
    assert_eq!(era_section_id_for(&at(NOW - 150 * DAY), NOW, 0), "era:year");
    assert_eq!(era_section_id_for(&at(NOW - 400 * DAY), NOW, 0), "era:2025");

    for (id, order) in [
        ("era:live", 0),
        ("era:month", 1),
        ("era:q", 2),
        ("era:year", 3),
        ("era:2025", 11),
        ("era:2016", 20),
        ("era:tail", 90),
        ("era:archived", 92),
        ("era:submodules", 94),
        ("era:notcloned", 98),
    ] {
        assert_eq!(era_section_order(id, 2026), order, "{id}");
    }
    assert_eq!(ERA_NAMED_YEARS, 10);
}

#[test]
fn bands_cut_on_the_calendar_year_not_a_rolling_three_sixty_five() {
    // The design's `2026 - floor(days/365)` files an October-2025 touch under EARLIER THIS YEAR
    // in January: two of its own labels claim one project.
    let october_last = 1_759_276_800; // 2025-10-01T00:00:00Z
    assert_eq!(era_section_id_for(&at(october_last), NOW, 0), "era:2025");
}

#[test]
fn archived_and_submodules_are_overrides_and_archived_wins() {
    let mut row = at(NOW);
    row.is_submodule = true;
    assert_eq!(era_section_id_for(&row, NOW, 0), "era:submodules");
    row.is_archived = true;
    assert_eq!(era_section_id_for(&row, NOW, 0), "era:archived");
}

/// AC-P2-23-9, replacing `era_notcloned_is_never_emitted_in_phase_one`.
///
/// The old bar iterated day offsets over rows that were **all zero-location**, so after §23.4 it
/// would have kept passing while proving nothing — a bar written past its own defect. The
/// replacement builds the shape both ways and **prints the number of zero-location fixtures it
/// scanned**: a passing run that scanned none of them is a failing gate.
#[test]
fn era_notcloned_is_emitted_for_a_zero_location_row_and_only_for_one() {
    let mut zero_location = 0_usize;
    let mut located = 0_usize;
    for days in [0, 5, 40, 100, 400, 4000] {
        let mut bare = ProjectRow::for_test_not_cloned(1);
        bare.last_touched_at = NOW - days * DAY;
        assert_eq!(
            era_section_id_for(&bare, NOW, 0),
            "era:notcloned",
            "a project with no working copy is not filed by recency ({days} days)"
        );
        zero_location += 1;

        assert_ne!(
            era_section_id_for(&at(NOW - days * DAY), NOW, 0),
            "era:notcloned",
            "a located project never lands there ({days} days)"
        );
        located += 1;
    }
    eprintln!(
        "projects_list: scanned {zero_location} zero-location and {located} located fixtures"
    );
    assert!(
        zero_location > 0,
        "scanned {zero_location} zero-location fixtures; a run that scanned none proves nothing"
    );
    assert_eq!(zero_location, located);
}

/// AC-P2-23-4's ordering half. §23.4 tests the location **first**, before `is_archived` and
/// before `is_submodule`: `is_archived` is a *user* flag, a user may archive a not-cloned
/// project, and `era:archived` at order 92 is an **interleaved** section whose header sums
/// tracked bytes. Filing a tile with no bytes and no `Play` among tiles that have both breaks
/// the settled *never interleaved* ruling.
#[test]
fn a_not_cloned_project_is_classified_before_archived_and_before_submodules() {
    let mut archived = ProjectRow::for_test_not_cloned(1);
    archived.is_archived = true;
    assert_eq!(era_section_id_for(&archived, NOW, 0), "era:notcloned");

    let mut submodule = ProjectRow::for_test_not_cloned(2);
    submodule.is_submodule = true;
    assert_eq!(era_section_id_for(&submodule, NOW, 0), "era:notcloned");

    let mut both = ProjectRow::for_test_not_cloned(3);
    both.is_archived = true;
    both.is_submodule = true;
    assert_eq!(era_section_id_for(&both, NOW, 0), "era:notcloned");
}

/// `AC-P3-35-1` — §35.4 as corrected by **R131/F6** and **R128/F10**.
///
/// Nine cases, one mechanism: §30's pipeline decides what the reading is and `rank_of` reads it.
/// The comparator holds no second gate, so this test needs **no production change** — and the
/// discriminating half is the flip below, which a comparator carrying its own exclusion fails.
#[test]
fn ac_p3_35_1_the_exclusion_set_is_applied_once_upstream() {
    // 1 · 2 ranked · 3 frozen, ranking on its frozen value · 4 Done, which **ranks** at
    // `scored_open = 0` (R131/F6) · 5 not cloned, which ranks if it has a reading (§23.4) ·
    // 6 Reference, excluded from health entirely (§30 gate 1) · 7 archived, which **tails** on
    // the `surface_suppressed` gate that produces it (§30 gate 5) · 8 `surface_suppressed`.
    let mut reference = with_health(6, HealthState::Absent, None);
    reference.is_reference = true;
    let mut archived = with_health(7, HealthState::Suppressed, None);
    archived.is_archived = true;
    let mut done = with_health(4, HealthState::Live, Some(0));
    done.lifecycle = ProjectLifecycle::Done;
    let mut not_cloned = ProjectRow::for_test_not_cloned(5);
    not_cloned.last_touched_at = NOW - DAY;
    not_cloned.health_summary = with_health(5, HealthState::Live, Some(2)).health_summary;

    let rows: Vec<LoadedRow> = vec![
        loaded(with_health(1, HealthState::Live, Some(9))),
        loaded(with_health(2, HealthState::Live, Some(4))),
        loaded(with_health(3, HealthState::Frozen, Some(5))),
        loaded(done),
        loaded(not_cloned),
        loaded(reference),
        loaded(archived),
        loaded(with_health(8, HealthState::Suppressed, None)),
    ];
    eprintln!("projects_list: {} fixture rows", rows.len());
    assert!(
        rows.len() >= 8,
        "the fixture must hold all eight cases; it holds {}",
        rows.len()
    );

    let expected = vec![1, 3, 2, 5, 4, 6, 7, 8];
    assert_eq!(sorted_ids(&rows, SortKey::NeedsAttention), expected);

    // The discriminating half. The readings are held and the *flags* move: a comparator that
    // re-applied §35.4's exclusions moves the row, and one that applies none does not.
    let mut flipped = rows.clone();
    flipped[1].row.is_reference = true;
    flipped[1].row.is_archived = true;
    assert_eq!(
        sorted_ids(&flipped, SortKey::NeedsAttention),
        expected,
        "the comparator read a flag §35.4 forbids it to read"
    );

    // A rank may not drift as a reading ages: *presence freezes decay*, and the frozen row ranks
    // on the value it was computed with however old that reading is.
    let mut aged = rows.clone();
    aged[2].row.health_summary.observed_at = Some(NOW - 10_000 * DAY);
    assert_eq!(sorted_ids(&aged, SortKey::NeedsAttention), expected);

    // A12b: `scored_open` counts ITEMS and `unknown_checks` counts CHECKS. Neither `unverified`
    // (R128/F10) nor `unknown_checks` is an addend, a weight or a tiebreak, and a comparator that
    // quietly used either is caught here and nowhere else.
    let mut noisy = rows;
    for (index, row) in noisy.iter_mut().enumerate() {
        let n = u32::try_from(index).unwrap_or(0);
        row.row.health_summary.unverified = Some(n * 7);
        row.row.health_summary.unknown_checks = Some((9 - n) * 3);
    }
    assert_eq!(sorted_ids(&noisy, SortKey::NeedsAttention), expected);
}

/// `AC-P3-35-2` — §35.3. A project with no reading is not a project with zero debt: the tail sits
/// contiguously after every ranked row, and a **computed** zero is ranked at the bottom of the
/// ranked run rather than thrown in with the rows nobody has looked at.
#[test]
fn ac_p3_35_2_the_tail_never_interleaves_and_is_never_ordered_as_zero() {
    let rows: Vec<LoadedRow> = vec![
        loaded(with_health(1, HealthState::Live, Some(3))),
        loaded(with_health(2, HealthState::Live, Some(7))),
        loaded(with_health(3, HealthState::Live, Some(0))),
        loaded(with_health(4, HealthState::Absent, None)),
        loaded(with_health(5, HealthState::Suppressed, None)),
    ];

    let ranked = rows.iter().filter(|r| rank_of(&r.row).is_some()).count();
    let tail = rows.len() - ranked;
    eprintln!("projects_list: {ranked} ranked and {tail} tail rows in the fixture");
    assert!(ranked > 0, "a run with no ranked row proves nothing");
    assert!(tail > 0, "a run with no tail row proves nothing");

    let ids = sorted_ids(&rows, SortKey::NeedsAttention);
    // 7 then 3 then the computed zero; then the tail in the default order, which on equal
    // `last_touched_at` is id ascending.
    assert_eq!(ids, vec![2, 1, 3, 4, 5]);

    let position = |id: i64| ids.iter().position(|&x| x == id).expect("id is ordered");
    for tail_id in [4, 5] {
        for ranked_id in [1, 2, 3] {
            assert!(
                position(ranked_id) < position(tail_id),
                "row {tail_id} carries no reading and must sort after every row that does"
            );
        }
    }
    // The whole of §35.3's *a computed zero is not the tail*: above the tail, below every
    // non-zero ranked row.
    assert!(position(1) < position(3) && position(2) < position(3));
}

/// §36.2 rule 7: a cross-language agreement reads both languages. The keys are read from
/// `protocol/shelf/order-corpus.json`, which `app/src/renderer/shelf/page.test.ts` reads too, so
/// neither side holds a literal the other never sees — a literal on each side stays green while
/// one implementation and its own literal move together.
#[test]
fn the_order_key_is_the_same_cursor_the_renderer_computes() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../protocol/shelf/order-corpus.json"
    );
    let text = std::fs::read_to_string(path).expect("order-corpus.json is readable");
    let doc: serde_json::Value = serde_json::from_str(&text).expect("order-corpus.json is JSON");
    let cases = doc["cases"].as_array().expect("cases is an array");
    let mut compared = 0_usize;
    for case in cases {
        let ids: Vec<i64> = case["expectedIds"]
            .as_array()
            .expect("expectedIds")
            .iter()
            .map(|id| id.as_i64().expect("an id is an integer"))
            .collect();
        // FNV-1a over the ordered ids, little-endian, eight hex digits.
        assert_eq!(
            order_key_of(&ids),
            case["expectedOrderKey"].as_str().expect("expectedOrderKey"),
            "{}",
            case["name"]
        );
        compared += 1;
    }
    eprintln!("the order key matched the shared corpus over {compared} case(s)");
    assert!(compared > 0, "a run that compared no key proves nothing");
    assert_ne!(order_key_of(&[1, 2, 3]), order_key_of(&[3, 2, 1]));
    assert_ne!(order_key_of(&[1, 2]), order_key_of(&[1, 2, 3]));
}

/// `AC-P3-35-9` — §35.1. Rows are **ordered first and bucketed second**, and the bucket is a
/// function of the row and the clock only (§8.1). A needs-attention sort therefore reorders rows
/// *inside* sections whose top is `Live` and whose worst repositories sit in the tail.
///
/// **A later author may not section by health.** A global worst-first list of a hundred and sixty
/// forgotten repositories is the firehose the settled suppression row exists to prevent, and one
/// commit that sections on the new key rebuilds it. Without this criterion that commit is
/// reasonable.
#[test]
fn ac_p3_35_9_the_sort_does_not_re_cut_the_shelf() {
    let (_dir, index) = seeded();
    let sink = CollectingSink::default();
    let jobs = codotheca_core::jobs::NullJobSink;
    let mounts = codotheca_core::testing::FakeMountResolver::new();
    let ctx = ProjectsCtx {
        index: &index,
        events: &sink,
        jobs: &jobs,
        mounts: &mounts,
        sync: &codotheca_core::sync::runner::NullSyncSink,
        now: NOW,
        tz_offset_min: 0,
    };

    let mut archived = with_health(4, HealthState::Live, Some(3));
    archived.is_archived = true;
    let rows: Vec<LoadedRow> = vec![
        loaded(with_health(1, HealthState::Live, Some(9))),
        loaded(touched(with_health(2, HealthState::Live, Some(1)), 400)),
        loaded(touched(with_health(3, HealthState::Live, Some(7)), 4400)),
        loaded(archived),
        loaded(touched(with_health(5, HealthState::Absent, None), 60)),
    ];
    // The §8.1 answer, written out rather than recomputed from the function under test — a
    // bucketing that read the reading would agree with itself across every sort key.
    let expected: Vec<(i64, &str)> = vec![
        (1, "era:live"),
        (2, "era:2025"),
        (3, "era:tail"),
        (4, "era:archived"),
        (5, "era:q"),
    ];

    let variants = schema_sort_variants();
    eprintln!(
        "projects_list: {} rows over {} SortKey variant(s)",
        rows.len(),
        variants.len()
    );
    assert!(!rows.is_empty(), "a run over no row proves nothing");
    assert!(!variants.is_empty(), "a run over no variant proves nothing");

    let ast = codotheca_core::query::parse_query("");
    let names = std::collections::BTreeMap::new();
    let exec = codotheca_core::query::execute::ExecContext {
        now: NOW,
        tz_offset_min: 0,
        first_run_completed_at: None,
        collection_ids_by_name: &names,
        paths_are_case_sensitive: cfg!(not(windows)),
        commit_subject_hits: None,
    };
    let page_for = |sort: SortKey| {
        codotheca_core::projects::list::build_project_page(&ctx, &rows, &ast, sort, None, 1, &exec)
    };

    let baseline = page_for(SortKey::LastTouched);
    let sections_of = |page: &codotheca_core::protocol::ProjectPage| {
        page.sections
            .iter()
            .map(|s| s.id.clone())
            .collect::<Vec<_>>()
    };
    let buckets_of = |page: &codotheca_core::protocol::ProjectPage| {
        let mut pairs = page
            .rows
            .iter()
            .map(|r| (r.id.0, r.era_section_id.clone()))
            .collect::<Vec<_>>();
        pairs.sort_unstable();
        pairs
    };
    assert_eq!(
        buckets_of(&baseline),
        expected
            .iter()
            .map(|(id, era)| (*id, (*era).to_owned()))
            .collect::<Vec<_>>()
    );

    for variant in &variants {
        let sort: SortKey = serde_json::from_value(serde_json::Value::String(variant.clone()))
            .expect("a SortKey variant");
        let page = page_for(sort);
        assert_eq!(
            buckets_of(&page),
            buckets_of(&baseline),
            "{variant} re-cut the shelf"
        );
        // A sort that reordered the sections without re-bucketing the rows would pass the
        // assertion above on its own.
        assert_eq!(
            sections_of(&page),
            sections_of(&baseline),
            "{variant} reordered the sections"
        );
    }

    // And the key really does reorder rows across section boundaries, or the assertions above
    // hold over a fixture that was never going to move.
    let ids = |page: &codotheca_core::protocol::ProjectPage| {
        page.rows.iter().map(|r| r.id.0).collect::<Vec<_>>()
    };
    assert_ne!(
        ids(&page_for(SortKey::NeedsAttention)),
        ids(&baseline),
        "the fixture does not reorder under the new key, so it proves nothing"
    );
}

/// Two **located** projects. The location rows are not decoration: §23.4 classifies on
/// `primary_location IS NULL` **first**, so a seed with no `location` files both rows under
/// `era:notcloned` and every section assertion below then describes one section instead of two.
fn seeded() -> (tempfile::TempDir, Index) {
    let dir = tempfile::tempdir().expect("tempdir");
    let index = Index::open(dir.path()).expect("open");
    index
        .conn()
        .execute(
            "INSERT INTO project (id, name, seed_basename, last_touched_at, created_at, updated_at)
             VALUES (1, 'a', 'a', ?1, ?1, ?1), (2, 'b', 'b', ?2, ?2, ?2)",
            rusqlite::params![NOW - DAY, NOW - 400 * DAY],
        )
        .expect("seed");
    index
        .conn()
        .execute(
            "INSERT INTO location (id, project_id, kind, distro, path_bytes, path_key,
                                   path_display, volume_key, store_key, presence, repo_kind)
             VALUES (10, 1, 'linux', '', x'2f612f61', x'2f612f61', '/a/a', 'v', 's', 'present',
                     'worktree'),
                    (20, 2, 'linux', '', x'2f622f62', x'2f622f62', '/b/b', 'v', 's', 'present',
                     'worktree')",
            [],
        )
        .expect("seed locations");
    (dir, index)
}

fn list(index: &Index, sink: &CollectingSink, args: serde_json::Value) -> serde_json::Value {
    // §6: the shelf does not ask for freshness, so a null sink here is not a stand-in — it is
    // the assertion that `projects.list` queues nothing, made structurally.
    let jobs = codotheca_core::jobs::NullJobSink;
    let mounts = codotheca_core::testing::FakeMountResolver::new();
    let ctx = ProjectsCtx {
        index,
        events: sink,
        jobs: &jobs,
        mounts: &mounts,
        sync: &codotheca_core::sync::runner::NullSyncSink,
        now: NOW,
        tz_offset_min: 0,
    };
    dispatch_projects_command(&ctx, "projects.list", args)
        .expect("owned")
        .expect("ok")
}

#[test]
fn the_page_is_the_generated_wire_type_and_carries_counts_before_rows() {
    let (_dir, index) = seeded();
    let sink = CollectingSink::default();
    let page = list(&index, &sink, serde_json::json!({}));

    assert_eq!(page["sections"][0]["id"], "era:live");
    assert_eq!(page["sections"][1]["id"], "era:2025");
    assert_eq!(page["sections"][0]["count"], 1);
    // §8.1: labels are shell-owned prose; the core emits the id, the order and the cut year.
    assert!(page["sections"][0].get("label").is_none());
    assert_eq!(page["sections"][1]["year"], 2025);
    assert_eq!(page["sections"][0]["cutAgainstYear"], 2026);
    assert_eq!(page["window"], serde_json::json!({ "from": 0, "to": 2 }));
    assert!(page["orderKey"].is_string());
}

#[test]
fn empty_args_are_valid_because_the_projects_snapshot_sends_exactly_that() {
    let (_dir, index) = seeded();
    let sink = CollectingSink::default();
    // This call is what finally replaces the `projects` topic's `Value::Null`, so `{}` must
    // deserialise rather than becoming a PROTOCOL failure.
    let page = list(&index, &sink, serde_json::json!({}));
    assert_eq!(page["rows"].as_array().expect("rows").len(), 2);
}

#[test]
fn a_window_pages_the_rendered_rows_and_is_clamped_not_wrapped() {
    let (_dir, index) = seeded();
    let sink = CollectingSink::default();
    let page = list(
        &index,
        &sink,
        serde_json::json!({ "window": { "from": 1, "to": 99 } }),
    );
    assert_eq!(page["window"], serde_json::json!({ "from": 1, "to": 2 }));
    assert_eq!(page["rows"].as_array().expect("rows").len(), 1);
    // Sections and the order key describe the whole matched set, not the window.
    assert_eq!(page["sections"].as_array().expect("sections").len(), 2);
    assert_eq!(
        page["orderKey"],
        list(&index, &sink, serde_json::json!({}))["orderKey"]
    );
}

#[test]
fn the_aggregate_carries_coverage_and_emits_it_even_when_fully_covered() {
    let (_dir, index) = seeded();
    index
        .conn()
        .execute("UPDATE project SET size_tracked_bytes = 1024", [])
        .expect("inventory");
    let sink = CollectingSink::default();
    let page = list(&index, &sink, serde_json::json!({}));
    let agg = &page["sections"][0]["agg"];
    // §8.2: both are emitted always. The shell drops the parenthetical on the values, and
    // never infers coverage from a missing field.
    assert_eq!(agg["indexedCount"], 1);
    assert_eq!(agg["unchecked"], 1); // no J1 result yet — an absent flag line would lie
    assert_eq!(agg["trackedBytes"], 1024);
}

#[test]
fn no_row_and_no_section_carries_a_roast_line_or_any_sentence() {
    let (_dir, index) = seeded();
    let sink = CollectingSink::default();
    let page = list(&index, &sink, serde_json::json!({}));
    // §5.6: roasting appears only inside an opened project card — never the grid, never triage.
    let text = serde_json::to_string(&page).expect("serialise");
    for banned in ["roast", "note", "Nothing outstanding"] {
        assert!(!text.contains(banned), "{banned} must not reach the shelf");
    }
}

#[test]
fn every_row_carries_an_uncomputed_completion_because_that_is_the_only_case() {
    let (_dir, index) = seeded();
    let sink = CollectingSink::default();
    let page = list(&index, &sink, serde_json::json!({}));
    for row in page["rows"].as_array().expect("rows") {
        assert_eq!(row["completionLit"], serde_json::Value::Null);
        assert_eq!(row["completionApplicable"], serde_json::Value::Null);
        assert!(row["eraSectionId"]
            .as_str()
            .is_some_and(|s| s.starts_with("era:")));
    }
}

#[test]
fn a_query_that_matches_nothing_returns_no_rows_and_no_sections_rather_than_a_zero() {
    let (_dir, index) = seeded();
    let sink = CollectingSink::default();
    // Nothing has been observed, so `is:dirty` is unknown everywhere and matches nobody.
    let page = list(&index, &sink, serde_json::json!({ "query": "is:dirty" }));
    assert_eq!(page["rows"].as_array().expect("rows").len(), 0);
    assert_eq!(page["sections"].as_array().expect("sections").len(), 0);
    assert_eq!(page["window"], serde_json::json!({ "from": 0, "to": 0 }));
}

#[test]
fn sort_by_name_reorders_the_rows_and_the_order_key_with_them() {
    let (_dir, index) = seeded();
    let sink = CollectingSink::default();
    let touched = list(&index, &sink, serde_json::json!({}));
    let by_name = list(&index, &sink, serde_json::json!({ "sort": "name" }));
    let ids = |p: &serde_json::Value| {
        p["rows"]
            .as_array()
            .expect("rows")
            .iter()
            .map(|r| r["id"].as_i64().expect("id"))
            .collect::<Vec<_>>()
    };
    assert_eq!(ids(&touched), vec![1, 2]);
    assert_eq!(ids(&by_name), vec![1, 2]);
    // A row with no inventory sorts last under SIZE: it is not small, it is unmeasured.
    index
        .conn()
        .execute(
            "UPDATE project SET size_tracked_bytes = 10 WHERE id = 2",
            [],
        )
        .expect("inventory");
    let by_size = list(&index, &sink, serde_json::json!({ "sort": "size" }));
    assert_eq!(ids(&by_size), vec![2, 1]);
}

/// AC-P2-23-5's second half, Rust mirror. `unchecked` exists so that *an absent flag line may
/// only ever mean observed, and nothing to report* (§8.1) — it is a coverage warning about
/// **local git state**, and a not-cloned project has no local git state to be uncovered about.
#[test]
fn unchecked_counts_only_rows_that_have_a_working_copy() {
    use codotheca_core::projects::list::aggregate_era;

    let bare: Vec<LoadedRow> = (1..=3)
        .map(|id| loaded(ProjectRow::for_test_not_cloned(id)))
        .collect();
    let bare_refs: Vec<&LoadedRow> = bare.iter().collect();
    let agg = aggregate_era(&bare_refs);
    assert_eq!(agg.unchecked, 0, "there is no local git state to cover");
    // The three remaining counters need no change, and this asserts that rather than assuming
    // it: all three are false on a NULL row, so with all four at zero no flag line renders.
    assert_eq!(agg.unpushed, 0);
    assert_eq!(agg.uncommitted, 0);
    assert_eq!(agg.interrupted, 0);

    let mixed: Vec<LoadedRow> = vec![
        loaded(ProjectRow::for_test(1)),
        loaded(ProjectRow::for_test(2)),
        loaded(ProjectRow::for_test_not_cloned(3)),
        loaded(ProjectRow::for_test_not_cloned(4)),
    ];
    let mixed_refs: Vec<&LoadedRow> = mixed.iter().collect();
    assert_eq!(aggregate_era(&mixed_refs).unchecked, 2);

    // A located section with no J1 result still counts every one of its rows.
    let located: Vec<LoadedRow> = (1..=3).map(|id| loaded(ProjectRow::for_test(id))).collect();
    let located_refs: Vec<&LoadedRow> = located.iter().collect();
    assert_eq!(aggregate_era(&located_refs).unchecked, 3);
}
