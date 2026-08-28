//! §6's basis, against a real repository, from both sides at once.
//!
//! Plan 05 and plan 09 each shipped a digest called "the ref-state basis". If the two disagree
//! the failure is silent and total: J1 writes one value into `location.refstate_basis`, the
//! freshness gate computes the other, they never compare equal, and every cacheable job re-runs
//! on every tick — a scheduler that never caches anything and never says so.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::must_use_candidate,
    clippy::missing_panics_doc
)]

mod support;

use codotheca_core::freshness::{collect_basis_inputs_split, compute_basis};
use codotheca_core::git::ref_fingerprint;
use support::TestRepo;

/// The one that matters: the value J1 stores and the value the gate computes are the same value.
#[test]
fn the_stored_basis_and_the_gates_basis_are_one_digest() {
    let repo = TestRepo::init();
    repo.write("a.txt", b"one\n");
    repo.git(&["add", "a.txt"]);
    repo.commit("first");

    let handle = repo.handle();
    let stored = ref_fingerprint(&handle).unwrap();
    let gate =
        compute_basis(&collect_basis_inputs_split(&handle.git_dir, &handle.common_dir).unwrap());
    assert_eq!(stored, gate);
}

/// A commit moves a loose ref, so the basis must move with it.
#[test]
fn a_commit_moves_the_basis() {
    let repo = TestRepo::init();
    repo.write("a.txt", b"one\n");
    repo.git(&["add", "a.txt"]);
    repo.commit("first");
    let handle = repo.handle();
    let before = ref_fingerprint(&handle).unwrap();

    repo.write("a.txt", b"two\n");
    repo.git(&["add", "a.txt"]);
    repo.commit("second");
    let after = ref_fingerprint(&handle).unwrap();

    assert_ne!(before, after);
}

/// A stash moves `logs/refs/stash` and nothing else the digest would otherwise see. `RefState`
/// reads `stash_count` from that file, so a basis blind to it leaves the count stale forever.
#[test]
fn a_stash_moves_the_basis() {
    let repo = TestRepo::init();
    repo.write("a.txt", b"one\n");
    repo.git(&["add", "a.txt"]);
    repo.commit("first");
    let handle = repo.handle();
    let before = ref_fingerprint(&handle).unwrap();

    repo.write("a.txt", b"dirty\n");
    repo.git(&["stash", "push", "-m", "wip"]);
    let after = ref_fingerprint(&handle).unwrap();

    assert_ne!(before, after, "a stash must invalidate the ref-state cache");
}

/// The basis is a digest and a digest has no age: it is 64 lowercase hex characters, which is
/// exactly what `refstate_basis TEXT` and `RefFingerprint::from_hex` both require.
#[test]
fn the_basis_is_storable_hex() {
    let repo = TestRepo::init();
    repo.write("a.txt", b"one\n");
    repo.git(&["add", "a.txt"]);
    repo.commit("first");
    let hex = ref_fingerprint(&repo.handle()).unwrap().to_hex();
    assert_eq!(hex.len(), 64);
    assert!(codotheca_core::git::RefFingerprint::from_hex(&hex).is_some());
}
