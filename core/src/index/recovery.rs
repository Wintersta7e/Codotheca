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

/// §48.7.1 4(c): move `index.db`, `-wal` and `-shm` to `<name>.corrupt-<now>` siblings and
/// **copy** the sidecar beside them as `<name>.corrupt-<now>`, with `step` run after each act and
/// every act recorded in `undo`, so the rebuild can fail between any two and take the set back
/// along with its own acts.
///
/// The sidecar is copied whether it read or not — an unreadable one is kept too — and the
/// original stays where it was. A destination that already exists fails the set rather than
/// being replaced: the product never deletes a quarantined file.
pub(crate) fn quarantine_set(
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
    /// created, and describe `error` followed by any act that could not be undone — with whether
    /// every act was.
    pub(crate) fn after(self, error: &IndexError) -> (String, bool) {
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
            (error.to_string(), true)
        } else {
            let reason = format!("{error}; and could not undo {}", failed.join("; "));
            (reason, false)
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

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use super::{quarantine_set, sibling, IndexError, QuarantinedFiles, Undo};
    use crate::index::Index;
    use std::path::Path;

    /// A failure left partly undone is described once: the cause in its own words, then what
    /// stayed. Wrapping the cause in a second error printed its category twice.
    #[test]
    fn a_failure_left_partly_undone_names_its_cause_once() {
        let dir = tempfile::tempdir().unwrap();
        // Recorded as created but a directory, so removing it as a file fails.
        let stuck = dir.path().join("stuck");
        std::fs::create_dir(&stuck).unwrap();
        let mut undo = Undo::default();
        undo.created(stuck);

        let (reason, undone) = undo.after(&IndexError::Io(std::io::Error::other("planted")));
        assert_eq!(reason.matches("io:").count(), 1, "{reason}");
        assert!(reason.contains("could not undo"), "{reason}");
        assert!(!undone, "{reason}");
    }

    /// The set with no sidecar beside it and a hook that never fails.
    fn set_aside(db: &Path, now: i64) -> QuarantinedFiles {
        let sidecar = db.with_file_name("index-sidecar.json");
        quarantine_set(db, &sidecar, now, &|_| Ok(()), &mut Undo::default()).unwrap()
    }

    #[test]
    fn quarantine_moves_the_database_wal_and_shm_together() {
        let dir = tempfile::tempdir().unwrap();
        let db = Index::db_path(dir.path());
        for suffix in ["", "-wal", "-shm"] {
            std::fs::write(sibling(&db, suffix), b"x").unwrap();
        }

        let QuarantinedFiles {
            db: moved_db,
            wal,
            shm,
            at,
            sidecar_copy,
        } = set_aside(&db, 1_787_126_520);
        assert_eq!(at, 1_787_126_520);
        assert!(
            !db.exists(),
            "the corrupt database must not be left in place"
        );
        assert!(moved_db.exists());
        assert_eq!(
            moved_db.file_name().unwrap().to_string_lossy(),
            "index.db.corrupt-1787126520"
        );
        assert!(
            wal.unwrap().exists(),
            "a stale WAL left behind would be replayed into the rebuild"
        );
        assert!(shm.unwrap().exists());
        assert_eq!(sidecar_copy, None, "no sidecar existed to copy");
    }

    #[test]
    fn quarantine_tolerates_a_database_with_no_wal_or_shm() {
        let dir = tempfile::tempdir().unwrap();
        let db = Index::db_path(dir.path());
        std::fs::write(&db, b"x").unwrap();

        let set = set_aside(&db, 7);
        assert_eq!((set.wal, set.shm), (None, None));
    }
}
