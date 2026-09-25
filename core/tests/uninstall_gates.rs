#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

//! §24.7B–G, now §45.6 step 2's gates. **AC-P2-24-16: none of these ever yields `safe`.**
//!
//! [p4] The remote cases — every failure is unknown, a mirror is named only beside an uncovered
//! root, a reached remote records when — are §45.3's composition's now, and run through the
//! handlers in `analyser_remotes.rs`.

use std::path::PathBuf;

use codotheca_core::analyser::gates::{gate_first_day, gate_path, gate_shallow};
use codotheca_core::analyser::verdict::fold_disposition;
use codotheca_core::protocol::{UninstallBlocker, UninstallDisposition};

/// §24.7B: a shallow clone can never prove every local ref is upstream, so it never clears.
#[test]
fn a_shallow_clone_never_clears() {
    let blocker = gate_shallow(true).expect("a shallow clone blocks");
    assert_eq!(blocker, UninstallBlocker::ShallowClone);
    assert_eq!(
        fold_disposition(&[blocker]),
        UninstallDisposition::Unknown,
        "shallow is an unknown, not a known-bad — but neither reaches removal"
    );
    assert_eq!(gate_shallow(false), None);
}

/// §24.7G: *never observed* and *stale but once known* are distinct, and the lock fires only on
/// the first.
#[test]
fn a_location_the_app_has_never_looked_at_is_not_uninstallable() {
    assert_eq!(
        gate_first_day(None, Some(100)),
        Some(UninstallBlocker::NeverObserved)
    );
    assert_eq!(
        gate_first_day(Some(100), None),
        Some(UninstallBlocker::NeverObserved)
    );
    assert_eq!(
        gate_first_day(None, None),
        Some(UninstallBlocker::NeverObserved)
    );
    // Stale but once known is a different fact and must not be rendered as the first.
    assert_eq!(
        gate_first_day(Some(1), Some(2)),
        None,
        "an old observation is still an observation"
    );
}

/// **§24.7D's containment, in both directions.** A path *under* a root passes; a path that *is* a
/// root is refused; a path outside every root is refused.
#[test]
fn the_scan_root_containment_check_runs_in_both_directions() {
    let dir = tempfile::tempdir().expect("tmp");
    let root = dir.path().join("library");
    let copy = root.join("widget");
    std::fs::create_dir_all(copy.join(".git")).expect("mkdir");
    let roots = vec![root.clone()];

    assert_eq!(
        gate_path(&copy, &roots),
        None,
        "a copy under a root is fine"
    );

    // The root itself: refused. Removing a scan root would take every project under it.
    std::fs::create_dir_all(root.join(".git")).expect("mkdir");
    assert_eq!(
        gate_path(&root, &roots),
        Some(UninstallBlocker::RefusedPath),
        "a path that IS a root is refused"
    );

    // Outside every root: refused, because the app was never given consent to touch it.
    let outside = dir.path().join("elsewhere");
    std::fs::create_dir_all(outside.join(".git")).expect("mkdir");
    assert_eq!(
        gate_path(&outside, &roots),
        Some(UninstallBlocker::RefusedPath),
        "a path outside every root has no consent behind it"
    );
}

/// A symlink is refused and **never followed** — here or while removing.
#[cfg(unix)]
#[test]
fn a_symlinked_path_is_refused_without_being_followed() {
    let dir = tempfile::tempdir().expect("tmp");
    let root = dir.path().join("library");
    let real = dir.path().join("precious");
    std::fs::create_dir_all(real.join(".git")).expect("mkdir");
    std::fs::create_dir_all(&root).expect("mkdir");
    let link = root.join("widget");
    std::os::unix::fs::symlink(&real, &link).expect("symlink");

    assert_eq!(
        gate_path(&link, &[root]),
        Some(UninstallBlocker::RefusedPath),
        "a symlink names somewhere else entirely"
    );
    assert!(real.join(".git").exists());
}

/// A directory with no `.git` directly inside it is not a working copy this may clear.
#[test]
fn a_path_whose_git_is_not_a_direct_child_is_refused() {
    let dir = tempfile::tempdir().expect("tmp");
    let root = dir.path().join("library");
    let plain = root.join("notes");
    std::fs::create_dir_all(&plain).expect("mkdir");
    assert_eq!(
        gate_path(&plain, std::slice::from_ref(&root)),
        Some(UninstallBlocker::RefusedPath)
    );

    // [p4] A `.git` **file** — a linked worktree's or a separate git dir's gitfile — is not the
    // copy's own git directory either (§45.5), and `exists()` used to pass it.
    let gitfile = root.join("linked");
    std::fs::create_dir_all(&gitfile).expect("mkdir");
    std::fs::write(gitfile.join(".git"), b"gitdir: /elsewhere/.git\n").expect("gitfile");
    assert_eq!(
        gate_path(&gitfile, &[root]),
        Some(UninstallBlocker::RefusedPath),
        "a gitfile names a git dir somewhere else"
    );
}

/// Every gate's blocker folds away from `safe` — the property AC-P2-24-16 is really about.
#[test]
fn every_gate_blocker_folds_away_from_safe() {
    let blockers = [
        UninstallBlocker::ShallowClone,
        UninstallBlocker::RemoteUnreachable,
        UninstallBlocker::RemoteIsLocalMirror,
        UninstallBlocker::RefusedPath,
        UninstallBlocker::LiveSession,
        UninstallBlocker::NeverObserved,
    ];
    for blocker in blockers {
        assert_ne!(
            fold_disposition(&[blocker]),
            UninstallDisposition::Safe,
            "{blocker:?}"
        );
    }
    let _unused: Vec<PathBuf> = Vec::new();
}
