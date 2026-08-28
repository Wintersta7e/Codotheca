#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! §3.5: bounded backoff on a held lock, and the guard that discards a torn read.

mod support;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use codotheca_core::cancel::CancelToken;
use codotheca_core::clock::Clock;
use codotheca_core::git::{
    busy_marker, defer_while_locked, observe_stable, Backoff, BusyMarker, GitError,
};
use support::TestRepo;

/// A clock that records every delay instead of taking it, so backoff is asserted, not waited on.
///
/// R3: `monotonic_ms` is a `u64` precisely so a fake like this one can produce it — it advances
/// by exactly the delays that were requested. An `Instant` has no public constructor and cannot
/// be moved, so the old `now_instant()` could only ever return the real time.
#[derive(Debug, Default)]
struct RecordingClock {
    slept: Mutex<Vec<Duration>>,
}

impl Clock for RecordingClock {
    fn now_unix(&self) -> i64 {
        1_800_000_000
    }
    fn monotonic_ms(&self) -> u64 {
        self.slept.lock().map_or(0, |v| {
            v.iter()
                .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
                .sum()
        })
    }
    fn sleep(&self, dur: Duration) {
        if let Ok(mut v) = self.slept.lock() {
            v.push(dur);
        }
    }
}

#[test]
fn the_backoff_is_bounded_and_exponential() {
    let d = Backoff::bounded().delays();
    assert_eq!(
        d,
        vec![
            Duration::from_millis(25),
            Duration::from_millis(50),
            Duration::from_millis(100),
            Duration::from_millis(200),
            Duration::from_millis(400),
            Duration::from_millis(800),
        ],
        "one 200 ms retry is meaningless against a rebase"
    );
    let total: Duration = d.iter().sum();
    assert_eq!(total, Duration::from_millis(1_575), "and it is bounded");
}

#[test]
fn every_marker_is_recognised() {
    let repo = TestRepo::init();
    repo.write("a.txt", b"one\n");
    repo.commit("first");
    let handle = repo.handle();
    let git_dir = repo.path().join(".git");
    assert_eq!(busy_marker(&handle), None);

    for (file, expected) in [
        ("index.lock", BusyMarker::IndexLock),
        ("MERGE_HEAD", BusyMarker::Merge),
        ("REBASE_HEAD", BusyMarker::Rebase),
        ("CHERRY_PICK_HEAD", BusyMarker::CherryPick),
        ("REVERT_HEAD", BusyMarker::Revert),
        ("BISECT_LOG", BusyMarker::Bisect),
    ] {
        std::fs::write(git_dir.join(file), b"").unwrap();
        assert_eq!(busy_marker(&handle), Some(expected), "{file}");
        std::fs::remove_file(git_dir.join(file)).unwrap();
    }
    std::fs::create_dir_all(git_dir.join("rebase-merge")).unwrap();
    assert_eq!(busy_marker(&handle), Some(BusyMarker::Rebase));
}

#[test]
fn a_held_lock_backs_off_the_full_sequence_and_then_defers() {
    let repo = TestRepo::init();
    repo.write("a.txt", b"one\n");
    repo.commit("first");
    std::fs::write(repo.path().join(".git").join("index.lock"), b"").unwrap();

    let clock = RecordingClock::default();
    let mut ran = 0u32;
    let err = defer_while_locked(&repo.handle(), &clock, &CancelToken::new(), &mut || {
        ran += 1;
        Ok(())
    })
    .unwrap_err();

    assert_eq!(
        err,
        GitError::Busy {
            marker: BusyMarker::IndexLock
        }
    );
    assert!(
        err.is_deferral(),
        "a held lock is a deferral, never a project error"
    );
    assert_eq!(ran, 0, "the work never ran while the lock was held");
    assert_eq!(
        clock.slept.lock().unwrap().len(),
        6,
        "the whole bounded sequence was tried"
    );
}

#[test]
fn a_lock_released_mid_backoff_lets_the_work_run() {
    let repo = TestRepo::init();
    repo.write("a.txt", b"one\n");
    repo.commit("first");
    let lock = repo.path().join(".git").join("index.lock");
    std::fs::write(&lock, b"").unwrap();

    let released = Arc::new(lock);
    let to_release = Arc::clone(&released);
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(120));
        let _ = std::fs::remove_file(to_release.as_path());
    });

    let clock = codotheca_core::clock::SystemClock::new();
    let mut ran = 0u32;
    let out = defer_while_locked(&repo.handle(), &clock, &CancelToken::new(), &mut || {
        ran += 1;
        Ok(42)
    })
    .unwrap();
    assert_eq!(out, 42);
    assert_eq!(ran, 1);
}

#[test]
fn cancellation_beats_the_backoff() {
    let repo = TestRepo::init();
    repo.write("a.txt", b"one\n");
    repo.commit("first");
    std::fs::write(repo.path().join(".git").join("index.lock"), b"").unwrap();
    let cancel = CancelToken::new();
    cancel.cancel();
    let err = defer_while_locked(
        &repo.handle(),
        &RecordingClock::default(),
        &cancel,
        &mut || Ok(()),
    )
    .unwrap_err();
    assert_eq!(err, GitError::Cancelled);
}

#[test]
fn a_stable_repository_yields_an_observation_with_a_basis() {
    let repo = TestRepo::init();
    repo.write("a.txt", b"one\n");
    repo.commit("first");
    let clock = RecordingClock::default();
    let obs = observe_stable(&repo.handle(), &clock, &mut || Ok("value")).unwrap();
    assert_eq!(obs.value, "value");
    assert_eq!(obs.observed_at, 1_800_000_000);
    assert_eq!(
        obs.basis.as_str().len(),
        64,
        "R5: hex SHA-256, matching refstate_basis TEXT"
    );
}

// §3.5: if HEAD, refs or the index moved, the result is discarded rather than stored.
#[test]
fn a_commit_during_the_observation_is_a_torn_read() {
    let repo = TestRepo::init();
    repo.write("a.txt", b"one\n");
    repo.commit("first");
    let err = observe_stable(&repo.handle(), &RecordingClock::default(), &mut || {
        repo.write("a.txt", b"two\n");
        repo.commit("moved underneath the observation");
        Ok(())
    })
    .unwrap_err();
    assert_eq!(err, GitError::TornRead);
    assert!(err.is_deferral());
    assert_eq!(err.protocol_code(), None);
}

#[test]
fn a_staged_file_during_the_observation_is_also_a_torn_read() {
    let repo = TestRepo::init();
    repo.write("a.txt", b"one\n");
    repo.commit("first");
    let err = observe_stable(&repo.handle(), &RecordingClock::default(), &mut || {
        repo.write("b.txt", b"two\n");
        repo.git(&["add", "b.txt"]);
        Ok(())
    })
    .unwrap_err();
    assert_eq!(
        err,
        GitError::TornRead,
        "the index is in the observation fingerprint"
    );
}
