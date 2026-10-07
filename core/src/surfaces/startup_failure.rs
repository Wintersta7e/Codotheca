//! §11.2a's three database windows. The core cannot answer a command it never
//! reached, and §1.10 forbids the shell from opening the database, so the core
//! leaves one report beside it and exits.
//!
//! The closed `ErrorCode` enum has no variant for any of the three, which is why this is a
//! file and not a frame: there is no protocol connection to carry it on.

use std::collections::BTreeMap;
use std::path::Path;

use crate::index::migrate::SUPPORTED_SCHEMA_VERSION;
use crate::index::sidecar::{self, SidecarState};
use crate::index::{Index, IndexError};

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
    /// The database was not a readable database. Nothing was moved: the report states what a
    /// rebuild would restore from, and a rebuild asked for and failed says why.
    #[serde(rename_all = "camelCase")]
    CorruptIndex {
        /// The sidecar as the rebuild would find it, read and never restored.
        sidecar: SidecarReport,
        /// Why the rebuild REBUILD asked for failed, and anything it could not undo; `None`
        /// before one was asked for. REBUILD may be asked for again.
        rebuild_failed: Option<String>,
        /// Whether the gap's contents can be counted. Always `false`, because what was made in
        /// the gap was recorded only in the database that cannot be read.
        gap_counts_recoverable: bool,
    },
}

/// §48.7.1 step 1's view of the sidecar: its state and, only when it reads, what it holds. A
/// field this state cannot know is `None`, never a zero.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SidecarReport {
    /// Which of the reader's four answers the sidecar gave.
    pub state: SidecarReportState,
    /// When it was written, in Unix seconds — where the gap starts. Only when it reads.
    pub written_at: Option<i64>,
    /// The export it is. Only when it reads.
    pub generation: Option<u64>,
    /// How many of each record a rebuild would restore, by the sidecar's count keys. Only when
    /// it reads.
    pub counts: Option<BTreeMap<String, u64>>,
    /// Why it cannot be used, in the reader's words. Only when unreadable or newer.
    pub reason: Option<String>,
}

/// The sidecar reader's four answers, as the report spells them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SidecarReportState {
    /// It reads in full; a rebuild restores from it.
    Present,
    /// There is none; a rebuild restores nothing.
    Absent,
    /// It exists and cannot be used; a rebuild restores nothing and keeps the file.
    Unreadable,
    /// A newer build wrote it; a rebuild would restore nothing rather than part of it.
    Newer,
}

impl SidecarReport {
    /// The report of what [`sidecar::inspect`] found.
    #[must_use]
    pub fn of(state: &SidecarState) -> Self {
        let empty = |answer, reason| Self {
            state: answer,
            written_at: None,
            generation: None,
            counts: None,
            reason,
        };
        match state {
            SidecarState::Present(doc) => Self {
                state: SidecarReportState::Present,
                written_at: Some(doc.written_at),
                generation: Some(doc.generation),
                counts: Some(sidecar::counts(doc)),
                reason: None,
            },
            SidecarState::Absent => empty(SidecarReportState::Absent, None),
            SidecarState::Unreadable { reason } => {
                empty(SidecarReportState::Unreadable, Some(reason.clone()))
            }
            SidecarState::Newer { reason } => {
                empty(SidecarReportState::Newer, Some(reason.clone()))
            }
        }
    }
}

/// The `corrupt_index` report for `data_dir`: its sidecar read — never restored — and, when a
/// rebuild was asked for and failed, why.
#[must_use]
pub fn corrupt_index(data_dir: &Path, rebuild_failed: Option<String>) -> StartupFailure {
    let state = sidecar::inspect(&Index::sidecar_path(data_dir), SUPPORTED_SCHEMA_VERSION);
    StartupFailure::CorruptIndex {
        sidecar: SidecarReport::of(&state),
        rebuild_failed,
        gap_counts_recoverable: false,
    }
}

/// The report for one of the three windows §11.2a draws, or `None`.
///
/// Everything else is a normal command failure and reaches the user through the protocol. A
/// corrupt index reads the sidecar in `data_dir` for its report.
#[must_use]
pub fn from_index_error(err: &IndexError, data_dir: &Path) -> Option<StartupFailure> {
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
        IndexError::Corrupt { .. } => Some(corrupt_index(data_dir, None)),
        _ => None,
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
