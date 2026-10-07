#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

//! Criterion 14's core half: a schema from the future refuses to open and touches nothing; a
//! corrupt database quarantines with its sidecar files and rebuilds into an honest report.
//!
//! The criterion, not the mechanism. `index_recovery.rs` and `sidecar_rebuild.rs` cover
//! classification, the rebuild and its report; this file asserts the two clauses §16.14 states —
//! **both** numbers in the error, the file byte-identical after the refusal, and the corrupt files
//! set aside by the binary the shell runs, never by a library call production does not make.
//!
//! The names carry the `ac_14_` tag, so the acceptance harness joins them to criterion 14's two
//! automated checks by the id `acceptance_recovery::<fn>`. `index_recovery.rs`'s tests carry no
//! tag and join to nothing, correctly: one criterion, one pair of tests.

use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

use codotheca_core::index::migrate::SUPPORTED_SCHEMA_VERSION;
use codotheca_core::index::rebuild::{RebuildReportFile, REBUILD_REPORT_FILE};
use codotheca_core::index::{open_connection, Index, IndexError};
use codotheca_core::proto::frame::{read_frame, write_frame};
use codotheca_core::surfaces::startup_failure::EXIT_INDEX_FATAL;

/// The whole file's bytes. The criterion says the file is left untouched, which is a statement
/// about content — an mtime comparison would pass over a rewrite that produced the same length.
fn fingerprint(path: &Path) -> Vec<u8> {
    fs::read(path).unwrap()
}

/// The real binary against `dir`, as the shell starts it — with `--rebuild` when REBUILD asked.
fn core(dir: &Path, rebuild: bool) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_codotheca-core"));
    cmd.arg(format!("--data-dir={}", dir.display()))
        .arg("--epoch=1")
        .arg(format!("--parent-pid={}", std::process::id()))
        .stdin(Stdio::null());
    if rebuild {
        cmd.arg("--rebuild");
    }
    cmd
}

#[test]
fn ac_14_schema_from_the_future_refuses_to_open() {
    let dir = tempfile::tempdir().unwrap();
    let db = Index::db_path(dir.path());

    let future = SUPPORTED_SCHEMA_VERSION + 1;
    {
        let conn = open_connection(&db).unwrap();
        conn.pragma_update(None, "user_version", future).unwrap();
    }
    let before = fingerprint(&db);

    let error = Index::open(dir.path()).expect_err("a schema from the future must refuse to open");
    match error {
        IndexError::SchemaFromFuture { on_disk, supported } => {
            // Both numbers, because "please update" without them is unactionable.
            assert_eq!(on_disk, future);
            assert_eq!(supported, SUPPORTED_SCHEMA_VERSION);
        }
        other => panic!("expected SchemaFromFuture, got {other:?}"),
    }

    assert_eq!(
        fingerprint(&db),
        before,
        "the refusal must not write to the file"
    );
    assert!(
        !Index::backup_dir(dir.path()).exists(),
        "there is no open-and-write path to offer, so there is nothing to back up"
    );

    // Refusing twice is the same refusal: nothing about the first attempt changed the file.
    assert!(matches!(
        Index::open(dir.path()),
        Err(IndexError::SchemaFromFuture { .. })
    ));
    assert_eq!(fingerprint(&db), before);
}

/// The database, its journal files and the sidecar, by name, as planted.
fn planted(dir: &Path) -> Vec<(&'static str, Vec<u8>)> {
    [
        "index.db",
        "index.db-wal",
        "index.db-shm",
        "index-sidecar.json",
    ]
    .into_iter()
    .map(|name| (name, fs::read(dir.join(name)).unwrap()))
    .collect()
}

/// Criterion 14's quarantine clause, through the production chain. The binary meets a corrupt
/// database, reports and exits having moved nothing; started again with `--rebuild`, it sets the
/// database, its `-wal` and `-shm` aside and copies the sidecar beside them as `*.corrupt-<t>`
/// siblings, and the rebuild report names each one.
#[test]
fn ac_14_the_production_chain_quarantines_the_three_files_with_a_sidecar_copy() {
    let dir = tempfile::tempdir().unwrap();
    {
        let index = Index::open_at(dir.path(), 1_700_000_000).unwrap();
        index.export_sidecar(1_700_000_000).unwrap();
    }
    let db = Index::db_path(dir.path());
    fs::write(&db, b"this is not a database").unwrap();
    fs::write(db.with_file_name("index.db-wal"), b"stale wal").unwrap();
    fs::write(db.with_file_name("index.db-shm"), b"stale shm").unwrap();
    let before = planted(dir.path());

    let detect = core(dir.path(), false).output().unwrap();
    assert_eq!(
        detect.status.code(),
        Some(i32::from(EXIT_INDEX_FATAL)),
        "{}",
        String::from_utf8_lossy(&detect.stderr)
    );
    for (name, bytes) in &before {
        assert_eq!(
            &fs::read(dir.path().join(name)).unwrap(),
            bytes,
            "{name} changed before any rebuild was asked for"
        );
    }

    let mut rebuild = core(dir.path(), true)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut buf = Vec::new();
    read_frame(rebuild.stdout.as_mut().unwrap(), &mut buf).unwrap();
    let hello: serde_json::Value = serde_json::from_slice(&buf).unwrap();
    assert_eq!(hello["t"], "hello", "{hello}");
    let ack = serde_json::json!({"t": "request", "id": 1, "command": "app.hello_ack", "args": {}});
    write_frame(
        rebuild.stdin.as_mut().unwrap(),
        &serde_json::to_vec(&ack).unwrap(),
    )
    .unwrap();
    buf.clear();
    read_frame(rebuild.stdout.as_mut().unwrap(), &mut buf).unwrap();
    // Closing stdin ends the loop: the core exits as the shell going away makes it.
    drop(rebuild.stdin.take());
    let out = rebuild.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "{:?}: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );

    let report: RebuildReportFile =
        serde_json::from_slice(&fs::read(dir.path().join(REBUILD_REPORT_FILE)).unwrap()).unwrap();
    eprintln!("quarantine set: {:?}", report.quarantine_files);
    assert_eq!(report.quarantine_files.len(), before.len());
    for (name, bytes) in &before {
        let moved = dir
            .path()
            .join(format!("{name}.corrupt-{}", report.quarantined_at));
        assert!(
            report
                .quarantine_files
                .contains(&moved.display().to_string()),
            "{name} is not in the report's quarantine set"
        );
        assert!(moved.exists(), "{name} was not set aside");
        // "The old one was preserved elsewhere", not "the old one is gone".
        assert_eq!(&fs::read(&moved).unwrap(), bytes, "{name}'s bytes moved");
    }
    assert!(
        dir.path().join("index-sidecar.json").exists(),
        "the sidecar is copied, never moved"
    );
    assert_eq!(
        Index::open(dir.path()).unwrap().schema_version().unwrap(),
        SUPPORTED_SCHEMA_VERSION
    );
}
