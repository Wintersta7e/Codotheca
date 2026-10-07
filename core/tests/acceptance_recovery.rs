#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

//! Criterion 14's core half: a schema from the future refuses to open and touches nothing; a
//! corrupt database quarantines with its sidecar files and rebuilds into an honest report.
//!
//! The criterion, not the mechanism. `index_recovery.rs` covers classification, quarantine and
//! the sidecar restore; this file asserts the two clauses §16.14 states — **both** numbers in
//! the error, the file byte-identical after the refusal, and a report that names exactly what
//! was not derivable rather than a zero that reads as "nothing was lost".
//!
//! The names carry the `ac_14_` tag, so the acceptance harness joins them to criterion 14's two
//! automated checks by the id `acceptance_recovery::<fn>`. `index_recovery.rs`'s tests carry no
//! tag and join to nothing, correctly: one criterion, one pair of tests.

use std::fs;
use std::path::Path;

use codotheca_core::index::migrate::SUPPORTED_SCHEMA_VERSION;
use codotheca_core::index::rebuild::{
    rebuild_in_place, RebuildError, RebuildOutcome, RebuildReportFile,
};
use codotheca_core::index::{open_connection, Index, IndexError};

/// The whole file's bytes. The criterion says the file is left untouched, which is a statement
/// about content — an mtime comparison would pass over a rewrite that produced the same length.
fn fingerprint(path: &Path) -> Vec<u8> {
    fs::read(path).unwrap()
}

fn rebuilt(outcome: Result<RebuildOutcome, RebuildError>) -> RebuildReportFile {
    match outcome {
        Ok(RebuildOutcome::Rebuilt(report)) => report,
        other => panic!("expected a rebuild, got {other:?}"),
    }
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

#[test]
fn ac_14_corrupt_database_quarantines_and_rebuilds() {
    // Scenario 1: the path a running core actually takes — open, classify, rebuild.
    let dir = tempfile::tempdir().unwrap();
    let db = Index::db_path(dir.path());
    let wal = db.with_file_name("index.db-wal");
    let shm = db.with_file_name("index.db-shm");
    fs::write(&db, b"this is not a database").unwrap();
    fs::write(&wal, b"stale wal").unwrap();
    fs::write(&shm, b"stale shm").unwrap();

    match Index::open(dir.path()) {
        Err(IndexError::Corrupt { .. }) => {}
        Err(other) => panic!("expected Corrupt, got {other:?}"),
        Ok(_) => panic!("a corrupt file must not open"),
    }

    // Measured, not assumed: SQLite unlinks both sidecar files itself when it decides the main
    // file is not a database, so by the time the recovery path runs there is nothing left to
    // move. The clause about the wal and the shm travelling with the database is therefore
    // asserted below, against `rebuild` on a tree that still has them — not here, where a
    // passing assertion would be describing SQLite's cleanup rather than ours.
    assert!(
        !wal.exists() && !shm.exists(),
        "a failed open leaves the sidecars behind; the quarantine assertion below is then \
         asserting the wrong subject and this test must be re-read"
    );

    let report = rebuilt(rebuild_in_place(dir.path(), 1_700_000_000));
    let moved_db = db.with_file_name("index.db.corrupt-1700000000");
    assert!(moved_db.exists());
    assert!(report
        .quarantine_files
        .contains(&moved_db.display().to_string()));
    assert_eq!(report.quarantined_at, 1_700_000_000);

    // With no sidecar there was nothing to restore, and the report says so rather than
    // reporting a zero that could be read as "nothing was lost".
    assert!(report.restored.is_empty());
    assert_eq!(report.pending, 0);
    assert_eq!(
        report.gap_started_at, None,
        "no sidecar means no gap start is knowable, which is not the same as a gap of zero"
    );

    // The rebuilt database is usable and is at the version this build speaks.
    let index = Index::open(dir.path()).unwrap();
    assert_eq!(index.schema_version().unwrap(), SUPPORTED_SCHEMA_VERSION);

    // Scenario 2: the wal and the shm travel with the database. A rebuild that moved only the
    // main file would open onto a stale journal, so this is asserted where the files still
    // exist — a caller holding a Corrupt error from somewhere other than a fresh open.
    let kept = tempfile::tempdir().unwrap();
    let kept_db = Index::db_path(kept.path());
    fs::write(&kept_db, b"this is not a database").unwrap();
    fs::write(kept_db.with_file_name("index.db-wal"), b"stale wal").unwrap();
    fs::write(kept_db.with_file_name("index.db-shm"), b"stale shm").unwrap();

    let moved = rebuilt(rebuild_in_place(kept.path(), 1_700_000_001));
    let set_aside = |name: &str| kept.path().join(format!("{name}.corrupt-1700000001"));
    for name in ["index.db", "index.db-wal", "index.db-shm"] {
        assert!(
            moved
                .quarantine_files
                .contains(&set_aside(name).display().to_string()),
            "{name} is not in the quarantine set"
        );
    }
    // The quarantined files carry the *stale* bytes: "the old one is gone" is not the assertion
    // to make; "the old one was preserved elsewhere" is.
    assert_eq!(fs::read(set_aside("index.db-wal")).unwrap(), b"stale wal");
    assert_eq!(fs::read(set_aside("index.db-shm")).unwrap(), b"stale shm");
    assert_eq!(
        fs::read(set_aside("index.db")).unwrap(),
        b"this is not a database"
    );
}
