#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! §47.3's deadline on the write path, and the group kill that makes it real (§37.8).
//!
//! The verifying read runs while a user waits. Before this, the wait loop in `WriteExec::run`
//! checked `cancel` and nothing else, and nothing ever fired it: an endpoint that accepted the
//! connection and never answered held the core's handler for as long as git cared to wait.

mod support;

use std::io::Read as _;
use std::net::TcpListener;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use codotheca_core::cancel::CancelToken;
use codotheca_core::git::GitError;
use codotheca_core::gitw::intent::GIT_INVOCATION_DEADLINE;
use codotheca_core::gitw::{Intent, MutatingGit, RemoteName, SystemMutatingGit};
use support::TestRepo;

/// What the test allows beyond the deadline for the kill and the wait to land, named so the
/// assertion says which part of the time is the product's and which is this test's.
const SLACK: Duration = Duration::from_secs(5);

/// How long the socket may stay open after the call returns before a survivor is declared.
const SURVIVOR_WINDOW: Duration = Duration::from_secs(2);

/// **AC-P4-47-10's Lane-0 clause.** An https endpoint that accepts and never writes: the child
/// is killed at `GIT_INVOCATION_DEADLINE`, the call returns `Budget` inside the named slack, and
/// no process of the group survives to hold the connection open.
///
/// The survivor check reads the accepted socket: every byte the client sent drains, and then the
/// socket must reach end-of-file within two seconds. A `git-remote-https` that outlived the kill
/// would keep it open, and the read would time out instead.
#[test]
fn a_stalled_child_is_killed_at_its_deadline_and_leaves_no_survivor() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind a local endpoint");
    let port = listener.local_addr().expect("address").port();
    let (sock_tx, sock_rx) = mpsc::channel();
    std::thread::spawn(move || {
        // Accept once, and never write a byte.
        if let Ok((stream, _)) = listener.accept() {
            let _ = sock_tx.send(stream);
        }
    });

    let repo = TestRepo::init();
    repo.write("a.txt", b"one");
    repo.commit("one");
    repo.git(&[
        "remote",
        "add",
        "origin",
        &format!("https://127.0.0.1:{port}/x.git"),
    ]);
    let hooks = repo.scratch().join("hooks-empty");
    std::fs::create_dir_all(&hooks).expect("hooks dir");

    let intent = Intent::Fetch {
        work_dir: repo.path().to_path_buf(),
        remote: RemoteName::parse("origin").expect("remote"),
    };
    assert_eq!(intent.deadline(), Some(GIT_INVOCATION_DEADLINE));

    let (done_tx, done_rx) = mpsc::channel();
    let started = Instant::now();
    std::thread::spawn(move || {
        let backend = SystemMutatingGit::new(std::path::PathBuf::from("git"), hooks);
        let outcome = backend.run(&intent, &CancelToken::new(), &mut |_| {});
        let _ = done_tx.send(outcome);
    });

    // The test's own watchdog: the shipped loop had no deadline and blocked here.
    let watchdog = GIT_INVOCATION_DEADLINE + Duration::from_secs(30);
    let outcome = done_rx
        .recv_timeout(watchdog)
        .unwrap_or_else(|_| panic!("the write child is still running after {watchdog:?}"));
    let elapsed = started.elapsed();
    eprintln!(
        "deadline: returned {outcome:?} after {elapsed:?}; deadline {GIT_INVOCATION_DEADLINE:?}, \
         slack {SLACK:?}"
    );
    assert!(
        matches!(outcome, Err(GitError::Budget { .. })),
        "a stalled child must end as Budget, got {outcome:?}"
    );
    assert!(
        elapsed <= GIT_INVOCATION_DEADLINE + SLACK,
        "returned after {elapsed:?}, past the deadline plus {SLACK:?}"
    );

    let mut stream = sock_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("git connected to the endpoint, so the stall was the network and not a refusal");
    stream
        .set_read_timeout(Some(SURVIVOR_WINDOW))
        .expect("read timeout");
    let closed_at = Instant::now();
    let mut buf = [0_u8; 4096];
    loop {
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::ConnectionReset => break,
            Err(e) => panic!(
                "the connection stayed open {:?} after the call returned — a process of the \
                 group survived the kill ({e})",
                closed_at.elapsed()
            ),
        }
    }
    eprintln!(
        "deadline: the endpoint saw end-of-file {:?} after the return",
        closed_at.elapsed()
    );
}
