//! Compiled only under `testkit`: these link `codotheca_core::testing`, which the feature gates.
#![cfg(feature = "testkit")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

//! Plan 06 Task 5's doubles, against plan 05's actual trait. That trait is one method per
//! operation with no argv at the boundary, so the task's argv-matching rules cannot exist here;
//! §3.2's neutralising options and criterion 63's `-c safe.directory=` are proven directly in
//! `git_invocation.rs`, against the argv builder itself.

use std::sync::Arc;

use codotheca_core::cancel::CancelToken;
use codotheca_core::clock::Clock;
use codotheca_core::git::{
    GitBackend, GitError, GitVersion, JobClass, JobContext, RepoHandle, StoreKey,
};
use codotheca_core::mount::StoreClass;
use codotheca_core::testing::{FakeClock, FakeGitBackend, GitReply, RecordingGitBackend};

fn handle(dir: &str) -> RepoHandle {
    let path = std::path::PathBuf::from(dir);
    RepoHandle {
        work_dir: path.clone(),
        git_dir: path.join(".git"),
        common_dir: path.join(".git"),
        store: StoreKey::new("store-a"),
        store_class: StoreClass::Local,
        trusted: false,
    }
}

fn version() -> GitVersion {
    GitVersion {
        major: 2,
        minor: 43,
        patch: 0,
        raw: "git version 2.43.0".to_owned(),
    }
}

#[test]
fn an_unconfigured_operation_fails_rather_than_inventing_a_value() {
    let git = FakeGitBackend::new();
    let cancel = CancelToken::new();
    let ctx = JobContext::new(JobClass::Interactive, &cancel, None);
    // A fake that answered "no commits" or "clean" by default would teach a test the wrong
    // thing, and absence is never the same as a computed zero.
    assert!(matches!(
        git.ref_state(&handle("/c/thing"), &ctx),
        Err(GitError::Unreadable { .. })
    ));
}

#[test]
fn a_reply_scoped_to_one_repository_does_not_fire_for_another() {
    let git = FakeGitBackend::new();
    let cancel = CancelToken::new();
    let ctx = JobContext::new(JobClass::Interactive, &cancel, None);
    git.on_version(
        Some("huge-untracked"),
        GitReply::Err(GitError::Budget { after_ms: 500 }),
    );
    git.always_version(GitReply::Ok(version()));
    // `version` takes no repo, so a scoped rule can never match it and the fallback answers.
    assert!(git.version(&ctx).is_ok());
}

#[test]
fn two_queued_replies_make_a_torn_read() {
    let git = FakeGitBackend::new();
    let cancel = CancelToken::new();
    let ctx = JobContext::new(JobClass::Interactive, &cancel, None);
    git.on_version(None, GitReply::Ok(version()));
    git.on_version(None, GitReply::Err(GitError::TornRead));

    assert!(git.version(&ctx).is_ok(), "the first observation succeeds");
    assert!(
        matches!(git.version(&ctx), Err(GitError::TornRead)),
        "the second sees the state move under it"
    );
}

#[test]
fn a_slow_reply_advances_the_injected_clock_instead_of_sleeping() {
    let clock = Arc::new(FakeClock::new(1_700_000_000));
    let git = FakeGitBackend::with_clock(Arc::clone(&clock));
    let cancel = CancelToken::new();
    let ctx = JobContext::new(JobClass::Interactive, &cancel, None);
    git.on_version(
        None,
        GitReply::Slow {
            delay_ms: 5_000,
            then: Box::new(GitReply::Ok(version())),
        },
    );

    let started = std::time::Instant::now();
    assert!(git.version(&ctx).is_ok());
    assert_eq!(clock.monotonic_ms(), 5_000);
    assert!(
        started.elapsed().as_millis() < 200,
        "a test for a 5 s budget must not take 5 s"
    );
}

#[test]
fn every_call_is_recorded_with_its_operation_and_repository() {
    let git = FakeGitBackend::new();
    let cancel = CancelToken::new();
    let ctx = JobContext::new(JobClass::Interactive, &cancel, None);
    let _ = git.ref_state(&handle("/c/bare"), &ctx);
    let calls = git.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].op, "ref_state");
    assert_eq!(
        calls[0].repo.as_deref(),
        Some(std::path::Path::new("/c/bare"))
    );
    git.clear();
    assert!(git.calls().is_empty());
}

#[test]
fn the_recorder_reports_what_it_forwarded() {
    let inner = FakeGitBackend::new();
    inner.always_version(GitReply::Ok(version()));
    let git = RecordingGitBackend::new(inner);
    let cancel = CancelToken::new();
    let ctx = JobContext::new(JobClass::Interactive, &cancel, None);
    assert!(git.version(&ctx).is_ok());
    assert_eq!(git.calls().len(), 1);
    assert_eq!(git.calls()[0].op, "version");
}

/// The fake half of the new seam method, in the same change as the trait and `SystemGit`'s
/// implementation. The rule exists because four rulings — R1, R35a, R40, R46 — each came from a
/// trait that got its fake and never its real impl, and each of those compiled.
#[test]
fn the_fake_returns_the_remotes_it_was_seeded_with_and_refuses_when_it_was_not() {
    let git = FakeGitBackend::new();
    let cancel = CancelToken::new();
    let ctx = JobContext::new(JobClass::Interactive, &cancel, None);

    assert!(
        matches!(
            git.remote_urls(&handle("/c/thing"), &ctx),
            Err(GitError::Unreadable { .. })
        ),
        "an unseeded fake must not answer `no remotes`: that is a real §1.1 fact, not a default"
    );

    let seeded = vec![(
        "origin".to_owned(),
        "https://example.invalid/one.git".to_owned(),
    )];
    git.on_remote_urls(Some("thing"), GitReply::Ok(seeded.clone()));
    assert_eq!(git.remote_urls(&handle("/c/thing"), &ctx).unwrap(), seeded);
}

/// The recorder passes the new method through rather than swallowing it, so a test that counts
/// git calls still counts this one.
#[test]
fn the_recorder_reports_a_remote_read() {
    let inner = FakeGitBackend::new();
    inner.always_remote_urls(GitReply::Ok(Vec::new()));
    let recorder = RecordingGitBackend::new(inner);
    let cancel = CancelToken::new();
    let ctx = JobContext::new(JobClass::Interactive, &cancel, None);
    assert!(recorder.remote_urls(&handle("/c/thing"), &ctx).is_ok());
    assert!(recorder.calls().iter().any(|c| c.op == "remote_urls"));
}
