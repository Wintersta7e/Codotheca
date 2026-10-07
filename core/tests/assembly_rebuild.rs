#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! §48.7.1's chain through the real binary. Started on a corrupt index the core writes its report
//! and nothing else; started with `--rebuild` it rebuilds beside the corrupt files, opens the
//! result as an ordinary start and answers — or, when the rebuild fails, says why in the same
//! report and leaves the directory as it found it, so REBUILD can be asked for again.

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::Path;
use std::process::{Child, Command, Output, Stdio};

use codotheca_core::index::path::{native_platform, PathPlatform, StoredPath};
use codotheca_core::index::rebuild::REBUILD_REPORT_FILE;
use codotheca_core::index::Index;
use codotheca_core::lifecycle::{CORE_LOCK_FILE, CORE_OWNER_FILE};
use codotheca_core::proto::frame::{read_frame, write_frame};
use codotheca_core::surfaces::startup_failure::{EXIT_INDEX_FATAL, STARTUP_FAILURE_FILE};
use serde_json::{json, Value};

const NOW: i64 = 1_760_000_000;

fn command(dir: &Path, rebuild: bool) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_codotheca-core"));
    cmd.arg(format!("--data-dir={}", dir.display()))
        .arg("--epoch=7")
        .arg(format!("--parent-pid={}", std::process::id()));
    if rebuild {
        cmd.arg("--rebuild");
    }
    cmd
}

/// One run that is expected to end on its own: stdin is closed, so a start that reaches the loop
/// ends there too instead of hanging the test.
fn run_to_exit(dir: &Path, rebuild: bool) -> Output {
    command(dir, rebuild)
        .stdin(Stdio::null())
        .output()
        .expect("core runs")
}

fn spawn(dir: &Path, rebuild: bool) -> Child {
    command(dir, rebuild)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("core spawns")
}

fn send(child: &mut Child, frame: &Value) {
    let body = serde_json::to_vec(frame).expect("frame");
    let stdin = child.stdin.as_mut().expect("stdin");
    write_frame(stdin, &body).expect("write");
    stdin.flush().expect("flush");
}

fn recv(child: &mut Child) -> Value {
    let stdout = child.stdout.as_mut().expect("stdout");
    let mut buf = Vec::new();
    read_frame(stdout, &mut buf).expect("read");
    serde_json::from_slice(&buf).expect("json")
}

/// The answer to request `id`, which takes no arguments, past any event frame.
fn request(child: &mut Child, id: u64, command: &str) -> Value {
    send(
        child,
        &json!({"t": "request", "id": id, "command": command, "args": {}}),
    );
    loop {
        let frame = recv(child);
        if frame["t"] != "event" && frame["id"] == id {
            return frame;
        }
    }
}

/// Handshake, then `app.hello_ack`; the core is in its loop when this returns.
fn handshake(child: &mut Child) {
    let hello = recv(child);
    assert_eq!(hello["t"], "hello", "{hello}");
    let ack = request(child, 1, "app.hello_ack");
    assert_eq!(ack["t"], "response", "{ack}");
}

fn shut_down(mut child: Child) {
    let bye = request(&mut child, 99, "app.shutdown");
    assert_eq!(bye["t"], "response", "{bye}");
    let out = child.wait_with_output().expect("exits");
    assert!(
        out.status.success(),
        "{:?}: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
}

fn sibling(path: &Path, suffix: &str) -> std::path::PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(suffix);
    name.into()
}

/// The database and its journal files overwritten with bytes SQLite reads as no database.
fn corrupt(dir: &Path) {
    let db = Index::db_path(dir);
    std::fs::write(&db, b"this is not a database").unwrap();
    std::fs::write(sibling(&db, "-wal"), b"a stale journal").unwrap();
    std::fs::write(sibling(&db, "-shm"), b"a stale wal-index").unwrap();
}

/// An index holding one manual collection and `roots`, its sidecar exported, then corrupted.
fn plant(dir: &Path, roots: &[&Path]) {
    let index = Index::open_at(dir, NOW).unwrap();
    let conn = index.conn();
    let kind = match native_platform() {
        PathPlatform::Windows => "win",
        PathPlatform::Unix => "linux",
    };
    for root in roots {
        let stored = StoredPath::from_os(root.as_os_str(), native_platform());
        let (bytes, key, display) = stored.as_params();
        conn.execute(
            "INSERT INTO scan_root (kind, path_bytes, path_key, path_display, added_by, added_at)
             VALUES (?1, ?2, ?3, ?4, 'user', 1)",
            rusqlite::params![kind, bytes, key, display],
        )
        .unwrap();
    }
    conn.execute(
        "INSERT INTO collection (name, kind) VALUES ('kept', 'manual')",
        [],
    )
    .unwrap();
    index.export_sidecar(NOW).unwrap();
    drop(index);
    corrupt(dir);
}

/// Every entry in `dir` by name, with its bytes — minus the report under test and what any start
/// writes (the lock and its owner file). A directory compares by presence only.
fn fingerprint(dir: &Path) -> BTreeMap<String, Option<Vec<u8>>> {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap())
        .filter(|entry| {
            let name = entry.file_name();
            ![
                STARTUP_FAILURE_FILE,
                CORE_LOCK_FILE,
                CORE_OWNER_FILE,
                "logs",
            ]
            .iter()
            .any(|skip| name == *skip)
        })
        .map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let bytes = entry
                .path()
                .is_file()
                .then(|| std::fs::read(entry.path()).unwrap());
            (name, bytes)
        })
        .collect()
}

/// Each name that appeared, vanished or changed between two fingerprints.
fn changed(
    before: &BTreeMap<String, Option<Vec<u8>>>,
    after: &BTreeMap<String, Option<Vec<u8>>>,
) -> Vec<String> {
    let mut names: Vec<&String> = before.keys().chain(after.keys()).collect();
    names.sort();
    names.dedup();
    names
        .into_iter()
        .filter(|name| before.get(*name) != after.get(*name))
        .map(
            |name| match (before.contains_key(name), after.contains_key(name)) {
                (true, false) => format!("{name} vanished"),
                (false, true) => format!("{name} appeared"),
                _ => format!("{name} changed"),
            },
        )
        .collect()
}

fn report(dir: &Path) -> Value {
    let bytes = std::fs::read(dir.join(STARTUP_FAILURE_FILE)).expect("a report");
    serde_json::from_slice(&bytes).expect("json")
}

fn set_aside(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.contains(".corrupt-"))
        .collect()
}

fn fatal(out: &Output) {
    assert_eq!(
        out.status.code(),
        Some(i32::from(EXIT_INDEX_FATAL)),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// §48.7.1 step 1: the core meets a corrupt index, writes its report and exits, and every other
/// byte of the data directory — the database, its journal, the sidecar — is as it was.
#[test]
fn detect_writes_only_the_report_and_moves_nothing() {
    let dir = tempfile::tempdir().unwrap();
    plant(dir.path(), &[]);
    let before = fingerprint(dir.path());
    eprintln!("before: {:?}", before.keys().collect::<Vec<_>>());
    for name in [
        "index.db",
        "index.db-wal",
        "index.db-shm",
        "index-sidecar.json",
    ] {
        assert!(before.contains_key(name), "the fixture has no {name}");
    }

    let out = run_to_exit(dir.path(), false);
    fatal(&out);

    let after = fingerprint(dir.path());
    eprintln!("entries compared: {}", before.len());
    assert_eq!(
        changed(&before, &after),
        Vec::<String>::new(),
        "the detect path wrote into the data directory"
    );
    assert_eq!(report(dir.path())["kind"], "corrupt_index");
}

/// The report states the sidecar as the rebuild would find it — in each of its four states — and
/// claims no quarantine, because none happened.
#[test]
fn the_report_carries_the_sidecar_state_and_no_quarantine() {
    let mut states = 0_u32;
    for state in ["present", "absent", "unreadable", "newer"] {
        let dir = tempfile::tempdir().unwrap();
        let sidecar = Index::sidecar_path(dir.path());
        match state {
            "present" => plant(dir.path(), &[]),
            "absent" => corrupt(dir.path()),
            "unreadable" => {
                corrupt(dir.path());
                std::fs::write(&sidecar, b"{ not a sidecar").unwrap();
            }
            _ => {
                corrupt(dir.path());
                std::fs::write(&sidecar, br#"{"format": 99}"#).unwrap();
            }
        }

        fatal(&run_to_exit(dir.path(), false));
        let doc = report(dir.path());
        eprintln!("{state}: {doc}");
        let mut keys: Vec<&String> = doc.as_object().unwrap().keys().collect();
        keys.sort();
        assert_eq!(
            keys,
            ["gapCountsRecoverable", "kind", "rebuildFailed", "sidecar"],
            "{state}: a key beyond these claims something that did not happen"
        );
        assert_eq!(doc["kind"], "corrupt_index");
        assert_eq!(doc["rebuildFailed"], Value::Null);
        assert_eq!(doc["gapCountsRecoverable"], false);
        let side = &doc["sidecar"];
        assert_eq!(side["state"], state);
        if state == "present" {
            assert_eq!(side["writtenAt"], NOW);
            assert!(side["generation"].as_u64().is_some_and(|g| g > 0), "{side}");
            assert_eq!(side["counts"]["collections"], 1, "{side}");
            assert_eq!(side["reason"], Value::Null);
        } else {
            assert_eq!(side["writtenAt"], Value::Null);
            assert_eq!(side["generation"], Value::Null);
            assert_eq!(side["counts"], Value::Null);
        }
        match state {
            "unreadable" | "newer" => {
                assert_ne!(side["reason"].as_str().unwrap_or(""), "", "{side}");
            }
            _ => assert_eq!(side["reason"], Value::Null),
        }
        assert_eq!(set_aside(dir.path()), Vec::<String>::new(), "{state}");
        states += 1;
    }
    eprintln!("sidecar states compared: {states}");
    assert_eq!(states, 4);
}

/// `--rebuild` rebuilds, opens the result as an ordinary start — clearing the report the detect
/// path left — and answers from the restored index. With no root restored, no scan starts.
#[test]
fn rebuild_mode_rebuilds_and_the_core_then_answers_a_command() {
    let dir = tempfile::tempdir().unwrap();
    plant(dir.path(), &[]);
    fatal(&run_to_exit(dir.path(), false));
    assert!(dir.path().join(STARTUP_FAILURE_FILE).exists());

    let mut child = spawn(dir.path(), true);
    handshake(&mut child);
    let list = request(&mut child, 2, "collections.list");
    eprintln!("collections.list: {list}");
    let names: Vec<&str> = list["ok"]
        .as_array()
        .unwrap_or_else(|| panic!("a list: {list}"))
        .iter()
        .filter_map(|c| c["name"].as_str())
        .collect();
    assert_eq!(names, ["kept"], "the restored collection");
    let status = request(&mut child, 3, "scan.status");
    eprintln!("scan.status: {status}");
    assert_eq!(status["ok"]["runId"], Value::Null, "no root, so no scan");
    shut_down(child);

    assert!(
        !dir.path().join(STARTUP_FAILURE_FILE).exists(),
        "the open after the rebuild clears the report"
    );
    let rebuilt: Value =
        serde_json::from_slice(&std::fs::read(dir.path().join(REBUILD_REPORT_FILE)).unwrap())
            .unwrap();
    eprintln!("rebuild report: {rebuilt}");
    assert_eq!(rebuilt["restored"]["collections"], 1);
    let mut moved = set_aside(dir.path());
    moved.sort();
    eprintln!("set aside: {moved:?}");
    assert_eq!(moved.len(), 4, "the triplet and the sidecar copy");
}

/// A rebuild that fails says so in the same report, `rebuildFailed` naming why, and leaves the
/// directory as it found it; asked again once the cause is gone, it succeeds.
#[test]
fn a_failed_rebuild_reports_itself_and_a_retry_succeeds() {
    let dir = tempfile::tempdir().unwrap();
    plant(dir.path(), &[]);
    // A directory where the rebuild writes its report's temporary file: the last act fails, after
    // every other act is done, so all of them must be taken back.
    let blocker = dir.path().join(format!("{REBUILD_REPORT_FILE}.tmp"));
    std::fs::create_dir(&blocker).unwrap();
    let before = fingerprint(dir.path());

    fatal(&run_to_exit(dir.path(), true));
    let doc = report(dir.path());
    eprintln!("failed rebuild: {doc}");
    assert_eq!(doc["kind"], "corrupt_index");
    assert_ne!(doc["rebuildFailed"].as_str().unwrap_or(""), "", "{doc}");
    assert_eq!(doc["sidecar"]["state"], "present");
    assert_eq!(
        changed(&before, &fingerprint(dir.path())),
        Vec::<String>::new(),
        "a failed rebuild left the directory changed"
    );
    assert_eq!(set_aside(dir.path()), Vec::<String>::new());

    std::fs::remove_dir(&blocker).unwrap();
    let mut child = spawn(dir.path(), true);
    handshake(&mut child);
    let list = request(&mut child, 2, "collections.list");
    assert_eq!(list["ok"][0]["name"], "kept", "{list}");
    shut_down(child);
    assert!(!dir.path().join(STARTUP_FAILURE_FILE).exists());
    assert!(dir.path().join(REBUILD_REPORT_FILE).exists());
}

/// §48.7.1 4(d): a rebuild that restored a scan root starts the full scan `scan.start` would, before
/// the shell asks for anything — the projects only a scan finds, and the records waiting for them,
/// come back on their own.
#[test]
fn a_rebuild_that_restored_roots_starts_a_scan() {
    let dir = tempfile::tempdir().unwrap();
    let root = tempfile::tempdir().unwrap();
    plant(dir.path(), &[root.path()]);

    let mut child = spawn(dir.path(), true);
    handshake(&mut child);
    // The first request after the handshake, and not `scan.start`: a run it names was started by
    // the rebuild.
    let status = request(&mut child, 2, "scan.status");
    eprintln!("scan.status: {status}");
    assert_ne!(status["ok"]["runId"], Value::Null, "{status}");
    assert_eq!(status["ok"]["mode"], "full", "{status}");
    shut_down(child);

    let rebuilt: Value =
        serde_json::from_slice(&std::fs::read(dir.path().join(REBUILD_REPORT_FILE)).unwrap())
            .unwrap();
    assert_eq!(rebuilt["restored"]["roots"], 1, "{rebuilt}");
}
