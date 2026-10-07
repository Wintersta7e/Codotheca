//! §1.12's two recovery rules, which are opposites and are easy to swap.
//!
//! An orphaned WAL is normal crash recovery — open and let SQLite recover, and never inspect
//! `-wal` for suspicion. A `SQLITE_NOTADB` or `SQLITE_CORRUPT` is not recoverable, and its
//! three files are quarantined together because moving one alone leaves a stale WAL to be
//! replayed into the rebuild.

use std::path::{Path, PathBuf};

use super::rebuild::RebuildStep;
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

/// Where an unreadable database's files were moved, so the user can still find them.
#[derive(Debug, Clone, serde::Serialize)]
pub struct QuarantinedFiles {
    /// The moved database file — always present, because a quarantine with none fails instead.
    pub db: PathBuf,
    /// The moved `-wal`, or `None` when there was none beside the database.
    pub wal: Option<PathBuf>,
    /// The moved `-shm`, or `None` when there was none beside the database.
    pub shm: Option<PathBuf>,
    /// The copy of the sidecar set beside them, or `None` when no sidecar file existed.
    pub sidecar_copy: Option<PathBuf>,
    /// The quarantine time in Unix seconds, and the suffix every moved name carries.
    pub at: i64,
}

/// Move `index.db`, `index.db-wal` and `index.db-shm` to `<name>.corrupt-<now>` siblings.
///
/// # Errors
/// Fails with [`IndexError::Corrupt`] when the database file does not exist, and with
/// [`IndexError::Io`] when a rename fails.
pub fn quarantine(db: &Path, now: i64) -> Result<QuarantinedFiles, IndexError> {
    let moved_db = move_aside(db, now)?.ok_or_else(|| IndexError::Corrupt {
        detail: format!("{} does not exist", db.display()),
    })?;
    Ok(QuarantinedFiles {
        db: moved_db,
        wal: move_aside(&sibling(db, "-wal"), now)?,
        shm: move_aside(&sibling(db, "-shm"), now)?,
        sidecar_copy: None,
        at: now,
    })
}

/// §48.7.1 4(c): move `index.db`, `-wal` and `-shm` to `<name>.corrupt-<now>` siblings and
/// **copy** the sidecar beside them as `<name>.corrupt-<now>`.
///
/// The sidecar is copied whether it read or not — an unreadable one is kept too — and the
/// original stays where it was. Each act already done is undone when a later one fails, and a
/// destination that already exists fails the set rather than being replaced: the product never
/// deletes a quarantined file.
///
/// # Errors
/// Fails with [`IndexError::Corrupt`] when the database file does not exist, and with
/// [`IndexError::Io`] when a destination exists or a rename or the copy fails.
pub fn quarantine_set(db: &Path, sidecar: &Path, now: i64) -> Result<QuarantinedFiles, IndexError> {
    let mut undo = Undo::default();
    quarantine_set_with(db, sidecar, now, &|_| Ok(()), &mut undo).map_err(|e| undo.after(e))
}

/// [`quarantine_set`] with `step` run after each act and every act recorded in `undo`, so the
/// rebuild can fail between any two and take the set back along with its own acts.
pub(crate) fn quarantine_set_with(
    db: &Path,
    sidecar: &Path,
    now: i64,
    step: &dyn Fn(RebuildStep) -> Result<(), IndexError>,
    undo: &mut Undo,
) -> Result<QuarantinedFiles, IndexError> {
    if !db.exists() {
        return Err(IndexError::Corrupt {
            detail: format!("{} does not exist", db.display()),
        });
    }
    let moved_db = corrupt_name(db, now);
    undo.rename(db, &moved_db)?;
    step(RebuildStep::QuarantinedDb)?;

    let mut siblings = [None, None];
    for (slot, suffix) in siblings.iter_mut().zip(["-wal", "-shm"]) {
        let from = sibling(db, suffix);
        if from.exists() {
            let to = corrupt_name(&from, now);
            undo.rename(&from, &to)?;
            *slot = Some(to);
        }
    }
    let [wal, shm] = siblings;
    step(RebuildStep::QuarantinedSiblings)?;

    let sidecar_copy = if sidecar.is_file() {
        let to = corrupt_name(sidecar, now);
        refuse_taken(&to)?;
        undo.created(to.clone());
        std::fs::copy(sidecar, &to)?;
        Some(to)
    } else {
        None
    };
    step(RebuildStep::CopiedSidecar)?;

    Ok(QuarantinedFiles {
        db: moved_db,
        wal,
        shm,
        sidecar_copy,
        at: now,
    })
}

/// What a rebuild did to the data directory, in order, so a failure can take all of it back.
#[derive(Debug, Default)]
pub(crate) struct Undo {
    renamed: Vec<(PathBuf, PathBuf)>,
    created: Vec<PathBuf>,
}

impl Undo {
    /// Record `path` as a file this process is about to create.
    pub(crate) fn created(&mut self, path: PathBuf) {
        self.created.push(path);
    }

    /// Rename `from` to `to` and record it, refusing a `to` that already exists.
    pub(crate) fn rename(&mut self, from: &Path, to: &Path) -> Result<(), IndexError> {
        refuse_taken(to)?;
        std::fs::rename(from, to)?;
        self.renamed.push((from.to_path_buf(), to.to_path_buf()));
        Ok(())
    }

    /// Reverse every rename, newest first, then remove every file this process recorded as
    /// created, and return `error` naming any act that could not be undone.
    pub(crate) fn after(self, error: IndexError) -> IndexError {
        let mut failed = Vec::new();
        for (from, to) in self.renamed.iter().rev() {
            if let Err(e) = std::fs::rename(to, from) {
                failed.push(format!("{} back to {}: {e}", to.display(), from.display()));
            }
        }
        for created in &self.created {
            // App-owned bytes this process made moments ago; one already gone is the goal.
            match std::fs::remove_file(created) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                    failed.push(format!("{}: {e}", created.display()));
                }
                _ => {}
            }
        }
        if failed.is_empty() {
            error
        } else {
            IndexError::Io(std::io::Error::other(format!(
                "{error}; and could not undo {}",
                failed.join("; ")
            )))
        }
    }
}

/// Fail when `to` names anything, a dangling link included.
pub(crate) fn refuse_taken(to: &Path) -> Result<(), IndexError> {
    if std::fs::symlink_metadata(to).is_ok() {
        return Err(IndexError::Io(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            format!("{} already exists", to.display()),
        )));
    }
    Ok(())
}

/// `path` with `suffix` appended to its file name.
pub(crate) fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut p = path.as_os_str().to_os_string();
    p.push(suffix);
    PathBuf::from(p)
}

fn corrupt_name(path: &Path, now: i64) -> PathBuf {
    sibling(path, &format!(".corrupt-{now}"))
}

fn move_aside(path: &Path, now: i64) -> Result<Option<PathBuf>, IndexError> {
    if !path.exists() {
        return Ok(None);
    }
    let dest = corrupt_name(path, now);
    std::fs::rename(path, &dest)?;
    Ok(Some(dest))
}
