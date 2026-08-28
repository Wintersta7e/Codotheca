//! Compiled only under `testkit`: these link `codotheca_core::testing`, which the feature gates.
#![cfg(feature = "testkit")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

//! §4.3's second paragraph. Links are not followed by default; enabled, three refusals stand
//! between a link and the scan — the consent boundary, the mount boundary, and a directory this
//! run already reached, which is also what breaks a cycle.

use codotheca_core::index::path::native_platform;
use codotheca_core::mount::{MountFacts, StoreClass};
use codotheca_core::paths::path_key;
use codotheca_core::scan::links::{LinkPolicy, LinkVerdict};
use codotheca_core::testing::FakeMountResolver;
use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;

fn facts(store: &str, volume: &str) -> MountFacts {
    MountFacts {
        store_key: store.to_owned(),
        volume_key: Some(volume.to_owned()),
        class: StoreClass::Local,
    }
}

/// One root, one store, and a mount table that answers for the whole temp tree.
fn policy(follow: bool, root: &Path, mounts: FakeMountResolver) -> LinkPolicy {
    let mut stores = BTreeSet::new();
    stores.insert("store-a".to_owned());
    LinkPolicy::new(
        follow,
        vec![path_key(root, native_platform())],
        stores,
        Arc::new(mounts),
    )
}

fn mounts_under(base: &Path, store: &str, volume: &str) -> FakeMountResolver {
    let m = FakeMountResolver::new();
    m.map(base, facts(store, volume));
    m
}

#[cfg(unix)]
#[test]
fn two_paths_to_one_directory_share_a_file_id() {
    use codotheca_core::scan::links::file_id;
    let base = tempfile::tempdir().unwrap();
    let real = base.path().join("real");
    std::fs::create_dir_all(&real).unwrap();
    let link = base.path().join("link");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    assert_eq!(file_id(&real).unwrap(), file_id(&link).unwrap());

    let other = base.path().join("other");
    std::fs::create_dir_all(&other).unwrap();
    assert_ne!(file_id(&real).unwrap(), file_id(&other).unwrap());
}

#[test]
fn links_are_not_followed_by_default_and_that_is_not_a_refusal() {
    let base = tempfile::tempdir().unwrap();
    let target = base.path().join("t");
    std::fs::create_dir_all(&target).unwrap();
    let p = policy(
        false,
        base.path(),
        mounts_under(base.path(), "store-a", "vol-a"),
    );
    assert_eq!(p.judge(&target), LinkVerdict::NotFollowed);
    assert_eq!(p.refused(), 0);
}

#[test]
fn a_link_out_of_every_enabled_root_is_refused() {
    let base = tempfile::tempdir().unwrap();
    let inside = base.path().join("in");
    std::fs::create_dir_all(&inside).unwrap();
    let p = policy(true, &inside, mounts_under(base.path(), "store-a", "vol-a"));
    assert_eq!(
        p.judge(&base.path().join("elsewhere")),
        LinkVerdict::RefusedOutsideRoots
    );
    assert_eq!(p.refused(), 1);
}

#[test]
fn a_link_onto_another_store_is_refused() {
    let base = tempfile::tempdir().unwrap();
    let target = base.path().join("t");
    std::fs::create_dir_all(&target).unwrap();
    let p = policy(
        true,
        base.path(),
        mounts_under(base.path(), "store-b", "vol-b"),
    );
    assert_eq!(p.judge(&target), LinkVerdict::RefusedCrossStore);
    assert_eq!(p.refused(), 1);
}

/// A target whose store cannot be established is refused, never assumed onto the root's store:
/// putting work on a queue the run never sized is exactly what the mount boundary prevents.
#[test]
fn a_target_whose_store_cannot_be_resolved_is_refused_rather_than_assumed() {
    let base = tempfile::tempdir().unwrap();
    let target = base.path().join("t");
    std::fs::create_dir_all(&target).unwrap();
    // An empty mount table: every resolve is `Unsupported`.
    let p = policy(true, base.path(), FakeMountResolver::new());
    assert_eq!(p.judge(&target), LinkVerdict::RefusedUnreadable);
    assert_eq!(p.refused(), 1);
}

/// An unplugged volume is the same answer: `NotMounted` is not permission to follow.
#[test]
fn a_link_onto_an_unmounted_volume_is_refused() {
    let base = tempfile::tempdir().unwrap();
    let target = base.path().join("t");
    std::fs::create_dir_all(&target).unwrap();
    let mounts = mounts_under(base.path(), "store-a", "vol-a");
    mounts.unmount("vol-a");
    let p = policy(true, base.path(), mounts);
    assert_eq!(p.judge(&target), LinkVerdict::RefusedUnreadable);
}

#[cfg(unix)]
#[test]
fn the_second_arrival_at_one_directory_is_refused_which_is_what_breaks_a_cycle() {
    let base = tempfile::tempdir().unwrap();
    let target = base.path().join("t");
    std::fs::create_dir_all(&target).unwrap();
    let link = base.path().join("loop");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    let p = policy(
        true,
        base.path(),
        mounts_under(base.path(), "store-a", "vol-a"),
    );
    assert_eq!(p.judge(&target), LinkVerdict::Follow);
    assert_eq!(p.judge(&link), LinkVerdict::RefusedDuplicate);
    assert_eq!(p.refused(), 1);
}

/// A refusal is counted, never surfaced: §11.1 fixes the problem groups at eight and none of
/// them is "links". The count is what reaches the run outcome.
#[test]
fn refusals_accumulate_across_verdicts() {
    let base = tempfile::tempdir().unwrap();
    let inside = base.path().join("in");
    std::fs::create_dir_all(&inside).unwrap();
    let p = policy(true, &inside, mounts_under(base.path(), "store-a", "vol-a"));
    p.judge(&base.path().join("a"));
    p.judge(&base.path().join("b"));
    assert_eq!(p.refused(), 2);
    assert!(!LinkVerdict::RefusedOutsideRoots.is_follow());
    assert!(LinkVerdict::Follow.is_follow());
    assert!(!LinkVerdict::NotFollowed.is_follow());
}
