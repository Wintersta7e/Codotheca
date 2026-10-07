//! §48.7.1 step 4: the rebuild probes without touching, builds beside, restores in one
//! transaction, and only then sets the corrupt files aside and swaps — or changes nothing.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use codotheca_core::index::migrate::{apply_all, MIGRATIONS};
use codotheca_core::index::pending::PendingRecord;
use codotheca_core::index::rebuild::{
    probe_open, probe_open_in, rebuild_in_place, rebuild_in_place_with, Probe, RebuildError,
    RebuildOutcome, RebuildReportFile, RebuildStep, REBUILD_REPORT_FILE,
};
use codotheca_core::index::sidecar::{export, write_atomically, Sidecar};
use codotheca_core::index::{open_connection, Index, IndexError};

const NOW: i64 = 5_000;

/// Every file in `dir` by name, with its bytes. A file created, removed, renamed or rewritten
/// changes it.
fn fingerprint(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| {
            let entry = entry.unwrap();
            let name = entry.file_name().to_string_lossy().into_owned();
            (name, std::fs::read(entry.path()).unwrap())
        })
        .collect()
}

fn sibling(db: &Path, suffix: &str) -> PathBuf {
    let mut p = db.as_os_str().to_os_string();
    p.push(suffix);
    PathBuf::from(p)
}

/// A library with one project (a copy, a session of one segment, a note), a root, an identity,
/// a manual collection holding the project, a setting, a view-state row and a pending row a
/// previous rebuild left; exported at generation 7 and written at 900.
fn seed(dir: &Path) -> Sidecar {
    let mut conn = open_connection(&Index::db_path(dir)).unwrap();
    apply_all(&mut conn, MIGRATIONS).unwrap();
    conn.execute_batch(
        "INSERT INTO project (name, seed_basename, lineage_key, remote_key, notes,
                              created_at, updated_at)
         VALUES ('thing', 'thing-dir', 'lineage-1', 'h/o/n', 'a note', 1, 1);
         INSERT INTO location (project_id, kind, path_bytes, path_key, path_display, store_key,
                               presence, repo_kind)
         VALUES (1, 'linux', x'2f722f61', x'2f722f61', '/r/a', 's', 'present', 'worktree');
         INSERT INTO session (project_id, location_id, started_at, ended_at, credited_seconds,
                              close_reason)
         VALUES (1, 1, 100, 200, 90, 'stop');
         INSERT INTO session_segment (session_id, started_at, ended_at, credited_seconds, closed_by)
         VALUES (1, 100, 190, 90, 'session_end');
         INSERT INTO scan_root (kind, path_bytes, path_key, path_display, added_by, added_at)
         VALUES ('linux', x'2f72', x'2f72', '/r', 'user', 5);
         INSERT INTO identity (email, source, confirmed_at)
         VALUES ('a@example.invalid', 'gitconfig', 42);
         INSERT INTO collection (name, kind) VALUES ('By hand', 'manual');
         INSERT INTO collection_member (collection_id, project_id) VALUES (1, 1);
         INSERT INTO app_meta (k, v) VALUES ('effects_tier', 'reduced');
         INSERT INTO view_state (k, v) VALUES ('shelf.sort', 'touched');
         INSERT INTO sidecar_pending (source_generation, subject_key, location_keys, record,
                                      queued_at)
         VALUES (3, 'path:linux::2f6f6c64', '[]', '{\"left\":\"by an earlier rebuild\"}', 50);",
    )
    .unwrap();
    let doc = export(&conn, 7, 900).unwrap();
    write_atomically(&doc, &Index::sidecar_path(dir)).unwrap();
    doc
}

/// Non-database bytes where the index was, with a stale journal and wal-index beside it.
fn corrupt(dir: &Path) {
    let db = Index::db_path(dir);
    std::fs::write(&db, b"this is not a database").unwrap();
    std::fs::write(sibling(&db, "-wal"), b"stale wal").unwrap();
    std::fs::write(sibling(&db, "-shm"), b"stale shm").unwrap();
}

fn rebuilt(outcome: Result<RebuildOutcome, RebuildError>) -> RebuildReportFile {
    match outcome {
        Ok(RebuildOutcome::Rebuilt(report)) => report,
        other => panic!("expected a rebuild, got {other:?}"),
    }
}

/// A healthy WAL database whose last commit is still only in its `-wal`, copied with its
/// siblings while the writer holds it open.
fn healthy_with_a_wal_commit(dir: &Path) {
    let source = tempfile::tempdir().unwrap();
    let db = Index::db_path(source.path());
    let conn = rusqlite::Connection::open(&db).unwrap();
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA wal_autocheckpoint=0;
         CREATE TABLE kept (v TEXT);
         PRAGMA wal_checkpoint(TRUNCATE);
         INSERT INTO kept (v) VALUES ('only in the wal');",
    )
    .unwrap();
    for suffix in ["", "-wal", "-shm"] {
        std::fs::copy(sibling(&db, suffix), sibling(&Index::db_path(dir), suffix)).unwrap();
    }
    drop(conn);
}

/// A real index, checkpointed into one file, with the b-tree page `sqlite_master` lives on
/// overwritten past the file header.
fn real_database_with_a_page_overwritten(dir: &Path) {
    let db = Index::db_path(dir);
    {
        let mut conn = open_connection(&db).unwrap();
        apply_all(&mut conn, MIGRATIONS).unwrap();
    }
    let mut bytes = std::fs::read(&db).unwrap();
    let page_size = usize::from(u16::from_be_bytes([bytes[16], bytes[17]]));
    for byte in &mut bytes[100..page_size] {
        *byte = 0xA5;
    }
    std::fs::write(&db, bytes).unwrap();
    for suffix in ["-wal", "-shm"] {
        assert!(
            !sibling(&db, suffix).exists(),
            "the fixture is one file by design"
        );
    }
}

/// What a crash during a checkpoint leaves: page 1 of the main file torn, the journal still
/// holding a good page 1 and the last commit. Copied with its siblings while the writer holds it
/// open, so the journal is not folded back on close.
fn torn_page_one_with_a_good_copy_in_the_journal(dir: &Path) {
    let source = tempfile::tempdir().unwrap();
    let db = Index::db_path(source.path());
    let conn = rusqlite::Connection::open(&db).unwrap();
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA wal_autocheckpoint=0;
         CREATE TABLE filler (v INTEGER);
         WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 50)
         INSERT INTO filler (v) SELECT i FROM n;
         PRAGMA wal_checkpoint(TRUNCATE);
         CREATE TABLE kept (v TEXT);
         INSERT INTO kept (v) VALUES ('only in the wal');",
    )
    .unwrap();
    for suffix in ["", "-wal", "-shm"] {
        std::fs::copy(sibling(&db, suffix), sibling(&Index::db_path(dir), suffix)).unwrap();
    }
    drop(conn);
    let copy = Index::db_path(dir);
    let mut bytes = std::fs::read(&copy).unwrap();
    for byte in &mut bytes[100..500] {
        *byte = 0xA5;
    }
    std::fs::write(&copy, bytes).unwrap();
}

/// A planted directory: what it is, how to plant it, and whether the probe must find it corrupt.
type Shape = (&'static str, fn(&Path), bool);

/// §48.7.1's evidence: an ordinary open of a file SQLite finds is not a database unlinks its
/// `-wal` and `-shm`. The probe must not — measured over four shapes, each classified, each
/// directory byte-identical afterwards and the probe's own copy gone; and each that opens still
/// recovers its commit. §48.7.1 4(a): a page 1 torn mid-checkpoint with a good copy in the
/// journal is one the ordinary open recovers, so it must probe as `Opens`.
#[test]
fn probe_leaves_all_three_files_byte_identical() {
    let mut measured = 0_u32;
    let shapes: [Shape; 4] = [
        (
            "a torn page 1 with a good copy in the journal",
            torn_page_one_with_a_good_copy_in_the_journal,
            false,
        ),
        (
            "non-database bytes with a stale -wal and -shm",
            corrupt,
            true,
        ),
        (
            "a real database with its schema page overwritten",
            real_database_with_a_page_overwritten,
            true,
        ),
        (
            "a healthy database with an uncheckpointed commit",
            healthy_with_a_wal_commit,
            false,
        ),
    ];
    for (case, plant, expect_corrupt) in shapes {
        let dir = tempfile::tempdir().unwrap();
        let scratch = tempfile::tempdir().unwrap();
        plant(dir.path());
        let before = fingerprint(dir.path());
        let probe = probe_open_in(&Index::db_path(dir.path()), scratch.path());
        eprintln!("{case}: {probe:?}, files {:?}", before.keys());
        match (&probe, expect_corrupt) {
            (Probe::Corrupt { detail }, true) => assert_ne!(detail, ""),
            (Probe::Opens, false) => {}
            _ => panic!("{case} probed as {probe:?}"),
        }
        assert_eq!(
            fingerprint(dir.path()),
            before,
            "{case}: the probe changed the directory"
        );
        assert_eq!(
            std::fs::read_dir(scratch.path()).unwrap().count(),
            0,
            "{case}: the probe left its copy behind"
        );
        measured += 1;

        if !expect_corrupt {
            let conn = open_connection(&Index::db_path(dir.path())).unwrap();
            let kept: String = conn
                .query_row("SELECT v FROM kept", [], |r| r.get(0))
                .unwrap();
            assert_eq!(
                kept, "only in the wal",
                "normal WAL recovery stays SQLite's"
            );
        }
    }
    eprintln!("probe shapes measured: {measured}");
    assert_eq!(measured, 4);

    let empty = tempfile::tempdir().unwrap();
    assert!(matches!(
        probe_open(&Index::db_path(empty.path())),
        Probe::Missing
    ));
    assert!(fingerprint(empty.path()).is_empty());
}

/// The second read copies the files only when there is a journal to read through. A scratch
/// directory nothing can be made in tells the two apart: a copy attempted there answers `Other`,
/// the safe side, so a `Corrupt` answer is a read that made no copy.
#[test]
fn a_corrupt_file_with_no_journal_to_read_makes_no_copy() {
    let root = tempfile::tempdir().unwrap();
    let unusable = root.path().join("never-created");

    let journals: [(&str, Option<&[u8]>); 2] = [("no -wal", None), ("an empty -wal", Some(b""))];
    for (case, journal) in journals {
        let dir = tempfile::tempdir().unwrap();
        let db = Index::db_path(dir.path());
        std::fs::write(&db, b"this is not a database").unwrap();
        if let Some(bytes) = journal {
            std::fs::write(sibling(&db, "-wal"), bytes).unwrap();
        }
        let before = fingerprint(dir.path());
        let probe = probe_open_in(&db, &unusable);
        eprintln!("{case}: {probe:?}");
        assert!(
            matches!(probe, Probe::Corrupt { .. }),
            "{case}: probed as {probe:?}"
        );
        assert_eq!(
            fingerprint(dir.path()),
            before,
            "{case}: the probe changed it"
        );
    }

    // The same scratch with a journal to read through: the copy is attempted, cannot be made,
    // and the answer is never `Corrupt`.
    let dir = tempfile::tempdir().unwrap();
    corrupt(dir.path());
    let before = fingerprint(dir.path());
    let probe = probe_open_in(&Index::db_path(dir.path()), &unusable);
    eprintln!("a stale -wal, no scratch: {probe:?}");
    assert!(matches!(probe, Probe::Other(_)), "probed as {probe:?}");
    assert_eq!(fingerprint(dir.path()), before);
    assert!(!unusable.exists());
}

/// A database whose header says WAL but which has no `-wal` beside it, as a clean close leaves
/// one. A read that is not immutable makes a `-wal` and a `-shm` to read it; the probe must
/// answer with neither appearing.
#[test]
fn a_wal_database_with_no_journal_probes_without_making_one() {
    let dir = tempfile::tempdir().unwrap();
    let scratch = tempfile::tempdir().unwrap();
    let db = Index::db_path(dir.path());
    {
        let conn = rusqlite::Connection::open(&db).unwrap();
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             CREATE TABLE kept (v TEXT);
             INSERT INTO kept (v) VALUES ('folded in on close');",
        )
        .unwrap();
    }
    assert_eq!(
        std::fs::read(&db).unwrap()[18..20],
        [2, 2],
        "the fixture's header says WAL"
    );
    let before = fingerprint(dir.path());
    assert_eq!(
        before.keys().collect::<Vec<_>>(),
        ["index.db"],
        "the fixture is one file"
    );

    let probe = probe_open_in(&db, scratch.path());
    eprintln!(
        "a WAL header, no -wal: {probe:?}, files {:?}",
        fingerprint(dir.path()).keys()
    );
    assert!(matches!(probe, Probe::Opens), "probed as {probe:?}");
    assert_eq!(
        fingerprint(dir.path()),
        before,
        "the probe made a journal beside the database"
    );
    assert_eq!(std::fs::read_dir(scratch.path()).unwrap().count(), 0);
}

/// AC-P4-48-14: a rebuild made to fail after any of its acts leaves `index.db`, its siblings and
/// the sidecar byte-identical, with nothing beside them, and REBUILD may be retried.
#[test]
fn ac_p4_48_14_a_failed_rebuild_changes_nothing() {
    // The steps come from a real run, so a step added later is exercised without listing it.
    let probe_dir = tempfile::tempdir().unwrap();
    seed(probe_dir.path());
    corrupt(probe_dir.path());
    let seen = RefCell::new(Vec::new());
    rebuilt(rebuild_in_place_with(
        probe_dir.path(),
        NOW,
        MIGRATIONS,
        &|step| {
            seen.borrow_mut().push(step);
            Ok(())
        },
    ));
    let steps = seen.into_inner();
    assert_eq!(
        steps,
        [
            RebuildStep::Built,
            RebuildStep::Restored,
            RebuildStep::Checkpointed,
            RebuildStep::QuarantinedDb,
            RebuildStep::QuarantinedSiblings,
            RebuildStep::CopiedSidecar,
            RebuildStep::Swapped,
        ],
        "nothing is set aside until the restore has committed"
    );

    let dir = tempfile::tempdir().unwrap();
    seed(dir.path());
    corrupt(dir.path());
    let before = fingerprint(dir.path());
    let mut exercised = 0_u32;
    for &failing in &steps {
        let outcome = rebuild_in_place_with(dir.path(), NOW, MIGRATIONS, &|step| {
            if step == failing {
                Err(IndexError::Sidecar(format!(
                    "a failure planted after {step:?}"
                )))
            } else {
                Ok(())
            }
        });
        match outcome {
            Err(RebuildError::Failed { reason }) => {
                assert!(reason.contains("planted"), "{failing:?}: {reason}");
            }
            other => panic!("a failure after {failing:?} answered {other:?}"),
        }
        assert_eq!(
            fingerprint(dir.path()),
            before,
            "a failure after {failing:?} changed the data directory"
        );
        exercised += 1;
    }
    eprintln!("rebuild steps failed after and undone: {exercised}");
    assert!(exercised > 0, "no step was exercised");

    let report = rebuilt(rebuild_in_place(dir.path(), NOW));
    assert_eq!(report.quarantined_at, NOW);
    let written: RebuildReportFile =
        serde_json::from_slice(&std::fs::read(dir.path().join(REBUILD_REPORT_FILE)).unwrap())
            .unwrap();
    assert_eq!(written, report, "the report on disk is the one returned");
}

/// A crash leaves no undo behind it. Until the restore has committed, the corrupt files and the
/// sidecar must still be where the user left them, untouched.
#[test]
fn a_crash_before_the_swap_leaves_the_corrupt_files_in_place() {
    let mut crashed = 0_u32;
    for failing in [
        RebuildStep::Built,
        RebuildStep::Restored,
        RebuildStep::Checkpointed,
    ] {
        let dir = tempfile::tempdir().unwrap();
        seed(dir.path());
        corrupt(dir.path());
        let before = fingerprint(dir.path());
        let crash = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            rebuild_in_place_with(dir.path(), NOW, MIGRATIONS, &|step| {
                assert!(step != failing, "a crash planted after {step:?}");
                Ok(())
            })
        }));
        assert!(
            crash.is_err(),
            "the planted crash after {failing:?} did not happen"
        );
        let after = fingerprint(dir.path());
        for (name, bytes) in &before {
            assert_eq!(
                after.get(name),
                Some(bytes),
                "a crash after {failing:?} moved or changed {name}"
            );
        }
        crashed += 1;
    }
    eprintln!("crashes before the swap: {crashed}");
    assert_eq!(crashed, 3);
}

#[test]
fn a_database_that_opens_is_never_quarantined() {
    let dir = tempfile::tempdir().unwrap();
    seed(dir.path());
    let before = fingerprint(dir.path());
    assert!(matches!(
        rebuild_in_place(dir.path(), NOW),
        Ok(RebuildOutcome::Opened)
    ));
    assert_eq!(fingerprint(dir.path()), before);

    let empty = tempfile::tempdir().unwrap();
    assert!(matches!(
        rebuild_in_place(empty.path(), NOW),
        Ok(RebuildOutcome::Opened)
    ));
    assert!(
        fingerprint(empty.path()).is_empty(),
        "no database means the ordinary open creates one, not the rebuild"
    );
}

#[test]
fn a_newer_sidecar_refuses_and_moves_nothing() {
    let dir = tempfile::tempdir().unwrap();
    seed(dir.path());
    corrupt(dir.path());
    let path = Index::sidecar_path(dir.path());
    let mut doc: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    doc["format"] = serde_json::json!(3);
    std::fs::write(&path, serde_json::to_vec(&doc).unwrap()).unwrap();
    let before = fingerprint(dir.path());

    match rebuild_in_place(dir.path(), NOW) {
        Err(RebuildError::SidecarNewer { reason }) => eprintln!("refused: {reason}"),
        other => panic!("a newer sidecar answered {other:?}"),
    }
    assert_eq!(fingerprint(dir.path()), before);
}

#[test]
fn an_unreadable_sidecar_restores_nothing_and_is_kept_in_the_quarantine_set() {
    let dir = tempfile::tempdir().unwrap();
    seed(dir.path());
    corrupt(dir.path());
    let path = Index::sidecar_path(dir.path());
    let tampered = std::fs::read_to_string(&path)
        .unwrap()
        .replace("a note", "a NOTE");
    std::fs::write(&path, &tampered).unwrap();

    let report = rebuilt(rebuild_in_place(dir.path(), NOW));
    assert!(
        report.restored.values().all(|n| *n == 0),
        "{:?}",
        report.restored
    );
    assert_eq!(report.pending, 0);
    assert_eq!(report.gap_started_at, None);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), tampered);

    let copy = sibling(&path, &format!(".corrupt-{NOW}"));
    assert_eq!(std::fs::read_to_string(&copy).unwrap(), tampered);
    assert!(report
        .quarantine_files
        .contains(&copy.display().to_string()));
    eprintln!("quarantine set: {:?}", report.quarantine_files);
    assert_eq!(report.quarantine_files.len(), 4);
    for name in &report.quarantine_files {
        assert!(Path::new(name).exists(), "{name} is named and missing");
    }
}

/// §48.8.1: the generation continues from the source, so the first export after a rebuild is the
/// next one, never generation 1 again.
#[test]
fn the_generation_continues_from_the_source() {
    let dir = tempfile::tempdir().unwrap();
    seed(dir.path());
    corrupt(dir.path());
    rebuilt(rebuild_in_place(dir.path(), NOW));

    let index = Index::open_at(dir.path(), NOW).unwrap();
    assert_eq!(
        index.app_meta("sidecar_generation").unwrap().as_deref(),
        Some("7")
    );
    assert_eq!(index.export_sidecar(NOW + 1).unwrap().generation, 8);
}

/// The global half lands now; every per-project record waits in the pending table for the
/// hand-off that brings its subject back — the session is not written to a project that does not
/// exist yet.
#[test]
fn subject_records_are_staged_not_applied() {
    let dir = tempfile::tempdir().unwrap();
    let doc = seed(dir.path());
    corrupt(dir.path());
    let report = rebuilt(rebuild_in_place(dir.path(), NOW));
    eprintln!("restored {:?}, pending {}", report.restored, report.pending);
    for key in ["roots", "identities", "collections", "view_state"] {
        assert_eq!(report.restored.get(key), Some(&1), "{key}");
    }
    assert!(report.restored["settings"] >= 1);
    assert_eq!(
        report.pending, 2,
        "the earlier rebuild's row and this project"
    );
    assert_eq!(report.gap_started_at, Some(900));

    let index = Index::open_at(dir.path(), NOW).unwrap();
    let conn = index.conn();
    for table in ["project", "location", "session", "session_segment"] {
        let n: i64 = conn
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0, "{table} was written before its subject came back");
    }
    let rows: Vec<(i64, String, String, String, i64)> = conn
        .prepare(
            "SELECT source_generation, subject_key, location_keys, record, queued_at
             FROM sidecar_pending ORDER BY id",
        )
        .unwrap()
        .query_map([], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows[0],
        (
            3,
            "path:linux::2f6f6c64".to_owned(),
            "[]".to_owned(),
            "{\"left\":\"by an earlier rebuild\"}".to_owned(),
            50
        ),
        "a row the document carried is staged verbatim"
    );

    let project = &doc.payload.projects[0];
    let (generation, subject, keys, record, queued_at) = &rows[1];
    assert_eq!(*generation, 7);
    assert_eq!(subject, &project.subject);
    assert_eq!(*queued_at, NOW);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(keys).unwrap(),
        serde_json::to_value(&project.location_keys).unwrap()
    );
    match serde_json::from_str::<PendingRecord>(record).unwrap() {
        PendingRecord::Project {
            record: staged,
            member_of,
        } => {
            assert_eq!(&staged, project);
            assert_eq!(member_of, vec!["By hand".to_owned()]);
        }
        other @ PendingRecord::Section { .. } => panic!("the project was staged as {other:?}"),
    }
}
