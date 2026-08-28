#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! §3.3's history rows: the root set, the committer walk and the subject list.

mod support;

use codotheca_core::cancel::CancelToken;
use codotheca_core::git::{
    authorship, commit_subjects, local_day, parse_tz_offset_min, root_commits, RunLimits,
};
use support::TestRepo;

fn limits() -> RunLimits {
    RunLimits::none()
}

#[test]
fn timezone_offsets_parse_in_both_directions() {
    assert_eq!(parse_tz_offset_min("+0000"), Some(0));
    assert_eq!(parse_tz_offset_min("+0530"), Some(330));
    assert_eq!(parse_tz_offset_min("-0800"), Some(-480));
    assert_eq!(parse_tz_offset_min("+1245"), Some(765));
    assert_eq!(parse_tz_offset_min("nonsense"), None);
}

#[test]
fn a_local_day_is_the_users_day_not_utc() {
    // 2024-01-01T23:30:00Z is still 2024-01-01 at UTC and already 2024-01-02 at +0530.
    let utc = 1_704_151_800;
    assert_eq!(local_day(utc, 0), 19_723);
    assert_eq!(local_day(utc, 330), 19_724);
    assert_eq!(local_day(utc, -480), 19_723);
}

#[test]
fn the_root_set_and_its_dates_come_back_together() {
    let repo = TestRepo::init();
    repo.write("a.txt", b"one\n");
    repo.commit_at("first", "2021-03-04T05:06:07+05:30");
    repo.write("a.txt", b"two\n");
    repo.commit_at("second", "2022-03-04T05:06:07+00:00");

    let roots = root_commits(&repo.exec(), &repo.handle(), limits(), &CancelToken::new()).unwrap();
    assert_eq!(roots.len(), 1);
    assert_eq!(
        roots[0].tz_offset_min, 330,
        "the local day must survive as an offset"
    );
    assert!(roots[0].committed_at > 1_600_000_000);
}

#[test]
fn a_multi_root_history_reports_every_root() {
    let repo = TestRepo::init();
    repo.write("a.txt", b"one\n");
    repo.commit("main root");
    repo.git(&["checkout", "-q", "--orphan", "second"]);
    repo.git(&["rm", "-r", "-f", "-q", "."]);
    repo.write("b.txt", b"two\n");
    repo.commit("second root");
    repo.git(&["checkout", "-q", "main"]);
    repo.git(&[
        "merge",
        "-q",
        "--allow-unrelated-histories",
        "--no-edit",
        "second",
    ]);

    let roots = root_commits(&repo.exec(), &repo.handle(), limits(), &CancelToken::new()).unwrap();
    assert_eq!(roots.len(), 2, "both roots are reachable from HEAD");
}

#[test]
fn a_zero_commit_repository_has_no_roots_and_no_authorship() {
    let repo = TestRepo::init();
    let roots = root_commits(&repo.exec(), &repo.handle(), limits(), &CancelToken::new()).unwrap();
    assert!(roots.is_empty());
    let a = authorship(&repo.exec(), &repo.handle(), limits(), &CancelToken::new()).unwrap();
    assert!(a.committers.is_empty());
    let s = commit_subjects(
        &repo.exec(),
        &repo.handle(),
        200,
        limits(),
        &CancelToken::new(),
    )
    .unwrap();
    assert!(s.is_empty());
}

#[test]
fn authorship_tallies_committers_and_their_local_days() {
    let repo = TestRepo::init();
    repo.write("a.txt", b"one\n");
    repo.commit_at("first", "2024-05-01T10:00:00+00:00");
    repo.write("a.txt", b"two\n");
    repo.commit_at("second", "2024-05-01T22:00:00+00:00"); // same local day
    repo.write("a.txt", b"three\n");
    repo.commit_at("third", "2024-05-03T10:00:00+00:00");

    let a = authorship(&repo.exec(), &repo.handle(), limits(), &CancelToken::new()).unwrap();
    assert_eq!(a.committers.len(), 1);
    let t = &a.committers[0];
    assert_eq!(t.email, "fixture@example.invalid");
    assert_eq!(t.commits, 3);
    assert_eq!(t.days.len(), 2, "three commits across two local days");
}

#[test]
fn authorship_uses_the_committer_not_the_author() {
    let repo = TestRepo::init();
    repo.write("a.txt", b"one\n");
    repo.commit("first");
    repo.write("a.txt", b"two\n");
    repo.git(&["add", "-A"]);
    // The committer stays the fixture identity; only the author differs.
    repo.git(&[
        "commit",
        "-q",
        "--author=Someone Else <author@example.invalid>",
        "-m",
        "authored elsewhere",
    ]);

    let a = authorship(&repo.exec(), &repo.handle(), limits(), &CancelToken::new()).unwrap();
    let emails: Vec<&str> = a.committers.iter().map(|c| c.email.as_str()).collect();
    assert_eq!(
        emails,
        vec!["fixture@example.invalid"],
        "the committer is the tallied identity"
    );
    assert!(
        !emails.contains(&"author@example.invalid"),
        "the author must NOT be tallied: v1's %ae bug, got {emails:?}"
    );
}

#[test]
fn subjects_are_newest_first_bounded_and_carry_their_commit_time() {
    let repo = TestRepo::init();
    for i in 0..5 {
        repo.write("a.txt", format!("{i}\n").as_bytes());
        repo.commit(&format!("commit {i}"));
    }
    let s = commit_subjects(
        &repo.exec(),
        &repo.handle(),
        3,
        limits(),
        &CancelToken::new(),
    )
    .unwrap();
    assert_eq!(s.len(), 3);
    assert_eq!(s[0].subject, "commit 4");
    assert!(s[0].committed_at > 1_600_000_000);
    assert_eq!(s[0].oid.len(), 40);
}

#[test]
fn a_subject_containing_a_tab_survives_the_split() {
    let repo = TestRepo::init();
    repo.write("a.txt", b"one\n");
    repo.git(&["add", "-A"]);
    repo.git(&["commit", "-q", "-m", "before\tafter"]);
    let s = commit_subjects(
        &repo.exec(),
        &repo.handle(),
        1,
        limits(),
        &CancelToken::new(),
    )
    .unwrap();
    assert_eq!(s[0].subject, "before\tafter");
}
