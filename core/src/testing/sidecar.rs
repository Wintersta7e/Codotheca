//! Fixtures for the sidecar's registered sections: one per section, so every registered section
//! can be populated by a test — and the indexes the core cannot open, one per startup report.

use std::path::Path;

use rusqlite::Transaction;

use crate::index::migrate::SUPPORTED_SCHEMA_VERSION;
use crate::index::{open_connection, Index, IndexError};
use crate::protocol::{LocationId, ProjectId};

/// The rows a fixture library already holds, for a section fixture to plant its own against.
#[derive(Debug, Clone, Default)]
pub struct FixtureIds {
    /// The library's projects, in creation order.
    pub projects: Vec<ProjectId>,
    /// The library's locations, in creation order.
    pub locations: Vec<LocationId>,
}

/// A fixture that plants rows for one registered section.
pub type SectionFixture = fn(&Transaction<'_>, &FixtureIds) -> Result<(), IndexError>;

/// One fixture per registered section, by section name. A section registers its fixture in the
/// change that registers the section; `sidecar_registry` holds the two lists equal.
pub const SECTION_FIXTURES: &[(&str, SectionFixture)] =
    &[("no_scan_projects", uninstalled_project)];

/// An uninstalled project — both its copies removed — with a note and one session.
fn uninstalled_project(tx: &Transaction<'_>, _ids: &FixtureIds) -> Result<(), IndexError> {
    tx.execute(
        "INSERT INTO project (name, seed_basename, lineage_key, notes, created_at, updated_at)
         VALUES ('uninstalled', 'uninstalled', 'uninstalled-lineage', 'kept after removal', 1, 1)",
        [],
    )?;
    let project = tx.last_insert_rowid();
    for (path, removed_at) in [
        ("/fixture/uninstalled-a", 10_i64),
        ("/fixture/uninstalled-b", 11),
    ] {
        tx.execute(
            "INSERT INTO location (project_id, kind, path_bytes, path_key, path_display,
                                   store_key, presence, repo_kind, removed_at)
             VALUES (?1, 'linux', ?2, ?2, ?3, 'fixture', 'missing', 'worktree', ?4)",
            rusqlite::params![project, path.as_bytes(), path, removed_at],
        )?;
    }
    tx.execute(
        "INSERT INTO session (project_id, location_id, started_at, ended_at, credited_seconds,
                              close_reason)
         VALUES (?1, ?2, 100, 160, 60, 'stop')",
        [project, tx.last_insert_rowid()],
    )?;
    Ok(())
}

/// An index the core cannot open, one per startup report window the shell paints (§11.2a).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupPlant {
    /// Bytes that are not a database beside a junk `-wal` and `-shm`: the `corrupt_index` report.
    Corrupt,
    /// A real database stamped one schema past this build's: the `schema_from_future` report.
    Future,
    /// An empty database stamped schema 5. Step 6 alters `location`, which an empty database does
    /// not have, so every later build stops there: the `migration_failed` report.
    MigrationFailed,
}

impl StartupPlant {
    /// The plant a `--kind` word names, or `None` for any other word.
    #[must_use]
    pub fn from_word(word: &str) -> Option<Self> {
        match word {
            "corrupt" => Some(Self::Corrupt),
            "future" => Some(Self::Future),
            "migration-failed" => Some(Self::MigrationFailed),
            _ => None,
        }
    }

    /// The `--kind` word for this plant, and the `kind` its result records.
    #[must_use]
    pub const fn word(self) -> &'static str {
        match self {
            Self::Corrupt => "corrupt",
            Self::Future => "future",
            Self::MigrationFailed => "migration-failed",
        }
    }
}

/// Plants `plant` in `data_dir`, so the next core started there exits with that plant's report,
/// and answers the names of the files the directory then holds, sorted.
///
/// # Errors
///
/// A filesystem or `SQLite` error while writing the files.
pub fn plant_startup_failure(
    plant: StartupPlant,
    data_dir: &Path,
    now: i64,
) -> Result<Vec<String>, IndexError> {
    let db = Index::db_path(data_dir);
    match plant {
        StartupPlant::Corrupt => {
            std::fs::create_dir_all(data_dir)?;
            std::fs::write(&db, b"this is not a database")?;
            std::fs::write(data_dir.join("index.db-wal"), b"a stale journal")?;
            std::fs::write(data_dir.join("index.db-shm"), b"a stale wal-index")?;
        }
        StartupPlant::Future => {
            drop(Index::open_at(data_dir, now)?);
            open_connection(&db)?.pragma_update(
                None,
                "user_version",
                SUPPORTED_SCHEMA_VERSION + 1,
            )?;
        }
        StartupPlant::MigrationFailed => {
            open_connection(&db)?.pragma_update(None, "user_version", 5)?;
        }
    }
    let mut names = std::fs::read_dir(data_dir)?
        .map(|entry| entry.map(|e| e.file_name().to_string_lossy().into_owned()))
        .collect::<Result<Vec<_>, _>>()?;
    names.sort();
    Ok(names)
}
