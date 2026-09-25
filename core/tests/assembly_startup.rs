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
        // A code of its own, so the parent's failure says this line was reached rather than
        // libtest's 101, which any panic in the child would also produce.
        #[allow(clippy::exit)]
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

// ---------------------------------------------------------------------------
// Task 7: the composition, end to end. This is the whole point of the plan —
// the real binary answering a real command over the real transport.
// ---------------------------------------------------------------------------

mod binary {
    use codotheca_core::proto::frame::{read_frame, write_frame};
    use std::io::Write as _;

    fn spawn(dir: &std::path::Path) -> std::process::Child {
        std::process::Command::new(env!("CARGO_BIN_EXE_codotheca-core"))
            .arg(format!("--data-dir={}", dir.display()))
            .arg("--epoch=7")
            .arg(format!("--parent-pid={}", std::process::id()))
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("core spawns")
    }

    fn send(child: &mut std::process::Child, frame: &serde_json::Value) {
        let body = serde_json::to_vec(frame).expect("frame");
        let stdin = child.stdin.as_mut().expect("stdin");
        write_frame(stdin, &body).expect("write");
        stdin.flush().expect("flush");
    }

    fn recv(child: &mut std::process::Child) -> serde_json::Value {
        let stdout = child.stdout.as_mut().expect("stdout");
        let mut buf = Vec::new();
        read_frame(stdout, &mut buf).expect("read");
        serde_json::from_slice(&buf).expect("json")
    }

    /// Against `RefusingHandler` — what shipped before this plan — `targets.list` came back as an
    /// error frame. A `response` here is the assembly working end to end.
    #[test]
    fn the_binary_answers_a_command_after_the_handshake() {
        let dir = tempfile::tempdir().expect("tmp");
        let mut child = spawn(dir.path());

        let hello = recv(&mut child);
        assert_eq!(hello["t"], "hello", "{hello}");

        send(
            &mut child,
            &serde_json::json!({"t": "request", "id": 1, "command": "app.hello_ack", "args": {}}),
        );
        let ack = recv(&mut child);
        assert_eq!(ack["t"], "response", "{ack}");

        send(
            &mut child,
            &serde_json::json!({"t": "request", "id": 2, "command": "targets.list", "args": {}}),
        );
        let reply = recv(&mut child);
        assert_eq!(
            reply["t"], "response",
            "against RefusingHandler this is an error frame: {reply}"
        );

        send(
            &mut child,
            &serde_json::json!({"t": "request", "id": 3, "command": "app.shutdown", "args": {}}),
        );
        let status = child.wait().expect("exits");
        assert!(
            status.success(),
            "and it exits rather than hanging on the transport join: {status:?}"
        );
    }

    /// Startup seeds the identity set, and `identity.list` is what proves it against the binary.
    ///
    /// `identity::people::seed` had **no production caller**: the table was empty on every real
    /// machine, so J1.5 folded every repository against an empty set, wrote `is_reference = 1`
    /// for all of them, and §8.0b's base predicate — *a bare query returns no `is_reference`
    /// rows* — left `projects.list` answering zero rows over a full index. The unit tests could
    /// not see it; every one of them seeds by hand.
    #[test]
    fn startup_seeds_the_identity_set_from_the_user_s_git_configuration() {
        let dir = tempfile::tempdir().expect("tmp");
        let home = tempfile::tempdir().expect("home");
        std::fs::write(
            home.path().join(".gitconfig"),
            "[user]\n\tname = A Person\n\temail = a@example.invalid\n",
        )
        .expect("gitconfig");

        let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_codotheca-core"))
            .arg(format!("--data-dir={}", dir.path().display()))
            .arg("--epoch=7")
            .arg(format!("--parent-pid={}", std::process::id()))
            .env("HOME", home.path())
            .env("USERPROFILE", home.path())
            .env_remove("XDG_CONFIG_HOME")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("core spawns");

        let hello = recv(&mut child);
        assert_eq!(hello["t"], "hello", "{hello}");
        send(
            &mut child,
            &serde_json::json!({"t": "request", "id": 1, "command": "app.hello_ack", "args": {}}),
        );
        let _ack = recv(&mut child);

        send(
            &mut child,
            &serde_json::json!({"t": "request", "id": 2, "command": "identity.list", "args": {}}),
        );
        let reply = recv(&mut child);
        assert_eq!(reply["t"], "response", "{reply}");
        // §2.4's response frame carries its payload on `ok`.
        let rows = reply["ok"]
            .as_array()
            .unwrap_or_else(|| panic!("a list of identities: {reply}"));
        assert_eq!(
            rows.len(),
            1,
            "the configured address, and nothing invented beside it: {reply}"
        );
        assert_eq!(rows[0]["email"], "a@example.invalid", "{reply}");

        send(
            &mut child,
            &serde_json::json!({"t": "request", "id": 3, "command": "app.shutdown", "args": {}}),
        );
        let status = child.wait().expect("exits");
        assert!(status.success(), "{status:?}");
    }

    /// stdout carries protocol frames and nothing else. The startup sequence and the tick now sit
    /// behind the loop, and every one of their diagnostics must still go to stderr.
    #[test]
    fn nothing_but_frames_reaches_stdout() {
        let dir = tempfile::tempdir().expect("tmp");
        let mut child = spawn(dir.path());

        // Written without reading, so every byte the core produces is still in the pipe for the
        // scan below. Reading first would consume the frames this test exists to count.
        send(
            &mut child,
            &serde_json::json!({"t": "request", "id": 1, "command": "app.hello_ack", "args": {}}),
        );
        send(
            &mut child,
            &serde_json::json!({"t": "request", "id": 2, "command": "app.shutdown", "args": {}}),
        );

        let out = child.wait_with_output().expect("exits");
        assert!(out.status.success());

        // Every byte of stdout parses as a length-prefixed frame carrying JSON. A stray
        // `println!` anywhere in startup would desynchronise this immediately.
        let mut cursor = std::io::Cursor::new(out.stdout);
        let mut frames = 0_u32;
        let mut buf = Vec::new();
        while read_frame(&mut cursor, &mut buf).is_ok() {
            serde_json::from_slice::<serde_json::Value>(&buf).expect("every frame is JSON");
            frames += 1;
            buf.clear();
        }
        assert!(
            frames >= 3,
            "hello, ack and the shutdown response: {frames}"
        );
    }
}
