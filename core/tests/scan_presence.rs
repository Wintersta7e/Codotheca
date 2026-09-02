//! Compiled only under `testkit`: these link `codotheca_core::testing`, which the feature gates.
#![cfg(feature = "testkit")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

//! §4.6 — presence, and the generation rule v1 omitted. Without it nothing ever left `present`,
//! `last_seen_at` was written and never read, and acceptance criterion 4 could not pass.

use codotheca_core::index::path::PathPlatform;
use codotheca_core::paths::{path_bytes, path_key};
use codotheca_core::scan::presence::{
    apply_presence, classify_presence, mark_root_unscanned, project_presence, LocationPresenceRow,
    Presence, PresenceContext, PresenceSummary, ScanRootRow, ScanStore,
};
use codotheca_core::scan::skiplist::SkipList;
use codotheca_core::testing::MemScanStore;
use std::collections::BTreeSet;
use std::path::Path;

// R2: these rows describe `linux` roots, so their keys are `PathPlatform::Unix` whatever host
// the test runs on. Under a `#[cfg(windows)]` form the same fixtures would fold case on Windows
// and not on Linux, so the suite would be testing two different rules.
fn root(id: i64, path: &str, enabled: bool) -> ScanRootRow {
    ScanRootRow {
        root_id: id,
        kind: "linux".to_owned(),
        distro: String::new(),
        path_bytes: path_bytes(Path::new(path)),
        path_key: path_key(Path::new(path), PathPlatform::Unix),
        enabled,
        descend_into_repos: false,
    }
}

fn loc(id: i64, project: i64, path: &str, store: &str, generation: i64) -> LocationPresenceRow {
    LocationPresenceRow {
        location_id: id,
        project_id: project,
        path_bytes: path_bytes(Path::new(path)),
        path_key: path_key(Path::new(path), PathPlatform::Unix),
        store_key: store.to_owned(),
        scan_generation: generation,
        presence: Presence::Present,
    }
}

fn stores(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|s| (*s).to_owned()).collect()
}

#[test]
fn presence_round_trips_through_the_text_the_column_stores() {
    for presence in [
        Presence::Present,
        Presence::Offline,
        Presence::Missing,
        Presence::Unscanned,
    ] {
        assert_eq!(Presence::parse(presence.as_str()), Some(presence));
        assert_eq!(
            serde_json::to_string(&presence).unwrap(),
            format!("\"{}\"", presence.as_str()),
            "the wire spelling and the column spelling are one value"
        );
    }
    assert_eq!(Presence::parse("gone"), None);
}

#[test]
fn seen_this_generation_is_present() {
    let roots = [root(1, "/r", true)];
    let skip = SkipList::default();
    let present = stores(&["s1"]);
    let ctx = PresenceContext {
        generation: 7,
        roots: &roots,
        skip: &skip,
        present_stores: &present,
    };
    assert_eq!(
        classify_presence(&loc(1, 1, "/r/a", "s1", 7), &ctx),
        Presence::Present
    );
}

#[test]
fn not_seen_on_a_present_store_is_missing_and_on_an_absent_store_is_offline() {
    let roots = [root(1, "/r", true)];
    let skip = SkipList::default();
    let present = stores(&["s1"]);
    let ctx = PresenceContext {
        generation: 7,
        roots: &roots,
        skip: &skip,
        present_stores: &present,
    };
    assert_eq!(
        classify_presence(&loc(1, 1, "/r/a", "s1", 6), &ctx),
        Presence::Missing
    );
    assert_eq!(
        classify_presence(&loc(2, 1, "/r/b", "usb", 6), &ctx),
        Presence::Offline
    );
}

#[test]
fn a_disabled_root_makes_its_locations_unscanned_not_missing() {
    let roots = [root(1, "/r", false)];
    let skip = SkipList::default();
    let present = stores(&["s1"]);
    let ctx = PresenceContext {
        generation: 7,
        roots: &roots,
        skip: &skip,
        present_stores: &present,
    };
    assert_eq!(
        classify_presence(&loc(1, 1, "/r/a", "s1", 6), &ctx),
        Presence::Unscanned
    );
    // Even one seen this generation: the root is off, so it is not verified.
    assert_eq!(
        classify_presence(&loc(2, 1, "/r/b", "s1", 7), &ctx),
        Presence::Unscanned
    );
}

#[test]
fn a_newly_excluded_directory_becomes_unscanned() {
    let roots = [root(1, "/r", true)];
    let skip = SkipList::with_user_entries(&["Archive".to_owned()]);
    let present = stores(&["s1"]);
    let ctx = PresenceContext {
        generation: 7,
        roots: &roots,
        skip: &skip,
        present_stores: &present,
    };
    assert_eq!(
        classify_presence(&loc(1, 1, "/r/Archive/old", "s1", 6), &ctx),
        Presence::Unscanned
    );
}

#[test]
fn a_project_is_offline_only_when_every_location_is() {
    assert_eq!(
        project_presence(&[Presence::Offline, Presence::Offline]),
        Presence::Offline
    );
    assert_eq!(
        project_presence(&[Presence::Offline, Presence::Present]),
        Presence::Present
    );
    assert_eq!(
        project_presence(&[Presence::Offline, Presence::Missing]),
        Presence::Missing
    );
    assert_eq!(
        project_presence(&[Presence::Unscanned]),
        Presence::Unscanned
    );
    assert_eq!(project_presence(&[]), Presence::Unscanned);
}

#[test]
fn unplugging_a_volume_moves_locations_offline_and_deletes_nothing() {
    // Acceptance criterion 4, in miniature.
    let store = MemScanStore::new();
    store.push_root(root(1, "/r", true));
    store.push_location(loc(1, 1, "/r/a", "internal", 7));
    store.push_location(loc(2, 2, "/r/removable/p", "usb", 7));
    let before = store.location_count();

    let roots = store.scan_roots().unwrap();
    let skip = SkipList::default();
    let present = stores(&["internal"]); // the usb store is gone
    let ctx = PresenceContext {
        generation: 8,
        roots: &roots,
        skip: &skip,
        present_stores: &present,
    };
    let summary = apply_presence(&store, &ctx).unwrap();

    assert_eq!(
        store.location_count(),
        before,
        "presence never deletes a row"
    );
    assert_eq!(store.presence_of(1), Some(Presence::Missing));
    assert_eq!(store.presence_of(2), Some(Presence::Offline));
    assert_eq!(summary.offline, 1);
    assert_eq!(summary.missing, 1);
    assert_eq!(summary.offline_projects, 1);
    assert_eq!(summary.changed, 2);

    // Re-plugging and re-walking restores present.
    store.set_generation_of(2, 9);
    store.set_generation_of(1, 9);
    let present = stores(&["internal", "usb"]);
    let ctx = PresenceContext {
        generation: 9,
        roots: &roots,
        skip: &skip,
        present_stores: &present,
    };
    let summary = apply_presence(&store, &ctx).unwrap();
    assert_eq!(store.presence_of(1), Some(Presence::Present));
    assert_eq!(store.presence_of(2), Some(Presence::Present));
    assert_eq!(summary.offline_projects, 0);
    assert_eq!(store.location_count(), before);
}

/// §4.6: disabling a root marks its locations `unscanned` — neither gone nor verified — at the
/// moment `roots.setEnabled` runs, without waiting for the next scan.
#[test]
fn disabling_a_root_marks_its_locations_unscanned_immediately() {
    let store = MemScanStore::new();
    let disabled = root(1, "/r", false);
    store.push_location(loc(1, 1, "/r/a", "internal", 7));
    store.push_location(loc(2, 2, "/elsewhere/b", "internal", 7));

    let changed = mark_root_unscanned(&store, &disabled).unwrap();
    assert_eq!(changed, 1);
    assert_eq!(store.presence_of(1), Some(Presence::Unscanned));
    assert_eq!(
        store.presence_of(2),
        Some(Presence::Present),
        "a location under a different root is untouched"
    );
    // Idempotent: a second call rewrites nothing.
    assert_eq!(mark_root_unscanned(&store, &disabled).unwrap(), 0);
}

/// An empty summary is the shape a cancelled run reports, so it has to be distinguishable from
/// a run that classified everything as present.
#[test]
fn the_default_summary_counts_nothing_at_all() {
    let empty = PresenceSummary::default();
    assert_eq!(empty.present, 0);
    assert_eq!(empty.changed, 0);
    assert_eq!(empty.offline_projects, 0);
}
