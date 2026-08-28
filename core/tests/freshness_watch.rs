//! The watch set against a real platform watcher.
//!
//! `select_targets` is pure and unit-tested beside itself; `WatchSet` is the half that talks to
//! inotify / `ReadDirectoryChangesW`, and a type introduced for a seam and never exercised is
//! the defect shape this project keeps finding. So this file drives the real thing.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::must_use_candidate,
    clippy::missing_panics_doc
)]

use std::path::PathBuf;
use std::time::{Duration, Instant};

use codotheca_core::freshness::watch::{WatchSet, WatchTarget};
use codotheca_core::protocol::LocationId;

/// Poll until the watcher reports something or the deadline passes. Platform watchers are
/// asynchronous; a bare `take_invalidations` immediately after a write races them.
fn drain_until(set: &mut WatchSet, deadline: Duration) -> Vec<LocationId> {
    let start = Instant::now();
    loop {
        let hit = set.take_invalidations();
        if !hit.is_empty() || start.elapsed() >= deadline {
            return hit;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[test]
fn a_write_under_a_watched_root_invalidates_that_location() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("worktree");
    std::fs::create_dir_all(root.join("src")).unwrap();

    let mut set = WatchSet::new().unwrap();
    set.retarget(&[WatchTarget {
        location_id: LocationId(7),
        root: root.clone(),
        pinned: false,
    }])
    .unwrap();

    std::fs::write(root.join("src").join("a.rs"), b"pub fn a() {}\n").unwrap();

    let hit = drain_until(&mut set, Duration::from_secs(10));
    assert_eq!(
        hit,
        vec![LocationId(7)],
        "a write under a watched worktree root must invalidate its location"
    );
}

#[test]
fn retargeting_drops_the_roots_that_are_no_longer_wanted() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a");
    let b = dir.path().join("b");
    std::fs::create_dir_all(&a).unwrap();
    std::fs::create_dir_all(&b).unwrap();

    let target = |id: i64, root: &PathBuf| WatchTarget {
        location_id: LocationId(id),
        root: root.clone(),
        pinned: false,
    };

    let mut set = WatchSet::new().unwrap();
    set.retarget(&[target(1, &a), target(2, &b)]).unwrap();
    assert_eq!(set.watched().len(), 2);

    set.retarget(&[target(2, &b)]).unwrap();
    assert_eq!(set.watched().keys().collect::<Vec<_>>(), vec![&b]);

    // Whatever the dropped root emitted before or during the unwatch is not this location's.
    let _ = set.take_invalidations();
    std::fs::write(a.join("x"), b"x").unwrap();
    std::thread::sleep(Duration::from_millis(200));
    assert!(!set.take_invalidations().contains(&LocationId(1)));
}

#[test]
fn watching_a_path_that_is_not_there_is_an_error_not_a_silent_no_op() {
    let dir = tempfile::tempdir().unwrap();
    let mut set = WatchSet::new().unwrap();
    let err = set.retarget(&[WatchTarget {
        location_id: LocationId(1),
        root: dir.path().join("gone"),
        pinned: false,
    }]);
    assert!(err.is_err());
}
