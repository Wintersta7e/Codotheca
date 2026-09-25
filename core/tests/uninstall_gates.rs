#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

//! §24.7B–G. **AC-P2-24-16: none of these ever yields `safe`.**

use std::path::PathBuf;

use codotheca_core::protocol::{UninstallBlocker, UninstallDisposition};
use codotheca_core::uninstall::fold_disposition;
use codotheca_core::uninstall::gates::{
    gate_first_day, gate_path, gate_shallow, is_local_mirror, verify_remote, RemoteOutcome,
};

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

/// **AC-P2-24-16.** 401, 403, 404 and offline are one claim — *this was not established* — and
/// **none of them is ever `safe`**.
///
/// A 404 against a private repository the caller cannot see is indistinguishable from one that
/// does not exist. Rendering it as *gone* is the specific mistake that turns this into a shredder.
#[test]
fn no_remote_failure_ever_yields_safe() {
    for outcome in [RemoteOutcome::Refused, RemoteOutcome::Unreachable] {
        let verification = verify_remote(outcome, 100);
        assert_eq!(
            verification.blockers,
            vec![UninstallBlocker::RemoteUnreachable],
            "{outcome:?}"
        );
        assert_eq!(
            verification.verified_at, None,
            "{outcome:?}: nothing was verified, so no instant is claimed"
        );
        assert_ne!(
            fold_disposition(&verification.blockers),
            UninstallDisposition::Safe,
            "{outcome:?} must never reach safe"
        );
    }
}

/// A mirror on the same machine is reported honestly and never counted as a backup: one disk
/// failure takes both copies.
///
/// **[p4] §45.9 moved `remote_is_local_mirror` to the unknown class**, so alone it folds to
/// `unknown` — still never `safe`. The name's *blocked* stops being true here; the test is
/// renamed with its body when §45.3's composition replaces `verify_remote`.
#[test]
fn a_local_mirror_is_blocked_and_not_treated_as_a_backup() {
    let verification = verify_remote(RemoteOutcome::LocalMirror, 100);
    assert_eq!(
        verification.blockers,
        vec![UninstallBlocker::RemoteIsLocalMirror]
    );
    assert_eq!(
        fold_disposition(&verification.blockers),
        UninstallDisposition::Unknown,
        "never counted as a backup, and never a verdict on its own (§45.9)"
    );
}

#[test]
fn a_reached_remote_records_when_it_was_reached() {
    let verification = verify_remote(RemoteOutcome::Reached, 1_700_000_000);
    assert!(verification.blockers.is_empty());
    assert_eq!(
        verification.verified_at,
        Some(1_700_000_000),
        "the label is VERIFIED <age>, never PUSHED — so the instant has to be real"
    );
}

#[test]
fn a_local_path_remote_is_recognised_however_it_is_spelled() {
    for local in [
        "file:///srv/mirrors/widget.git",
        "/srv/mirrors/widget.git",
        "../widget.git",
        "~/mirrors/widget.git",
        "D:\\Mirrors\\widget.git",
    ] {
        assert!(is_local_mirror(local), "{local} is on this machine");
    }
    for remote in [
        "https://forge.example/owner/widget",
        "ssh://git@forge.example/owner/widget.git",
        "git@forge.example:owner/widget.git",
    ] {
        assert!(!is_local_mirror(remote), "{remote} is a real remote");
    }
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
        gate_path(&plain, &[root]),
        Some(UninstallBlocker::RefusedPath)
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
