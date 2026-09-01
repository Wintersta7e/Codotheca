#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! §11.2a's index-fatal report, and the startup sequence that runs before `run_loop`.

use codotheca_core::assembly::startup::open_index;
use codotheca_core::surfaces::startup_failure::{EXIT_INDEX_FATAL, STARTUP_FAILURE_FILE};

const NOW: i64 = 1_760_000_000;

/// Re-invokes this test binary to run one `#[test]` in a child process.
///
/// The fatal path calls `std::process::exit`, so it cannot be asserted in-process: there would
/// be no process left to assert in. This is the standard way to test an exiting path, and it is
/// the only way to observe the exit *code*, which is the part the shell actually reads.
fn run_in_child(test_name: &str, dir: &std::path::Path) -> std::process::ExitStatus {
    std::process::Command::new(std::env::current_exe().expect("test binary"))
        .args([test_name, "--exact", "--nocapture"])
        .env("CODOTHECA_CHILD", "1")
        .env("CODOTHECA_CHILD_DIR", dir)
        .status()
        .expect("child runs")
}

fn is_child() -> bool {
    std::env::var_os("CODOTHECA_CHILD").is_some()
}

fn child_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(std::env::var_os("CODOTHECA_CHILD_DIR").expect("child dir"))
}

/// Puts a `user_version` on disk that no migration in this build supports.
fn write_index_from_the_future(dir: &std::path::Path) {
    // `Index::db_path`, never a literal: the filename is the index's to own, and writing it
    // twice is how a fixture ends up creating a second, empty database and passing vacuously.
    let conn =
        rusqlite::Connection::open(codotheca_core::index::Index::db_path(dir)).expect("create");
    conn.pragma_update(None, "user_version", 999_i64)
        .expect("user_version");
}

#[test]
fn a_schema_from_the_future_writes_the_report_and_does_not_return() {
    if is_child() {
        // Must not return. Reaching the line below is the failure this test exists to catch.
        let _ = open_index(&child_dir(), NOW);
        eprintln!("open_index returned on a schema from the future");
        std::process::exit(99);
    }

    let dir = tempfile::tempdir().expect("tmp");
    write_index_from_the_future(dir.path());

    let status = run_in_child(
        "a_schema_from_the_future_writes_the_report_and_does_not_return",
        dir.path(),
    );
    assert_eq!(
        status.code(),
        Some(i32::from(EXIT_INDEX_FATAL)),
        "the shell reads the exit code, so it is the part that must be exact"
    );

    let report = dir.path().join(STARTUP_FAILURE_FILE);
    let doc: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&report).expect("report written")).expect("json");
    assert_eq!(
        doc["kind"], "schema_from_future",
        "the shell draws a window from this, so the kind must survive: {doc}"
    );
}

#[test]
fn a_clean_open_clears_a_stale_report() {
    if is_child() {
        return;
    }
    let dir = tempfile::tempdir().expect("tmp");
    std::fs::write(dir.path().join(STARTUP_FAILURE_FILE), b"{}").expect("stale report");

    let _index = open_index(dir.path(), NOW).expect("opens");
    assert!(
        !dir.path().join(STARTUP_FAILURE_FILE).exists(),
        "yesterday's failure must not draw over today's working app"
    );
}

#[test]
fn a_clean_open_with_no_stale_report_is_untroubled_by_its_absence() {
    if is_child() {
        return;
    }
    let dir = tempfile::tempdir().expect("tmp");
    let _index = open_index(dir.path(), NOW).expect("opens");
    assert!(!dir.path().join(STARTUP_FAILURE_FILE).exists());
}
