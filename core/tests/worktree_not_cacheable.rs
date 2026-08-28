//! Acceptance criterion 6: editing a tracked file and re-observing flips `is:dirty` to true.
//!
//! This is the v1 regression, and it is asserted against a real repository on disk because the
//! finding it encodes is a property of git's own bookkeeping, not of our types. v1 keyed the
//! status cache on `.git/index` mtime plus HEAD; an edit moves neither, so an actively-worked
//! repository would have been cached as clean indefinitely.
//!
//! **R29**: no second `TempRepo`. The fixture is the one every git integration test in the tree
//! already shares.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::must_use_candidate,
    clippy::missing_panics_doc
)]

mod support;

use codotheca_core::freshness::{
    collect_basis_inputs_split, compute_basis, FreshnessGate, StoredObservation,
};
use codotheca_core::jobs::JobKind;
use support::TestRepo;

#[test]
fn editing_a_tracked_file_moves_neither_head_nor_the_index_mtime() {
    let repo = TestRepo::init();
    repo.write("src/lib.rs", b"pub fn a() {}\n");
    repo.git(&["add", "src/lib.rs"]);
    repo.commit("first");

    let handle = repo.handle();
    let index_mtime = || {
        std::fs::metadata(handle.git_dir.join("index"))
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_nanos())
    };
    let basis =
        || compute_basis(&collect_basis_inputs_split(&handle.git_dir, &handle.common_dir).unwrap());

    let before_index = index_mtime();
    let before_basis = basis();

    // No git command runs. This is exactly what an editor does when it saves.
    repo.write("src/lib.rs", b"pub fn a() { let _ = 1; }\n");

    assert_eq!(
        before_index,
        index_mtime(),
        "the index mtime does not move on an edit"
    );
    assert_eq!(
        before_basis,
        basis(),
        "no ref-state fingerprint can see this edit"
    );
}

/// The consequence, stated as the gate answers it: because no fingerprint moved, a cache keyed
/// on one would have skipped J2 — so J2 must not be keyed on one.
#[test]
fn the_gate_reruns_j2_across_an_edit_that_no_fingerprint_can_see() {
    let repo = TestRepo::init();
    repo.write("src/lib.rs", b"pub fn a() {}\n");
    repo.git(&["add", "src/lib.rs"]);
    repo.commit("first");

    let handle = repo.handle();
    let basis =
        compute_basis(&collect_basis_inputs_split(&handle.git_dir, &handle.common_dir).unwrap());
    let stored = StoredObservation {
        basis: Some(basis.clone()),
        observed_at: 10,
    };

    repo.write("src/lib.rs", b"pub fn a() { let _ = 1; }\n");
    let after =
        compute_basis(&collect_basis_inputs_split(&handle.git_dir, &handle.common_dir).unwrap());
    assert_eq!(basis, after, "the premise: the basis really did not move");

    assert!(FreshnessGate::needs_run(
        JobKind::J2Status,
        Some(&stored),
        Some(&after)
    ));
    assert!(!FreshnessGate::needs_run(
        JobKind::J1Refstate,
        Some(&stored),
        Some(&after)
    ));
}
