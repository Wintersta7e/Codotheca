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

/// The report's file name inside the data directory, beside the database it describes.
/// The shell's reader in `app/src/main/startupFailure.ts` names the same file.
pub const STARTUP_FAILURE_FILE: &str = "startup-failure.json";
/// The process exit code after a report is written, so the shell knows to go and read it.
pub const EXIT_INDEX_FATAL: u8 = 4;

/// One block of §11.2a's ledger. Only the rows §11.2a names, not the sidecar's full set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LedgerCounts {
    /// Project rows in this block.
    pub projects: u64,
    /// Projects in this block carrying a note.
    pub notes: u64,
    /// Launched play sessions in this block.
    pub sessions: u64,
    /// Collections in this block.
    pub collections: u64,
    /// Scan roots in this block.
    pub roots: u64,
    /// XP ledger events in this block.
    pub xp_events: u64,
    /// Configured launch targets in this block.
    pub launch_targets: u64,
}

/// Which of §11.2a's three database windows the shell must draw, with what it states.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StartupFailure {
    /// The database was written by a newer build; this one refuses to open it.
    #[serde(rename_all = "camelCase")]
    SchemaFromFuture {
        /// The schema version found in the database.
        on_disk: u32,
        /// The highest schema version this build knows.
        supported: u32,
    },
    /// A migration failed; the pre-migration backup, when one was taken, was restored over the
    /// database.
    #[serde(rename_all = "camelCase")]
    MigrationFailed {
        /// The schema version of the migration that failed.
        version: u32,
        /// That migration's name.
        name: String,
        /// The schema version the database was at before migrating, which it is back at.
        restored_to: u32,
        /// When the failure was handled, in unix seconds.
        restored_at: i64,
    },
    /// The database was not a readable database.
    #[serde(rename_all = "camelCase")]
    CorruptIndex {
        /// When the corruption was met, in unix seconds; once a rebuild has run, the time the
        /// files were quarantined.
        quarantined_at: i64,
        /// When the unrecoverable gap begins: the sidecar's `written_at`. `None` when there was
        /// no sidecar to date it from.
        gap_started_at: Option<i64>,
        /// Whether the gap's contents can be counted. Always `false` today, because what was
        /// made in the gap was recorded only in the destroyed database.
        gap_counts_recoverable: bool,
        /// `None` until a rebuild has run. `core/src/index/recovery.rs:83` already records
        /// why: a figure nobody computed must not print as `0` on the one screen whose
        /// subject is what was lost.
        re_derivable: Option<LedgerCounts>,
        /// What the sidecar restored. `None` until a rebuild has run, for the same reason.
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
pub const fn corrupt_from_report(report: &RebuildReport) -> StartupFailure {
    StartupFailure::CorruptIndex {
        quarantined_at: report.quarantined.at,
        gap_started_at: report.gap_started_at,
        gap_counts_recoverable: report.gap_counts_recoverable,
        re_derivable: Some(from_sidecar_counts(report.deferred)),
        restorable: Some(from_restore_counts(report.restored)),
    }
}

const fn from_restore_counts(c: RestoreCounts) -> LedgerCounts {
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

const fn from_sidecar_counts(c: SidecarCounts) -> LedgerCounts {
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
