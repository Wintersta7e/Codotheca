#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! §3.4: the process-group spawn and the duplex pipe shape that cannot deadlock.

mod support;

use std::ffi::OsStr;
use std::io::{BufRead, Write};
use std::sync::mpsc;
use std::time::Duration;

use codotheca_core::cancel::CancelToken;
use codotheca_core::git::{GitError, RunLimits};
use support::TestRepo;

#[test]
fn runs_a_subcommand_and_returns_its_stdout() {
    let repo = TestRepo::init();
    let out = repo
        .exec()
        .run(
            &repo.handle(),
            &[OsStr::new("rev-parse"), OsStr::new("--is-bare-repository")],
            RunLimits::none(),
            &CancelToken::new(),
        )
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "false");
}

#[test]
fn a_failing_subcommand_is_classified_from_its_stderr() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join(".git")).unwrap();
    let repo = codotheca_core::git::RepoHandle::resolve(
        tmp.path(),
        codotheca_core::git::StoreKey::new("s"),
        codotheca_core::mount::StoreClass::Local,
    )
    .unwrap();
    let hooks = codotheca_core::git::ensure_empty_hooks_dir(tmp.path()).unwrap();
    let err = codotheca_core::git::GitExec::system(hooks)
        .run(
            &repo,
            &[OsStr::new("rev-parse"), OsStr::new("HEAD")],
            RunLimits::none(),
            &CancelToken::new(),
        )
        .unwrap_err();
    assert!(matches!(err, GitError::Unreadable { .. }), "{err:?}");
}

// The five-hour bug, in its smallest reproducible form: 5,000 queries in and 5,000 answers out
// both exceed a 64 KiB pipe buffer, so a writer that finishes before a reader starts deadlocks.
// The 30-second channel timeout is what makes a regression a failure instead of a hung suite.
#[test]
fn duplex_pipes_do_not_deadlock_when_both_directions_exceed_the_buffer() {
    let repo = TestRepo::init();
    repo.write("a.txt", b"contents\n");
    repo.commit("one");
    let oid = repo.git(&["rev-parse", "HEAD:a.txt"]).trim().to_owned();

    let queries: Vec<String> = vec![oid; 5_000];
    let expected_in: usize = queries.iter().map(|q| q.len() + 1).sum();
    assert!(expected_in > 64 * 1024, "stdin must exceed a pipe buffer");

    let (tx, rx) = mpsc::channel();
    let handle = repo.handle();
    let exec = repo.exec();
    std::thread::spawn(move || {
        let got = exec.run_piped(
            &handle,
            &[OsStr::new("cat-file"), OsStr::new("--batch-check")],
            RunLimits::none(),
            &CancelToken::new(),
            move |stdin: &mut dyn Write| {
                for q in &queries {
                    stdin.write_all(q.as_bytes())?;
                    stdin.write_all(b"\n")?;
                }
                stdin.flush()
            },
            |stdout: &mut dyn BufRead| {
                let mut lines = 0usize;
                let mut bytes = 0usize;
                let mut line = String::new();
                loop {
                    line.clear();
                    let n = stdout.read_line(&mut line)?;
                    if n == 0 {
                        break;
                    }
                    bytes += n;
                    lines += 1;
                }
                Ok((lines, bytes))
            },
        );
        let _ = tx.send(got);
    });

    let result = rx
        .recv_timeout(Duration::from_secs(30))
        .expect("run_piped deadlocked: no result within 30s");
    let (lines, bytes) = result.unwrap();
    assert_eq!(lines, 5_000);
    assert!(
        bytes > 64 * 1024,
        "stdout must exceed a pipe buffer, got {bytes}"
    );
}

#[test]
fn a_deadline_kills_the_child_and_reports_budget() {
    let repo = TestRepo::init();
    repo.write("a.txt", b"x\n");
    repo.commit("one");
    let oid = repo.git(&["rev-parse", "HEAD:a.txt"]).trim().to_owned();

    let (tx, rx) = mpsc::channel();
    let handle = repo.handle();
    let exec = repo.exec();
    std::thread::spawn(move || {
        // A writer that trickles keeps `cat-file --batch-check` alive well past the deadline.
        let got = exec.run_piped(
            &handle,
            &[OsStr::new("cat-file"), OsStr::new("--batch-check")],
            RunLimits::after(Duration::from_millis(300)),
            &CancelToken::new(),
            move |stdin: &mut dyn Write| {
                for _ in 0..100 {
                    stdin.write_all(oid.as_bytes())?;
                    stdin.write_all(b"\n")?;
                    stdin.flush()?;
                    std::thread::sleep(Duration::from_millis(100));
                }
                Ok(())
            },
            |stdout: &mut dyn BufRead| {
                let mut sink = Vec::new();
                stdout.read_to_end(&mut sink)?;
                Ok(sink.len())
            },
        );
        let _ = tx.send(got);
    });

    let err = rx
        .recv_timeout(Duration::from_secs(20))
        .expect("the deadline did not fire")
        .unwrap_err();
    assert!(matches!(err, GitError::Budget { .. }), "{err:?}");
}

#[test]
fn cancellation_stops_a_running_child() {
    let repo = TestRepo::init();
    repo.write("a.txt", b"x\n");
    repo.commit("one");
    let oid = repo.git(&["rev-parse", "HEAD:a.txt"]).trim().to_owned();

    let cancel = CancelToken::new();
    let watcher = cancel.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(200));
        watcher.cancel();
    });

    let err = repo
        .exec()
        .run_piped(
            &repo.handle(),
            &[OsStr::new("cat-file"), OsStr::new("--batch-check")],
            RunLimits::none(),
            &cancel,
            move |stdin: &mut dyn Write| {
                for _ in 0..100 {
                    stdin.write_all(oid.as_bytes())?;
                    stdin.write_all(b"\n")?;
                    stdin.flush()?;
                    std::thread::sleep(Duration::from_millis(100));
                }
                Ok(())
            },
            |stdout: &mut dyn BufRead| {
                let mut sink = Vec::new();
                stdout.read_to_end(&mut sink)?;
                Ok(sink.len())
            },
        )
        .unwrap_err();
    assert_eq!(err, GitError::Cancelled);
}

// The orphan trap: a killed child left eight live grandchildren holding file locks. The alias
// records the pid of the process that outlives its parent, and the kill must take it too.
#[cfg(target_os = "linux")]
#[test]
fn killing_a_child_kills_its_grandchildren() {
    let repo = TestRepo::init();
    repo.write("a.txt", b"x\n");
    repo.commit("one");
    let pidfile = repo.path().join("grandchild.pid");

    let alias = format!(
        "alias.slowtree=!echo $$ > {} && exec sleep 60",
        pidfile.to_string_lossy()
    );
    let err = repo
        .exec()
        .run(
            &repo.handle(),
            &[OsStr::new("-c"), OsStr::new(&alias), OsStr::new("slowtree")],
            RunLimits::after(Duration::from_millis(800)),
            &CancelToken::new(),
        )
        .unwrap_err();
    assert!(matches!(err, GitError::Budget { .. }), "{err:?}");

    let pid: u32 = std::fs::read_to_string(&pidfile)
        .expect("the alias never recorded a pid")
        .trim()
        .parse()
        .unwrap();
    let mut alive = true;
    for _ in 0..50 {
        if !std::path::Path::new(&format!("/proc/{pid}")).exists() {
            alive = false;
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(!alive, "grandchild {pid} survived the process-tree kill");
}
