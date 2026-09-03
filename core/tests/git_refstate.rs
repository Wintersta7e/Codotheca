#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! §3.3's first row: ref state read straight off the filesystem, and §6's basis.

mod support;

use codotheca_core::clock::SystemClock;
use codotheca_core::git::{
    observation_fingerprint, read_ref_state, ref_fingerprint, GitError, InterruptedOp,
    RefFingerprint,
};
use support::TestRepo;

/// `SystemClock` carries a monotonic origin, so it is constructed rather than named (plan 06).
fn clock() -> SystemClock {
    SystemClock::new()
}

#[test]
fn reads_branch_head_and_counts_without_spawning_git() {
    let repo = TestRepo::init();
    repo.write("a.txt", b"one\n");
    repo.commit("first");
    repo.git(&["tag", "v1"]);
    repo.git(&["tag", "v2"]);

    let st = read_ref_state(&repo.handle(), &clock()).unwrap();
    assert_eq!(st.branch.as_deref(), Some("main"));
    assert_eq!(
        st.head_oid.as_deref(),
        Some(repo.git(&["rev-parse", "HEAD"]).trim())
    );
    assert_eq!(st.tag_count, 2);
    assert_eq!(st.stash_count, 0);
    assert!(!st.is_shallow);
    assert!(!st.is_bare);
    assert_eq!(st.interrupted_op, None);
    assert_eq!(st.fetch_head_at, None, "no fetch recorded is None, never 0");
    // R5: the union's three added fields.
    assert_eq!(
        st.ahead, None,
        "a file read cannot count ahead; None is not computed, not 0"
    );
    assert_eq!(st.behind, None, "and the same for behind");
    assert!(st.reflog_tail_at.is_some(), "committing wrote logs/HEAD");
    assert!(st.observed_at > 1_700_000_000);
}

#[test]
fn a_detached_head_has_an_oid_and_no_branch() {
    let repo = TestRepo::init();
    repo.write("a.txt", b"one\n");
    repo.commit("first");
    let oid = repo.git(&["rev-parse", "HEAD"]).trim().to_owned();
    repo.git(&["checkout", "-q", "--detach", &oid]);

    let st = read_ref_state(&repo.handle(), &clock()).unwrap();
    assert_eq!(st.branch, None);
    assert_eq!(st.head_oid.as_deref(), Some(oid.as_str()));
}

#[test]
fn an_unborn_head_reports_a_branch_and_no_oid() {
    let repo = TestRepo::init();
    let st = read_ref_state(&repo.handle(), &clock()).unwrap();
    assert_eq!(st.branch.as_deref(), Some("main"));
    assert_eq!(
        st.head_oid, None,
        "no commit yet is not computed, never a zero oid"
    );
}

#[test]
fn packed_refs_are_resolved_as_well_as_loose_ones() {
    let repo = TestRepo::init();
    repo.write("a.txt", b"one\n");
    repo.commit("first");
    let oid = repo.git(&["rev-parse", "HEAD"]).trim().to_owned();
    repo.git(&["tag", "packed-tag"]);
    repo.git(&["pack-refs", "--all"]);

    let st = read_ref_state(&repo.handle(), &clock()).unwrap();
    assert_eq!(st.head_oid.as_deref(), Some(oid.as_str()));
    assert_eq!(st.tag_count, 1);
}

#[test]
fn stashes_are_counted_from_the_stash_reflog() {
    let repo = TestRepo::init();
    repo.write("a.txt", b"one\n");
    repo.commit("first");
    repo.write("a.txt", b"two\n");
    repo.git(&["stash", "push", "-q", "-m", "one"]);
    repo.write("a.txt", b"three\n");
    repo.git(&["stash", "push", "-q", "-m", "two"]);

    let st = read_ref_state(&repo.handle(), &clock()).unwrap();
    assert_eq!(st.stash_count, 2);
}

#[test]
fn an_unreadable_stash_reflog_is_not_reported_as_zero() {
    let repo = TestRepo::init();
    repo.write("a.txt", b"one\n");
    repo.commit("first");
    let stash = repo
        .path()
        .join(".git")
        .join("logs")
        .join("refs")
        .join("stash");
    std::fs::write(stash, [0xff]).unwrap();

    let error = read_ref_state(&repo.handle(), &clock())
        .expect_err("an unreadable stash reflog must not produce a numeric stash count");
    assert!(matches!(error, GitError::Unreadable { .. }), "{error:?}");
}

#[test]
fn an_interrupted_merge_and_rebase_are_seen() {
    let repo = TestRepo::init();
    repo.write("a.txt", b"one\n");
    repo.commit("first");
    std::fs::write(repo.path().join(".git").join("MERGE_HEAD"), "0".repeat(40)).unwrap();
    let st = read_ref_state(&repo.handle(), &clock()).unwrap();
    assert_eq!(st.interrupted_op, Some(InterruptedOp::Merge));
    assert_eq!(st.interrupted_op.map(|o| o.as_str()), Some("merge"));

    std::fs::remove_file(repo.path().join(".git").join("MERGE_HEAD")).unwrap();
    std::fs::create_dir_all(repo.path().join(".git").join("rebase-merge")).unwrap();
    let st = read_ref_state(&repo.handle(), &clock()).unwrap();
    assert_eq!(st.interrupted_op, Some(InterruptedOp::Rebase));
}

#[test]
fn the_upstream_comes_from_config_and_resolves_to_an_oid() {
    let repo = TestRepo::init();
    repo.write("a.txt", b"one\n");
    repo.commit("first");
    let oid = repo.git(&["rev-parse", "HEAD"]).trim().to_owned();
    repo.git(&["update-ref", "refs/remotes/origin/main", &oid]);
    repo.git(&["config", "branch.main.remote", "origin"]);
    repo.git(&["config", "branch.main.merge", "refs/heads/main"]);

    let st = read_ref_state(&repo.handle(), &clock()).unwrap();
    let up = st.upstream.expect("upstream should resolve");
    assert_eq!(up.remote, "origin");
    assert_eq!(up.ref_name, "refs/remotes/origin/main");
    assert_eq!(up.oid.as_deref(), Some(oid.as_str()));
}

#[test]
fn fetch_head_supplies_a_content_clock() {
    let repo = TestRepo::init();
    repo.write("a.txt", b"one\n");
    repo.commit("first");
    assert_eq!(
        read_ref_state(&repo.handle(), &clock())
            .unwrap()
            .fetch_head_at,
        None
    );

    std::fs::write(repo.path().join(".git").join("FETCH_HEAD"), b"").unwrap();
    let at = read_ref_state(&repo.handle(), &clock())
        .unwrap()
        .fetch_head_at;
    assert!(
        at.unwrap_or(0) > 1_700_000_000,
        "FETCH_HEAD mtime should be a real epoch"
    );
}

#[test]
fn a_shallow_clone_is_flagged() {
    let repo = TestRepo::init();
    repo.write("a.txt", b"one\n");
    repo.commit("first");
    std::fs::write(repo.path().join(".git").join("shallow"), "0".repeat(40)).unwrap();
    assert!(read_ref_state(&repo.handle(), &clock()).unwrap().is_shallow);
}

#[test]
fn the_basis_is_stable_and_moves_with_every_member_of_the_tuple() {
    let repo = TestRepo::init();
    repo.write("a.txt", b"one\n");
    repo.commit("first");
    let handle = repo.handle();
    let base = ref_fingerprint(&handle).unwrap();
    assert_eq!(
        base,
        ref_fingerprint(&handle).unwrap(),
        "two reads must agree"
    );
    // R5: a lowercase-hex SHA-256, because `location.refstate_basis` is TEXT.
    assert_eq!(base.as_str().len(), 64);
    assert!(base
        .as_str()
        .bytes()
        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)));

    repo.commit("second");
    let after_commit = ref_fingerprint(&handle).unwrap();
    assert_ne!(
        base, after_commit,
        "a moved branch tip must change the basis"
    );

    repo.git(&["tag", "v9"]);
    let after_tag = ref_fingerprint(&handle).unwrap();
    assert_ne!(
        after_commit, after_tag,
        "a new loose ref must change the basis"
    );

    std::fs::write(repo.path().join(".git").join("FETCH_HEAD"), b"x").unwrap();
    let after_fetch = ref_fingerprint(&handle).unwrap();
    assert_ne!(after_tag, after_fetch, "a fetch must change the basis");
}

// R5: the basis round-trips through `refstate_basis TEXT`, and only a real digest gets in.
#[test]
fn the_basis_round_trips_through_text_and_rejects_a_non_digest() {
    let repo = TestRepo::init();
    repo.write("a.txt", b"one\n");
    repo.commit("first");
    let base = ref_fingerprint(&repo.handle()).unwrap();

    let stored: String = base.to_hex();
    assert_eq!(RefFingerprint::from_hex(&stored), Some(base));

    assert_eq!(RefFingerprint::from_hex("not a digest"), None);
    assert_eq!(RefFingerprint::from_hex(""), None);
    assert_eq!(
        RefFingerprint::from_hex(&stored.to_uppercase()),
        None,
        "lowercase hex only"
    );
    assert_eq!(
        RefFingerprint::from_hex(&stored[..63]),
        None,
        "64 characters or nothing"
    );
}

// §6's finding, held mechanically: editing a tracked file moves neither the index nor the basis.
#[test]
fn editing_a_tracked_file_does_not_move_the_ref_basis() {
    let repo = TestRepo::init();
    repo.write("a.txt", b"one\n");
    repo.commit("first");
    let handle = repo.handle();
    let before = ref_fingerprint(&handle).unwrap();
    repo.write("a.txt", b"edited\n");
    assert_eq!(before, ref_fingerprint(&handle).unwrap());
}

// The observation fingerprint is wider on purpose: it must notice the index moving.
#[test]
fn the_observation_fingerprint_covers_the_index() {
    let repo = TestRepo::init();
    repo.write("a.txt", b"one\n");
    repo.commit("first");
    let handle = repo.handle();
    let before = observation_fingerprint(&handle).unwrap();
    let ref_before = ref_fingerprint(&handle).unwrap();

    repo.write("b.txt", b"two\n");
    repo.git(&["add", "b.txt"]);

    assert_eq!(
        ref_before,
        ref_fingerprint(&handle).unwrap(),
        "staging moves no ref"
    );
    assert_ne!(
        before,
        observation_fingerprint(&handle).unwrap(),
        "staging moves the index"
    );
}

use codotheca_core::cancel::CancelToken;
use codotheca_core::git::{divergence, Divergence, RunLimits};

#[test]
fn no_upstream_yields_none_and_never_a_zero() {
    let repo = TestRepo::init();
    repo.write("a.txt", b"one\n");
    repo.commit("first");
    let handle = repo.handle();
    let state = read_ref_state(&handle, &clock()).unwrap();
    let d = divergence(
        &repo.exec(),
        &handle,
        &state,
        RunLimits::none(),
        &CancelToken::new(),
    )
    .unwrap();
    assert_eq!(d, None, "no upstream is not computed, never 0/0");
}

#[test]
fn equal_tips_are_a_measured_zero_without_a_walk() {
    let repo = TestRepo::init();
    repo.write("a.txt", b"one\n");
    repo.commit("first");
    let oid = repo.git(&["rev-parse", "HEAD"]).trim().to_owned();
    repo.git(&["update-ref", "refs/remotes/origin/main", &oid]);
    repo.git(&["config", "branch.main.remote", "origin"]);
    repo.git(&["config", "branch.main.merge", "refs/heads/main"]);

    let handle = repo.handle();
    let state = read_ref_state(&handle, &clock()).unwrap();
    let d = divergence(
        &repo.exec(),
        &handle,
        &state,
        RunLimits::none(),
        &CancelToken::new(),
    )
    .unwrap();
    assert_eq!(
        d,
        Some(Divergence {
            ahead: 0,
            behind: 0
        })
    );
}

#[test]
fn counts_are_oriented_ahead_of_upstream_and_behind_it() {
    let repo = TestRepo::init();
    repo.write("a.txt", b"one\n");
    repo.commit("base");
    let base = repo.git(&["rev-parse", "HEAD"]).trim().to_owned();
    repo.git(&["update-ref", "refs/remotes/origin/main", &base]);
    repo.git(&["config", "branch.main.remote", "origin"]);
    repo.git(&["config", "branch.main.merge", "refs/heads/main"]);

    // Two local commits the upstream has not seen.
    repo.commit("local one");
    repo.commit("local two");
    // One upstream commit the branch has not seen, built on the shared base.
    repo.git(&["checkout", "-q", "--detach", &base]);
    repo.commit("remote one");
    let remote_tip = repo.git(&["rev-parse", "HEAD"]).trim().to_owned();
    repo.git(&["update-ref", "refs/remotes/origin/main", &remote_tip]);
    repo.git(&["checkout", "-q", "main"]);

    let handle = repo.handle();
    let state = read_ref_state(&handle, &clock()).unwrap();
    let d = divergence(
        &repo.exec(),
        &handle,
        &state,
        RunLimits::none(),
        &CancelToken::new(),
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        d,
        Divergence {
            ahead: 2,
            behind: 1
        }
    );
}
