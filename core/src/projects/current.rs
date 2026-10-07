//! §46.9's current-state predicate.
//!
//! A project is current when the user has not removed it and at least one of its copies has not
//! been removed. Presence is never read: an offline copy is still current, because unplugging a
//! drive is not removing a project, and an uninstalled copy leaves `presence` as it was.

use rusqlite::{Connection, OptionalExtension as _};

use crate::index::IndexError;
use crate::protocol::{LocationId, ProjectId};

/// The predicate as a SQL boolean over a `project` row aliased `p`, so a list query filters on
/// exactly what [`is_current`] answers rather than a second spelling of it.
pub const IS_CURRENT_SQL: &str = "(p.removed_at IS NULL AND EXISTS (SELECT 1 FROM location cur_l \
     WHERE cur_l.project_id = p.id AND cur_l.removed_at IS NULL))";

/// Whether `project` is current. A project the index does not hold is not.
///
/// # Errors
///
/// [`IndexError::Sqlite`] when the read fails.
pub fn is_current(conn: &Connection, project: ProjectId) -> Result<bool, IndexError> {
    let current: Option<bool> = conn
        .query_row(
            &format!("SELECT {IS_CURRENT_SQL} FROM project p WHERE p.id = ?1"),
            [project.0],
            |row| row.get(0),
        )
        .optional()?;
    Ok(current.unwrap_or(false))
}

/// Whether work may run for `location`: the copy itself is not removed and its project is
/// current. A location the index does not hold is not.
///
/// # Errors
///
/// [`IndexError::Sqlite`] when the read fails.
pub fn location_is_current(conn: &Connection, location: LocationId) -> Result<bool, IndexError> {
    let row: Option<(i64, Option<i64>)> = conn
        .query_row(
            "SELECT project_id, removed_at FROM location WHERE id = ?1",
            [location.0],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    match row {
        Some((project, None)) => is_current(conn, ProjectId(project)),
        _ => Ok(false),
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use rusqlite::Connection;

    use super::{is_current, location_is_current, IS_CURRENT_SQL};
    use crate::index::Index;
    use crate::protocol::{LocationId, ProjectId};

    /// One row of the truth table: the project's own flag, each copy's flag and presence, and
    /// the answer.
    struct Row {
        project_removed: bool,
        copies: &'static [(bool, &'static str)],
        current: bool,
    }

    const TABLE: [Row; 7] = [
        Row {
            project_removed: false,
            copies: &[(false, "present")],
            current: true,
        },
        // Presence is never read: an offline copy nobody removed keeps its project current.
        Row {
            project_removed: false,
            copies: &[(false, "offline")],
            current: true,
        },
        Row {
            project_removed: false,
            copies: &[(true, "present")],
            current: false,
        },
        // A project with no copy at all — never cloned — is not current.
        Row {
            project_removed: false,
            copies: &[],
            current: false,
        },
        Row {
            project_removed: true,
            copies: &[(false, "present")],
            current: false,
        },
        Row {
            project_removed: true,
            copies: &[(true, "present")],
            current: false,
        },
        Row {
            project_removed: false,
            copies: &[(true, "present"), (false, "present")],
            current: true,
        },
    ];

    /// Every row of [`TABLE`] written into one index: each project with its copies, in order.
    fn planted(conn: &Connection) -> Vec<(ProjectId, Vec<LocationId>)> {
        let mut path = 0_u32;
        TABLE
            .iter()
            .map(|row| {
                conn.execute(
                    "INSERT INTO project (name, seed_basename, created_at, updated_at, removed_at)
                     VALUES ('p', 'p', 1, 1, ?1)",
                    [row.project_removed.then_some(5_i64)],
                )
                .unwrap();
                let project = conn.last_insert_rowid();
                let copies = row
                    .copies
                    .iter()
                    .map(|(removed, presence)| {
                        path += 1;
                        let at = format!("/copies/{path}");
                        conn.execute(
                            "INSERT INTO location (project_id, kind, path_bytes, path_key,
                                                   path_display, store_key, presence, repo_kind,
                                                   removed_at)
                             VALUES (?1, 'linux', ?2, ?2, ?3, 'store', ?4, 'worktree', ?5)",
                            rusqlite::params![
                                project,
                                at.as_bytes(),
                                at,
                                presence,
                                removed.then_some(5_i64)
                            ],
                        )
                        .unwrap();
                        LocationId(conn.last_insert_rowid())
                    })
                    .collect();
                (ProjectId(project), copies)
            })
            .collect()
    }

    #[test]
    fn the_truth_table_and_the_fragment_agree() {
        let dir = tempfile::tempdir().unwrap();
        let index = Index::open_at(dir.path(), 0).unwrap();
        let conn = index.conn();
        let projects = planted(conn);

        let mut evaluated = 0_usize;
        for ((project, _), row) in projects.iter().zip(TABLE.iter()) {
            assert_eq!(
                is_current(conn, *project).unwrap(),
                row.current,
                "row {evaluated}: project removed {}, copies {:?}",
                row.project_removed,
                row.copies
            );
            evaluated += 1;
        }
        eprintln!("rows evaluated: {evaluated}");
        assert_eq!(evaluated, TABLE.len());

        // The function and the fragment are one predicate.
        let selected: Vec<i64> = conn
            .prepare(&format!(
                "SELECT p.id FROM project p WHERE {IS_CURRENT_SQL} ORDER BY p.id"
            ))
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        let expected: Vec<i64> = projects
            .iter()
            .zip(TABLE.iter())
            .filter(|(_, row)| row.current)
            .map(|((project, _), _)| project.0)
            .collect();
        eprintln!("projects the fragment selects: {}", selected.len());
        assert_eq!(selected, expected);

        assert!(!is_current(conn, ProjectId(9_999)).unwrap(), "unknown id");
    }

    #[test]
    fn a_location_is_current_only_when_it_and_its_project_are() {
        let dir = tempfile::tempdir().unwrap();
        let index = Index::open_at(dir.path(), 0).unwrap();
        let conn = index.conn();
        let projects = planted(conn);

        // Row 6: a current project's removed copy, then its live one.
        let removed_copy_of_current = projects[6].1[0];
        let live_copy_of_current = projects[6].1[1];
        // Row 4: a live copy of a removed project.
        let live_copy_of_removed = projects[4].1[0];
        // Row 1: the offline copy nobody removed.
        let offline_copy = projects[1].1[0];

        let cases = [
            (removed_copy_of_current, false),
            (live_copy_of_current, true),
            (live_copy_of_removed, false),
            (offline_copy, true),
            (LocationId(9_999), false),
        ];
        for (location, expected) in cases {
            assert_eq!(
                location_is_current(conn, location).unwrap(),
                expected,
                "location {}",
                location.0
            );
        }
        eprintln!("locations evaluated: {}", cases.len());
    }
}
