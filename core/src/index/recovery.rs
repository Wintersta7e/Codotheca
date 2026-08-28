//! §1.12's two recovery rules, which are opposites and are easy to swap.
//!
//! An orphaned WAL is normal crash recovery — open and let SQLite recover, and never inspect
//! `-wal` for suspicion. A `SQLITE_NOTADB` or `SQLITE_CORRUPT` is not recoverable, and its
//! three files are quarantined together because moving one alone leaves a stale WAL to be
//! replayed into the rebuild.

use std::path::{Path, PathBuf};

use super::IndexError;

/// Distinguish "this database is unreadable" from every other sqlite failure.
#[must_use]
pub fn classify(err: rusqlite::Error) -> IndexError {
    match err.sqlite_error_code() {
        Some(rusqlite::ErrorCode::NotADatabase | rusqlite::ErrorCode::DatabaseCorrupt) => {
            IndexError::Corrupt {
                detail: err.to_string(),
            }
        }
        _ => IndexError::Sqlite(err),
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct QuarantinedFiles {
    pub db: PathBuf,
    pub wal: Option<PathBuf>,
    pub shm: Option<PathBuf>,
    pub at: i64,
}

/// Move `index.db`, `index.db-wal` and `index.db-shm` to `<name>.corrupt-<now>` siblings.
pub fn quarantine(db: &Path, now: i64) -> Result<QuarantinedFiles, IndexError> {
    let moved_db = move_aside(db, now)?.ok_or_else(|| IndexError::Corrupt {
        detail: format!("{} does not exist", db.display()),
    })?;
    Ok(QuarantinedFiles {
        db: moved_db,
        wal: move_aside(&sibling(db, "-wal"), now)?,
        shm: move_aside(&sibling(db, "-shm"), now)?,
        at: now,
    })
}

fn sibling(db: &Path, suffix: &str) -> PathBuf {
    let mut p = db.as_os_str().to_os_string();
    p.push(suffix);
    PathBuf::from(p)
}

fn move_aside(path: &Path, now: i64) -> Result<Option<PathBuf>, IndexError> {
    if !path.exists() {
        return Ok(None);
    }
    let mut dest = path.as_os_str().to_os_string();
    dest.push(format!(".corrupt-{now}"));
    let dest = PathBuf::from(dest);
    std::fs::rename(path, &dest)?;
    Ok(Some(dest))
}

/// What a rebuild did, in the three blocks §11.2a's ledger draws.
#[derive(Debug, Clone)]
pub struct RebuildReport {
    pub quarantined: QuarantinedFiles,
    /// Restored from the sidecar, now.
    pub restored: super::sidecar::RestoreCounts,
    /// In the sidecar and waiting for the scan to re-discover each subject.
    pub deferred: super::sidecar::SidecarCounts,
    /// When the gap starts: the sidecar's own `written_at`. `None` if there was no sidecar.
    pub gap_started_at: Option<i64>,
    /// Always `false`. What was made after `gap_started_at` was recorded only in the database
    /// that was destroyed, so the gap has a window and no counts — and printing `0` there
    /// would be the unknown-as-zero this project bans, on the screen where it matters most.
    pub gap_counts_recoverable: bool,
}
