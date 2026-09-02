//! §5.1's multi-location fold, and every NULL that must survive the trip to the wire.
//!
//! Compiled only under `testkit`: `art::testsupport::CollectingSink` is gated there, so without
//! the feature this fails to compile rather than skipping silently.
#![cfg(feature = "testkit")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use codotheca_core::art::testsupport::CollectingSink;
use codotheca_core::index::Index;
use codotheca_core::projects::rows::{
    any_present_dirty, enum_from_column, load_project_rows, locations_of, pick_primary,
};
use codotheca_core::projects::ProjectsCtx;
use codotheca_core::protocol::{
    ArtState, ConditionSignal, DescriptionSource, LocationKind, Presence, ProjectId,
};

const NOW: i64 = 1_781_000_000; // mid-2026

fn project(conn: &rusqlite::Connection, id: i64, name: &str, touched: i64) {
    conn.execute(
        "INSERT INTO project (id, name, seed_basename, last_touched_at, created_at, updated_at)
         VALUES (?1, ?2, ?2, ?3, ?3, ?3)",
        rusqlite::params![id, name, touched],
    )
    .expect("insert project");
}

/// One `location` row. `repo_kind` is NOT NULL with no default in the DDL, so every insert has
/// to name it; the observed columns stay NULL unless a test sets them.
struct Loc {
    id: i64,
    path: &'static str,
    presence: &'static str,
    branch: Option<&'static str>,
    is_dirty: Option<i64>,
    ahead: Option<i64>,
    worktree_observed_at: Option<i64>,
    worktree_newest_mtime: Option<i64>,
    last_seen_at: Option<i64>,
}

impl Loc {
    fn new(id: i64, path: &'static str) -> Self {
        Self {
            id,
            path,
            presence: "present",
            branch: None,
            is_dirty: None,
            ahead: None,
            worktree_observed_at: None,
            worktree_newest_mtime: None,
            last_seen_at: None,
        }
    }
}

fn location(conn: &rusqlite::Connection, project_id: i64, loc: &Loc) {
    conn.execute(
        "INSERT INTO location (id, project_id, kind, distro, path_bytes, path_key, path_display,
                               volume_key, store_key, presence, repo_kind, branch, is_dirty,
                               ahead, worktree_observed_at, worktree_newest_mtime, last_seen_at)
         VALUES (?1, ?2, 'linux', '', ?3, ?3, ?4, 'v', 's', ?5, 'worktree', ?6, ?7, ?8, ?9,
                 ?10, ?11)",
        rusqlite::params![
            loc.id,
            project_id,
            loc.path.as_bytes(),
            loc.path,
            loc.presence,
            loc.branch,
            loc.is_dirty,
            loc.ahead,
            loc.worktree_observed_at,
            loc.worktree_newest_mtime,
            loc.last_seen_at,
        ],
    )
    .expect("insert location");
}

/// §6's two collaborators, which the projection itself never uses: `load_project_rows` is the
/// shelf's read and the shelf does **not** ask for freshness — a thousand rows would queue a
/// thousand status jobs on every keystroke.
struct Deps {
    jobs: codotheca_core::jobs::NullJobSink,
    mounts: codotheca_core::testing::FakeMountResolver,
}

impl Deps {
    fn new() -> Deps {
        Deps {
            jobs: codotheca_core::jobs::NullJobSink,
            mounts: codotheca_core::testing::FakeMountResolver::new(),
        }
    }
}

fn ctx<'a>(index: &'a Index, sink: &'a CollectingSink, deps: &'a Deps) -> ProjectsCtx<'a> {
    ProjectsCtx {
        index,
        events: sink,
        jobs: &deps.jobs,
        mounts: &deps.mounts,
        now: NOW,
        tz_offset_min: 0,
    }
}

fn opened() -> (tempfile::TempDir, Index) {
    let dir = tempfile::tempdir().expect("tempdir");
    let index = Index::open(dir.path()).expect("open");
    (dir, index)
}

#[test]
fn a_never_observed_worktree_stays_null_on_the_wire_and_never_false() {
    let (_dir, index) = opened();
    project(index.conn(), 1, "p", NOW);
    location(index.conn(), 1, &Loc::new(10, "/a"));
    let sink = CollectingSink::default();
    let rows = load_project_rows(&ctx(&index, &sink, &Deps::new())).expect("load");

    assert_eq!(rows.len(), 1);
    // §6: absence of dirty means "no changes as of T", never "clean".
    assert_eq!(rows[0].row.is_dirty, None);
    assert_eq!(rows[0].row.worktree_observed_at, None);
    // §1.3: NULL means no fetch has ever been recorded here — never `0`, never an age.
    assert_eq!(rows[0].row.fetch_head_at, None);
    // §7.7a: in phase 1 nothing writes completion, so this is the only case, not an edge case.
    assert_eq!(rows[0].row.completion_lit, None);
    assert_eq!(rows[0].row.completion_applicable, None);
    // And the serialised form carries nulls, not zeroes.
    let wire = serde_json::to_value(&rows[0].row).expect("serialise");
    for key in [
        "isDirty",
        "fetchHeadAt",
        "completionLit",
        "ahead",
        "behind",
        "sizeTrackedBytes",
    ] {
        assert_eq!(
            wire[key],
            serde_json::Value::Null,
            "{key} must cross as null"
        );
    }
}

#[test]
fn dirty_is_the_or_across_present_locations_and_null_only_when_none_was_observed() {
    let (_dir, index) = opened();
    project(index.conn(), 1, "p", NOW);
    location(
        index.conn(),
        1,
        &Loc {
            is_dirty: Some(0),
            worktree_observed_at: Some(NOW),
            ..Loc::new(10, "/a")
        },
    );
    location(
        index.conn(),
        1,
        &Loc {
            is_dirty: Some(1),
            worktree_observed_at: Some(NOW),
            ..Loc::new(11, "/b")
        },
    );
    let sink = CollectingSink::default();
    let rows = load_project_rows(&ctx(&index, &sink, &Deps::new())).expect("load");
    // §5.1: any present location dirty means the project is dirty.
    assert_eq!(rows[0].row.is_dirty, Some(true));

    // One observed clean, one never looked at: the observation stands, the answer is false.
    let (_dir2, index2) = opened();
    project(index2.conn(), 1, "p", NOW);
    location(
        index2.conn(),
        1,
        &Loc {
            is_dirty: Some(0),
            worktree_observed_at: Some(NOW),
            ..Loc::new(10, "/a")
        },
    );
    location(index2.conn(), 1, &Loc::new(11, "/b"));
    let rows2 = load_project_rows(&ctx(&index2, &sink, &Deps::new())).expect("load");
    assert_eq!(rows2[0].row.is_dirty, Some(false));
}

#[test]
fn branch_and_ahead_come_from_the_primary_location_not_from_the_or() {
    let (_dir, index) = opened();
    project(index.conn(), 1, "p", NOW);
    // 10 is offline with work on it; 11 is present. §5.1 makes 11 primary.
    location(
        index.conn(),
        1,
        &Loc {
            presence: "offline",
            branch: Some("old"),
            ahead: Some(9),
            last_seen_at: Some(5),
            worktree_newest_mtime: Some(NOW),
            ..Loc::new(10, "/a")
        },
    );
    location(
        index.conn(),
        1,
        &Loc {
            branch: Some("main"),
            ahead: Some(2),
            last_seen_at: Some(9),
            ..Loc::new(11, "/b")
        },
    );
    let sink = CollectingSink::default();
    let rows = load_project_rows(&ctx(&index, &sink, &Deps::new())).expect("load");
    assert_eq!(rows[0].row.branch.as_deref(), Some("main"));
    assert_eq!(rows[0].row.ahead, Some(2));
    assert_eq!(
        rows[0]
            .row
            .primary_location
            .as_ref()
            .map(|l| l.path_display.as_str()),
        Some("/b")
    );
    // An offline copy is still named, and its presence is reported rather than `unscanned`.
    assert_eq!(rows[0].row.presence, Presence::Present);
}

#[test]
fn an_all_offline_project_still_names_a_copy_and_reports_offline() {
    // `derive::aggregate` filters to present copies and answers `None`; the wire row cannot,
    // because §11.1 draws an offline tile and a `presence: unscanned` there would be a lie.
    let (_dir, index) = opened();
    project(index.conn(), 1, "p", NOW);
    location(
        index.conn(),
        1,
        &Loc {
            presence: "offline",
            ..Loc::new(10, "/a")
        },
    );
    let sink = CollectingSink::default();
    let rows = load_project_rows(&ctx(&index, &sink, &Deps::new())).expect("load");
    assert_eq!(rows[0].row.presence, Presence::Offline);
    assert!(rows[0].row.primary_location.is_some());
}

#[test]
fn a_merged_project_is_never_listed() {
    let (_dir, index) = opened();
    project(index.conn(), 1, "survivor", NOW);
    project(index.conn(), 2, "absorbed", NOW);
    index
        .conn()
        .execute("UPDATE project SET merged_into = 1 WHERE id = 2", [])
        .expect("merge");
    let sink = CollectingSink::default();
    let rows = load_project_rows(&ctx(&index, &sink, &Deps::new())).expect("load");
    assert_eq!(
        rows.iter().map(|r| r.row.id.0).collect::<Vec<_>>(),
        vec![1_i64]
    );
}

#[test]
fn the_three_valued_or_and_the_primary_rule_are_pure_and_testable_alone() {
    assert_eq!(any_present_dirty(&[]), None);
    assert!(pick_primary(&[]).is_none());
}

/// The primary copy is chosen by **one** rule, and `core::derive` already owns it.
///
/// R41 recorded that plan 13 used `last_seen_at` because "§5.1 gives no `touched` column on
/// `location`". Migration `0007_jobs_derived.sql` added `location.worktree_newest_mtime`, so
/// that premise no longer holds and the two sides can — and must — agree. This reads both
/// implementations off one database rather than restating either one's table.
#[test]
fn the_primary_copy_is_the_same_one_core_derive_picks() {
    let (_dir, index) = opened();
    project(index.conn(), 1, "p", NOW);
    for (id, path, mtime) in [(10, "/a", 100), (11, "/b", 900), (12, "/c", 400)] {
        location(
            index.conn(),
            1,
            &Loc {
                worktree_newest_mtime: Some(mtime),
                last_seen_at: Some(1_000 - mtime), // deliberately the opposite order
                ..Loc::new(id, path)
            },
        );
    }
    let via_derive = codotheca_core::derive::aggregate(
        &codotheca_core::derive::persist::load_location_facts(index.conn(), ProjectId(1))
            .expect("facts"),
        None,
        None,
    )
    .primary_location;
    let via_rows =
        pick_primary(&locations_of(index.conn(), ProjectId(1)).expect("locations")).map(|l| l.id);
    assert_eq!(via_rows, via_derive);
    assert_eq!(via_rows.map(|id| id.0), Some(11));
}

/// A stored slug and a serde rename are the same value stated twice (R26's shape). `rows.rs`
/// reads every TEXT enum through the generated renames, so this pins them to the slugs the
/// existing owners write.
#[test]
fn the_stored_slugs_and_the_generated_renames_are_the_same_words() {
    for signal in ConditionSignal::ALL {
        assert_eq!(
            enum_from_column::<ConditionSignal>(signal.slug()),
            Some(signal)
        );
    }
    for state in [
        ArtState::Pending,
        ArtState::Ready,
        ArtState::Failed,
        ArtState::Stale,
    ] {
        assert_eq!(
            enum_from_column::<ArtState>(codotheca_core::art::store::state_slug(state)),
            Some(state)
        );
    }
    for source in [
        DescriptionSource::Manifest,
        DescriptionSource::Readme,
        DescriptionSource::Note,
        DescriptionSource::Detected,
    ] {
        assert_eq!(
            enum_from_column::<DescriptionSource>(source.slug()),
            Some(source)
        );
    }
    for presence in [
        Presence::Present,
        Presence::Offline,
        Presence::Missing,
        Presence::Unscanned,
    ] {
        assert_eq!(
            enum_from_column::<Presence>(presence.as_str()),
            Some(presence)
        );
    }
    for kind in LocationKind::ALL {
        assert_eq!(enum_from_column::<LocationKind>(kind.as_str()), Some(kind));
    }
    // A word this build does not know is not guessed at.
    assert_eq!(enum_from_column::<Presence>("teleported"), None);
}

/// §7.3a derives the jewel from `seedBasename` and `rerollOffset`, so both must cross as
/// stored. Plan 12b added them to the wire on one branch while this loader was written on
/// another; a merge that compiles is not a merge that carries the values.
#[test]
fn the_two_fields_the_jewel_is_derived_from_cross_as_stored_and_not_as_defaults() {
    let (_dir, index) = opened();
    project(index.conn(), 1, "p", NOW);
    // A basename that differs from the name, and an offset that is not the column default,
    // so neither can pass by accident.
    index
        .conn()
        .execute(
            "UPDATE project SET seed_basename = 'other-basename', reroll_offset = 3 WHERE id = 1",
            [],
        )
        .expect("set seed and offset");
    location(index.conn(), 1, &Loc::new(10, "/a"));
    let sink = CollectingSink::default();
    let rows = load_project_rows(&ctx(&index, &sink, &Deps::new())).expect("load");

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].row.seed_basename, "other-basename");
    assert_eq!(rows[0].row.reroll_offset, 3);
    let wire = serde_json::to_value(&rows[0].row).expect("serialise");
    assert_eq!(wire["seedBasename"], "other-basename");
    assert_eq!(wire["rerollOffset"], 3);
}
