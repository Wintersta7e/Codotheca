#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! Spawning a launch target: the exit waiter, a detached child, our stdout never inherited, and
//! a launched program that outlives the core.

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

/// Set, this test binary sleeps a minute: the program a launch starts.
#[cfg(windows)]
const SLEEPER_ENV: &str = "CODOTHECA_TEST_SLEEP";
/// Set, this test binary stands in for the core: it joins its own job and launches.
#[cfg(windows)]
const CORE_ENV: &str = "CODOTHECA_TEST_LAUNCH_THEN_WAIT";
/// What the stand-in prints, followed by three pids, once everything is started.
#[cfg(windows)]
const CORE_READY: &str = "launched";
/// How long a process may take to end once the job holding it has closed.
#[cfg(windows)]
const SETTLE: std::time::Duration = std::time::Duration::from_secs(10);

/// Not a check of its own: with [`SLEEPER_ENV`] set it is a program that runs for a minute.
#[cfg(windows)]
#[test]
fn helper_sleeps_for_a_minute() {
    if std::env::var_os(SLEEPER_ENV).is_some() {
        std::thread::sleep(std::time::Duration::from_secs(60));
    }
}

#[cfg(windows)]
fn sleeper(wait: bool) -> Invocation {
    Invocation {
        program: std::env::current_exe().unwrap(),
        argv: ["--exact", "helper_sleeps_for_a_minute"]
            .iter()
            .map(std::ffi::OsString::from)
            .collect(),
        cwd: None,
        env: vec![(SLEEPER_ENV.to_owned(), "1".to_owned())],
        wait,
    }
}

/// Not a check of its own: with [`CORE_ENV`] set it joins its own job as the core does at
/// startup, launches one detached and one waited-on program through the production spawner, and
/// starts one plain child that stays in the job, then waits to be killed.
#[cfg(windows)]
#[test]
fn helper_joins_its_own_job_and_launches() {
    if std::env::var_os(CORE_ENV).is_none() {
        return;
    }
    codotheca_core::lifecycle::join_own_job().expect("the job is joined");
    let detached = OsSpawner.spawn(&sleeper(false), "sleeper").expect("spawn");
    let waited = OsSpawner.spawn(&sleeper(true), "sleeper").expect("spawn");
    let mut plain = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "helper_sleeps_for_a_minute"])
        .env(SLEEPER_ENV, "1")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn");
    eprintln!(
        "{CORE_READY} {} {} {}",
        detached.pid,
        waited.pid,
        plain.id()
    );
    // Killed long before the minute is up; the wait is only for a run that is not.
    let _ = plain.wait();
}

#[cfg(windows)]
fn alive(pid: u32) -> bool {
    let out = std::process::Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/NH", "/FO", "CSV"])
        .stdin(std::process::Stdio::null())
        .output()
        .expect("tasklist runs");
    assert!(out.status.success(), "the process listing failed: {out:?}");
    String::from_utf8_lossy(&out.stdout).contains(&format!("\",\"{pid}\",\""))
}

/// A program the user launched outlives a core killed outright, waited on or not: the core's
/// kill-on-close job ends every child that stayed in it — the plain child proves the job is
/// there — and a launch leaves the job as it is created.
#[cfg(windows)]
#[test]
fn a_launched_program_outlives_a_killed_core() {
    let mut core = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "helper_joins_its_own_job_and_launches",
            "--nocapture",
        ])
        .env(CORE_ENV, "1")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("stand-in spawns");
    let stderr = core.stderr.take().unwrap();
    let ready = std::io::BufRead::lines(std::io::BufReader::new(stderr))
        .map_while(Result::ok)
        .find(|line| line.starts_with(CORE_READY))
        .expect("the stand-in never launched");
    let pids: Vec<u32> = ready
        .split_whitespace()
        .skip(1)
        .map(|p| p.parse().unwrap())
        .collect();
    let [detached, waited, plain] = pids[..] else {
        panic!("three pids expected: {ready}")
    };
    assert!(
        alive(detached) && alive(waited) && alive(plain),
        "the listing does not see the stand-in's children"
    );

    core.kill().expect("kill");
    core.wait().expect("wait");
    let until = std::time::Instant::now() + SETTLE;
    while alive(plain) && std::time::Instant::now() < until {
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    let survived = (alive(detached), alive(waited), alive(plain));
    for pid in [detached, waited, plain] {
        // Ours: started by this test; one pid, never a tree.
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/F"])
            .stdin(std::process::Stdio::null())
            .output();
    }
    assert_eq!(
        survived,
        (true, true, false),
        "(detached launch, waited launch, plain child) alive after the core was killed"
    );
}
