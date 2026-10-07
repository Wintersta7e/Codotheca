//! Fixtures for the sidecar's registered sections: one per section, so every registered section
//! can be populated by a test.

use rusqlite::Transaction;

use crate::index::IndexError;
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
