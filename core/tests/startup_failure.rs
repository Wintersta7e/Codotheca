#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

//! §11.2a's three database windows, reported through a file the shell reads without ever
//! opening the database.

use codotheca_core::index::recovery::RebuildReport;
use codotheca_core::index::sidecar::{RestoreCounts, SidecarCounts};
use codotheca_core::index::IndexError;
use codotheca_core::surfaces::startup_failure::{self, StartupFailure};

#[test]
fn a_future_schema_becomes_a_report_naming_both_numbers() {
    let err = IndexError::SchemaFromFuture {
        on_disk: 9,
        supported: 5,
    };
    let failure = startup_failure::from_index_error(&err, 100).expect("recognised");
    assert!(matches!(
        failure,
        StartupFailure::SchemaFromFuture {
            on_disk: 9,
            supported: 5
        }
    ));
}

#[test]
fn a_failed_migration_keeps_the_schema_it_was_restored_to_and_when() {
    let err = IndexError::MigrationFailed {
        version: 4,
        name: "sessions",
        restored_to: 3,
        restored_at: 777,
        detail: "constraint".into(),
    };
    let failure = startup_failure::from_index_error(&err, 100).expect("recognised");
    match failure {
        StartupFailure::MigrationFailed {
            version,
            restored_to,
            restored_at,
            name,
        } => {
            assert_eq!((version, restored_to, restored_at), (4, 3, 777));
            assert_eq!(name, "sessions");
        }
        other => panic!("wrong variant: {other:?}"),
    }
}

#[test]
fn an_error_the_shell_has_no_window_for_produces_no_report() {
    let err = IndexError::CompletionNotComputable;
    assert!(startup_failure::from_index_error(&err, 100).is_none());
}

#[test]
fn a_corrupt_index_with_no_rebuild_yet_reports_no_counts_rather_than_zeroes() {
    let err = IndexError::Corrupt {
        detail: "file is not a database".into(),
    };
    let failure = startup_failure::from_index_error(&err, 100).expect("recognised");
    match failure {
        StartupFailure::CorruptIndex {
            quarantined_at,
            gap_started_at,
            gap_counts_recoverable,
            re_derivable,
            restorable,
        } => {
            assert_eq!(quarantined_at, 100);
            assert_eq!(gap_started_at, None);
            assert!(!gap_counts_recoverable);
            assert_eq!(
                (re_derivable, restorable),
                (None, None),
                "no rebuild has run, so the ledger has no figures — and `0 projects restorable` \
                 would be a claim, on the screen where unknown-as-zero matters most"
            );
        }
        other => panic!("wrong variant: {other:?}"),
    }
}

#[test]
fn a_rebuild_report_is_what_puts_real_figures_in_the_ledger() {
    let report = RebuildReport {
        quarantined: codotheca_core::index::recovery::QuarantinedFiles {
            db: std::path::PathBuf::from("index.db.corrupt-900"),
            wal: None,
            shm: None,
            at: 900,
        },
        restored: RestoreCounts {
            projects: 12,
            notes: 3,
            sessions: 40,
            collections: 2,
            roots: 1,
            xp_events: 7,
            launch_targets: 5,
            ..RestoreCounts::default()
        },
        deferred: SidecarCounts {
            projects: 4,
            ..SidecarCounts::default()
        },
        gap_started_at: Some(555),
        gap_counts_recoverable: false,
    };
    let failure = startup_failure::corrupt_from_report(&report);
    match failure {
        StartupFailure::CorruptIndex {
            quarantined_at,
            gap_started_at,
            re_derivable,
            restorable,
            ..
        } => {
            assert_eq!(
                quarantined_at, 900,
                "the quarantine already happened; its own timestamp is when"
            );
            assert_eq!(gap_started_at, Some(555));
            let restorable = restorable.expect("a rebuild ran, so this block has figures");
            assert_eq!(restorable.projects, 12);
            assert_eq!(restorable.sessions, 40);
            assert_eq!(
                re_derivable
                    .expect("deferred rows are the re-derivable block")
                    .projects,
                4
            );
        }
        other => panic!("wrong variant: {other:?}"),
    }
}

#[test]
fn the_report_round_trips_through_the_file_and_is_cleared() {
    let dir = tempfile::tempdir().expect("tempdir");
    let failure = StartupFailure::SchemaFromFuture {
        on_disk: 9,
        supported: 5,
    };
    startup_failure::write(dir.path(), &failure).expect("write");
    let path = dir.path().join(startup_failure::STARTUP_FAILURE_FILE);
    assert!(path.exists());
    let text = std::fs::read_to_string(&path).expect("read");
    assert!(
        text.contains("\"onDisk\": 9"),
        "camelCase, so the shell reads it unchanged"
    );
    assert!(
        text.contains("\"kind\": \"schema_from_future\""),
        "the tag is what the shell switches on"
    );
    startup_failure::clear(dir.path());
    assert!(!path.exists());
}

#[test]
fn clearing_a_report_that_was_never_written_is_not_an_error() {
    let dir = tempfile::tempdir().expect("tempdir");
    startup_failure::clear(dir.path());
    assert!(!dir
        .path()
        .join(startup_failure::STARTUP_FAILURE_FILE)
        .exists());
}
