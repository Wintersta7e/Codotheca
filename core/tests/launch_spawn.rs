#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use codotheca_core::launch::argv::Invocation;
use codotheca_core::launch::spawn::{OsSpawner, Spawner};

#[cfg(unix)]
fn inv(program: &str, args: &[&str], wait: bool) -> Invocation {
    Invocation {
        program: std::path::PathBuf::from(program),
        argv: args.iter().map(std::ffi::OsString::from).collect(),
        cwd: None,
        env: vec![],
        wait,
    }
}

// The three tests below drive a real child through `/bin/sh`, which is the *test's* way of
// producing a controllable exit code — the product never shells out, which the source guard at
// the bottom of this file proves. They are `cfg(unix)` because `/bin/sh` does not exist on the
// other target, matching the nine existing platform-scoped tests in this crate.

#[cfg(unix)]
#[test]
fn a_waiting_child_reports_its_exit_through_the_waiter() {
    let spawned = OsSpawner
        .spawn(&inv("/bin/sh", &["-c", "exit 7"], true), "/bin/sh")
        .expect("spawn");
    let code = spawned
        .waiter
        .expect("a waiting child has a waiter")
        .join()
        .expect("join");
    assert_eq!(code, Some(7));
}

#[cfg(unix)]
#[test]
fn a_detached_child_has_no_waiter_and_still_reports_a_pid() {
    let spawned = OsSpawner
        .spawn(&inv("/bin/sh", &["-c", "exit 0"], false), "/bin/sh")
        .expect("spawn");
    assert!(spawned.pid > 0);
    assert!(spawned.waiter.is_none());
}

#[cfg(unix)]
#[test]
fn the_child_never_inherits_our_stdout() {
    // `printf` writing to an inherited stdout would corrupt the protocol stream (§2.1).
    let spawned = OsSpawner
        .spawn(
            &inv("/bin/sh", &["-c", "printf x >&1; exit 0"], true),
            "/bin/sh",
        )
        .expect("spawn");
    assert_eq!(
        spawned.waiter.expect("waiter").join().expect("join"),
        Some(0)
    );
}

#[test]
fn a_missing_executable_fails_with_what_was_tried_rather_than_panicking() {
    let missing = Invocation {
        program: std::path::PathBuf::from("/nowhere/at/all"),
        argv: vec![],
        cwd: None,
        env: vec![],
        wait: false,
    };
    let err = OsSpawner
        .spawn(&missing, "/nowhere/at/all")
        .expect_err("must fail");
    let text = format!("{err:?}");
    assert!(
        text.contains("/nowhere/at/all"),
        "the failure must name what was tried (§11.5)"
    );
}

#[test]
fn no_source_file_in_this_crate_shells_out_to_run_a_target() {
    let mut offenders = Vec::new();
    let mut scanned = 0_usize;
    let mut stack = vec![std::path::PathBuf::from("src")];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("read_dir").flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            scanned += 1;
            let text = std::fs::read_to_string(&path).unwrap_or_default();
            if text.contains("\"sh\", \"-c\"") || text.contains("cmd.exe\", \"/c") {
                offenders.push(path);
            }
        }
    }
    // A guard whose passing run scans zero files is a failing guard, not a passing one.
    assert!(
        scanned > 20,
        "the source scan reached only {scanned} files; it is not looking at the crate"
    );
    assert!(
        offenders.is_empty(),
        "a launch path went through a shell: {offenders:?}"
    );
}
