#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! §46.9's path release, on real repositories through the scan's hand-off. A copy this app
//! removed is evidence of nothing on disk now: a different repository found later at its path is
//! not its project.

#[path = "support/handoff_rig.rs"]
mod handoff_rig;

use handoff_rig::Rig;

#[test]
fn a_different_repository_at_a_removed_copys_path_is_not_its_project() {
    let rig = Rig::new();
    let path = rig.repo("x", "first");
    let a = rig.hand_off(&path);
    rig.uninstall(a.location, &path);
    rig.repo("x", "another first");
    let b = rig.hand_off(&path);
    eprintln!("A at the path {a:?}, then B at the same path {b:?}");
    assert_ne!(
        b.project, a.project,
        "a different repository joined the removed copy's project"
    );
}
