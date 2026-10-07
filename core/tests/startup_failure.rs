#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

//! §11.2a's three database windows, reported through a file the shell reads without ever
//! opening the database.

use std::path::Path;

use codotheca_core::index::IndexError;
use codotheca_core::surfaces::startup_failure::{
    self, SidecarReport, SidecarReportState, StartupFailure,
};

#[test]
fn a_future_schema_becomes_a_report_naming_both_numbers() {
    let err = IndexError::SchemaFromFuture {
        on_disk: 9,
        supported: 5,
    };
    let failure = startup_failure::from_index_error(&err, Path::new("unused")).expect("recognised");
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
    let failure = startup_failure::from_index_error(&err, Path::new("unused")).expect("recognised");
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
    assert!(startup_failure::from_index_error(&err, Path::new("unused")).is_none());
}

/// §48.7.1 step 1: the report states the sidecar as the rebuild would find it and claims no
/// quarantine. A corrupt index met with no sidecar beside it reports `absent` and no figure.
#[test]
fn a_corrupt_index_reports_the_sidecar_state_and_no_quarantine() {
    let dir = tempfile::tempdir().expect("tempdir");
    let err = IndexError::Corrupt {
        detail: "file is not a database".into(),
    };
    let failure = startup_failure::from_index_error(&err, dir.path()).expect("recognised");
    match failure {
        StartupFailure::CorruptIndex {
            sidecar,
            rebuild_failed,
            gap_counts_recoverable,
        } => {
            assert_eq!(
                sidecar,
                SidecarReport {
                    state: SidecarReportState::Absent,
                    written_at: None,
                    generation: None,
                    counts: None,
                    reason: None,
                },
                "no sidecar, so no figure — `0 projects restorable` would be a claim, on the \
                 screen where unknown-as-zero matters most"
            );
            assert_eq!(rebuild_failed, None);
            assert!(!gap_counts_recoverable);
        }
        other => panic!("wrong variant: {other:?}"),
    }
}

/// The corrupt-index report on disk carries exactly these keys, camelCase, with every absent
/// value written as `null` rather than left out — the shell's reader refuses any other set.
#[test]
fn the_corrupt_index_report_serialises_every_key() {
    let failure = StartupFailure::CorruptIndex {
        sidecar: SidecarReport {
            state: SidecarReportState::Unreadable,
            written_at: None,
            generation: None,
            counts: None,
            reason: Some("checksum".to_owned()),
        },
        rebuild_failed: Some("the side index exists".to_owned()),
        gap_counts_recoverable: false,
    };
    let doc = serde_json::to_value(&failure).expect("json");
    assert_eq!(
        doc,
        serde_json::json!({
            "kind": "corrupt_index",
            "sidecar": {
                "state": "unreadable",
                "writtenAt": null,
                "generation": null,
                "counts": null,
                "reason": "checksum",
            },
            "rebuildFailed": "the side index exists",
            "gapCountsRecoverable": false,
        })
    );
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
