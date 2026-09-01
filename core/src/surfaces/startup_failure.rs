//! §11.2a's three database windows. The core cannot answer a command it never
//! reached, and §1.10 forbids the shell from opening the database, so the core
//! leaves one report beside it and exits.
//!
//! The closed `ErrorCode` enum has no variant for any of the three, which is why this is a
//! file and not a frame: there is no protocol connection to carry it on.

use std::path::Path;

use crate::index::recovery::RebuildReport;
use crate::index::sidecar::{RestoreCounts, SidecarCounts};
use crate::index::IndexError;

pub const STARTUP_FAILURE_FILE: &str = "startup-failure.json";
pub const EXIT_INDEX_FATAL: u8 = 4;

/// One block of §11.2a's ledger. Only the rows §11.2a names, not the sidecar's full set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LedgerCounts {
    pub projects: u64,
    pub notes: u64,
    pub sessions: u64,
    pub collections: u64,
    pub roots: u64,
    pub xp_events: u64,
    pub launch_targets: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StartupFailure {
    #[serde(rename_all = "camelCase")]
    SchemaFromFuture { on_disk: u32, supported: u32 },
    #[serde(rename_all = "camelCase")]
    MigrationFailed {
        version: u32,
        name: String,
        restored_to: u32,
        restored_at: i64,
    },
    #[serde(rename_all = "camelCase")]
    CorruptIndex {
        quarantined_at: i64,
        gap_started_at: Option<i64>,
        gap_counts_recoverable: bool,
        /// `None` until a rebuild has run. `core/src/index/recovery.rs:73` already records
        /// why: a figure nobody computed must not print as `0` on the one screen whose
        /// subject is what was lost.
        re_derivable: Option<LedgerCounts>,
        restorable: Option<LedgerCounts>,
    },
}

/// Only the three windows §11.2a draws produce a report; everything else is a normal
/// command failure and reaches the user through the protocol.
#[must_use]
pub fn from_index_error(err: &IndexError, now: i64) -> Option<StartupFailure> {
    match err {
        IndexError::SchemaFromFuture { on_disk, supported } => {
            Some(StartupFailure::SchemaFromFuture {
                on_disk: *on_disk,
                supported: *supported,
            })
        }
        IndexError::MigrationFailed {
            version,
            name,
            restored_to,
            restored_at,
            ..
        } => Some(StartupFailure::MigrationFailed {
            version: *version,
            name: (*name).to_owned(),
            restored_to: *restored_to,
            restored_at: *restored_at,
        }),
        // A bare `Corrupt` carries no counts, so the ledger has none to state.
        IndexError::Corrupt { .. } => Some(StartupFailure::CorruptIndex {
            quarantined_at: now,
            gap_started_at: None,
            gap_counts_recoverable: false,
            re_derivable: None,
            restorable: None,
        }),
        _ => None,
    }
}

/// The report a rebuild produces, with the two blocks §11.2a can actually fill.
///
/// `restorable` is what came back from the sidecar; `re_derivable` is what is waiting for a
/// scan to re-discover its subject. The third block — the gap — keeps
/// `gap_counts_recoverable: false`, because what was made after `gap_started_at` was recorded
/// only in the database that was destroyed.
/// Takes no `now`: the quarantine already happened, and `QuarantinedFiles.at` is when.
#[must_use]
pub fn corrupt_from_report(report: &RebuildReport) -> StartupFailure {
    StartupFailure::CorruptIndex {
        quarantined_at: report.quarantined.at,
        gap_started_at: report.gap_started_at,
        gap_counts_recoverable: report.gap_counts_recoverable,
        re_derivable: Some(from_sidecar_counts(report.deferred)),
        restorable: Some(from_restore_counts(report.restored)),
    }
}

fn from_restore_counts(c: RestoreCounts) -> LedgerCounts {
    LedgerCounts {
        projects: c.projects,
        notes: c.notes,
        sessions: c.sessions,
        collections: c.collections,
        roots: c.roots,
        xp_events: c.xp_events,
        launch_targets: c.launch_targets,
    }
}

fn from_sidecar_counts(c: SidecarCounts) -> LedgerCounts {
    LedgerCounts {
        projects: c.projects,
        notes: c.notes,
        sessions: c.sessions,
        collections: c.collections,
        roots: c.roots,
        xp_events: c.xp_events,
        launch_targets: c.launch_targets,
    }
}

/// Temp-and-rename, so the shell never reads a half-written report.
///
/// # Errors
/// Fails when the report cannot be serialised or the file cannot be written.
pub fn write(data_dir: &Path, failure: &StartupFailure) -> std::io::Result<()> {
    let path = data_dir.join(STARTUP_FAILURE_FILE);
    let tmp = path.with_extension("json.tmp");
    let text = serde_json::to_vec_pretty(failure)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, &path)
}

/// Clearing a report that was never written is not an error: the caller clears on every
/// successful open, and most opens succeed.
pub fn clear(data_dir: &Path) {
    let _ = std::fs::remove_file(data_dir.join(STARTUP_FAILURE_FILE));
}
