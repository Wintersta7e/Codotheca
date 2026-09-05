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
    era_section_id_for, era_section_order, order_key_of, ERA_NAMED_YEARS,
};
use codotheca_core::projects::{dispatch_projects_command, ProjectsCtx};
use codotheca_core::protocol::ProjectRow;

/// 2026-06-11T12:00:00Z, so the cut year is 2026 and the ten named years run 2025 down to 2016.
const NOW: i64 = 1_781_179_200;
const DAY: i64 = 86_400;

fn at(secs: i64) -> ProjectRow {
    let mut row = ProjectRow::for_test(1);
    row.last_touched_at = secs;
    row
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

#[test]
fn era_notcloned_is_never_emitted_in_phase_one() {
    for days in [0, 5, 40, 100, 400, 4000] {
        assert_ne!(
            era_section_id_for(&at(NOW - days * DAY), NOW, 0),
            "era:notcloned"
        );
    }
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

#[test]
fn the_order_key_is_the_same_cursor_the_renderer_computes() {
    // FNV-1a over the ordered ids, little-endian, eight hex digits.
    assert_eq!(order_key_of(&[1, 2, 3]), "794671b5");
    assert_ne!(order_key_of(&[1, 2, 3]), order_key_of(&[3, 2, 1]));
    assert_ne!(order_key_of(&[1, 2]), order_key_of(&[1, 2, 3]));
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
