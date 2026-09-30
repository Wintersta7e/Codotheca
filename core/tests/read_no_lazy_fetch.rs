#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! R249: a read of a partial clone never fetches from its promisor remote.
//!
//! On a git that honours `GIT_NO_LAZY_FETCH` the read answers the absent blobs as missing; on one
//! that ignores it (the governed floor, 2.29) the read is refused before any child starts. Either
//! way the clone holds exactly the objects it held before. The promisor stays reachable and holds
//! every object, so a fetch would succeed and the count would fall — which is what makes a fetch
//! visible. Which branch runs is keyed on the probe, printed, never on a version number (R246).
//! Run it on every matrix git through `CODOTHECA_TEST_GIT`.

mod support;

use std::path::{Path, PathBuf};
use std::time::Duration;

use codotheca_core::cancel::CancelToken;
use codotheca_core::clock::SystemClock;
use codotheca_core::git::{
    read_blobs, tracked_inventory, GitError, LazyFetch, RepoHandle, RunLimits, StoreKey,
};
use codotheca_core::mount::StoreClass;
use support::{test_git, TestRepo};

/// A `blob:none` partial clone of `source`, with its index filled from HEAD's trees so the
/// product's index reads name blobs the clone does not hold.
fn partial_clone(source: &TestRepo) -> PathBuf {
    let clone = source.scratch().join("partial");
    source.git(&["config", "uploadpack.allowFilter", "true"]);
    source.git(&["config", "uploadpack.allowAnySHA1InWant", "true"]);
    source.git(&[
        "clone",
        "-q",
        "--no-local",
        "--no-checkout",
        "--filter=blob:none",
        source.path().to_str().unwrap(),
        clone.to_str().unwrap(),
    ]);
    source.git(&["-C", clone.to_str().unwrap(), "read-tree", "HEAD"]);
    clone
}

/// The blobs `clone` does not hold, as `rev-list --missing=print` names them. That flag reads
/// without fetching on every git in the matrix, so counting cannot itself move the count.
fn missing(source: &TestRepo, clone: &Path) -> Vec<String> {
    source
        .git(&[
            "-C",
            clone.to_str().unwrap(),
            "rev-list",
            "--objects",
            "--missing=print",
            "--all",
        ])
        .lines()
        .filter_map(|line| line.strip_prefix('?').map(str::to_owned))
        .collect()
}

fn source_repo() -> TestRepo {
    let source = TestRepo::init();
    source.write("a.txt", b"alpha\n");
    source.write("src/b.rs", b"fn b() {}\n");
    source.commit("first");
    source
}

#[test]
fn a_read_of_a_partial_clone_never_fetches_from_its_promisor() {
    let source = source_repo();
    let clone = partial_clone(&source);
    let before = missing(&source, &clone);
    assert_eq!(
        before.len(),
        2,
        "the fixture must start with both blobs absent, or nothing here is tested"
    );

    let exec = source.exec();
    let probe = exec.lazy_fetch();
    eprintln!(
        "git under test: {} — lazy-fetch probe: {probe:?}",
        test_git().display()
    );
    let handle = RepoHandle::resolve(&clone, StoreKey::new("test-store"), StoreClass::Local)
        .expect("the partial clone resolves");
    let cancel = CancelToken::new();
    let limits = RunLimits::after(Duration::from_secs(60));

    let inventory = tracked_inventory(&exec, &handle, limits, &cancel, &SystemClock::new());
    let blobs = read_blobs(&exec, &handle, &before, 1024, u64::MAX, limits, &cancel);

    match probe {
        // A git that honours the pin either answers an absent blob as missing, which contributes
        // nothing, or refuses the whole read — measured on 2.43: *"lazy fetching disabled … could
        // not fetch … from promisor remote"*, exit 128. Either is honest; neither is absence.
        LazyFetch::Pinned => {
            eprintln!("inventory: {inventory:?}\nblobs: {blobs:?}");
            match &inventory {
                Ok(inventory) => assert_eq!(
                    inventory.size_tracked_bytes, 0,
                    "an absent blob contributes nothing; bytes here were fetched"
                ),
                Err(e) => assert!(!e.implies_absent(), "a refused read is unknown: {e:?}"),
            }
            match &blobs {
                Ok(blobs) => assert!(
                    blobs.reads.is_empty(),
                    "an absent blob has no body; a body here was fetched"
                ),
                Err(e) => assert!(!e.implies_absent(), "a refused read is unknown: {e:?}"),
            }
        }
        LazyFetch::Unguarded => {
            assert!(
                matches!(inventory, Err(GitError::LazyFetchUnguarded)),
                "{inventory:?}"
            );
            assert!(
                matches!(blobs, Err(GitError::LazyFetchUnguarded)),
                "{blobs:?}"
            );
        }
    }
    assert_eq!(
        missing(&source, &clone),
        before,
        "a read fetched from the promisor remote"
    );
}

#[test]
fn a_repository_without_a_promisor_reads_whatever_the_probe_says() {
    let source = source_repo();
    let exec = source.exec();
    eprintln!("lazy-fetch probe: {:?}", exec.lazy_fetch());
    let inventory = tracked_inventory(
        &exec,
        &source.handle(),
        RunLimits::after(Duration::from_secs(60)),
        &CancelToken::new(),
        &SystemClock::new(),
    )
    .expect("the refusal is for partial clones only");
    assert_eq!(inventory.tracked_files, 2);
    assert_eq!(inventory.size_tracked_bytes, 16);
}
