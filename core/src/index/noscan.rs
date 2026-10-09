//! §48.8.3's `no_scan` section: the projects no scan can ever rediscover, carried whole.
//!
//! A project whose every copy this app removed has no disk left for a scan to find, so the
//! rebuild re-creates it from the sidecar alone — every column of its `project` and `location`
//! rows, its ids kept — and applies its pending records in the same transaction.

use rusqlite::{Connection, Transaction};

use super::sidecar::{
    dump_row, insert_row, location_key_of_row, RestoreCtx, RestoreOutcome, SectionRow, SidecarRow,
    SidecarValue,
};
use super::subject::subject_for_project;
use super::IndexError;
use crate::projects::current::is_current_sql;
use crate::protocol::ProjectId;

/// The `WHERE` fragment over `project p` selecting a project no scan can rediscover.
///
/// One with at least one location that is not current (§46.14): the user removed it, or this app
/// removed every copy of it. A project with no location at all is a not-cloned one — it has no
/// subject, and sync lists it again — so it is not one, removed or not.
pub const NO_SCAN_PREDICATE_SQL: &str = concat!(
    "EXISTS (SELECT 1 FROM location l WHERE l.project_id = p.id) AND NOT ",
    is_current_sql!()
);

/// Whether no scan can ever rediscover `project`, so only the sidecar can bring it back.
///
/// # Errors
/// Fails when SQLite refuses the read.
pub fn is_no_scan(conn: &Connection, project: ProjectId) -> Result<bool, IndexError> {
    let n: i64 = conn.query_row(
        &format!("SELECT count(*) FROM project p WHERE p.id = ?1 AND {NO_SCAN_PREDICATE_SQL}"),
        [project.0],
        |r| r.get(0),
    )?;
    Ok(n != 0)
}

/// One row's data: the project and each of its locations, every column as [`dump_row`] reads it,
/// so a column added later travels without a change here.
#[derive(serde::Serialize, serde::Deserialize)]
struct NoScanProject {
    project: SidecarRow,
    locations: Vec<SidecarRow>,
}

/// The `no_scan_projects` section's export: one row per project [`NO_SCAN_PREDICATE_SQL`]
/// selects, with every column of the project and of each of its locations.
pub(crate) fn export_rows(conn: &Connection) -> Result<Vec<SectionRow>, IndexError> {
    let ids: Vec<i64> = conn
        .prepare(&format!(
            "SELECT p.id FROM project p WHERE {NO_SCAN_PREDICATE_SQL} ORDER BY p.id"
        ))?
        .query_map([], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    let mut out = Vec::new();
    for id in ids {
        let location_ids: Vec<i64> = conn
            .prepare("SELECT id FROM location WHERE project_id = ?1 ORDER BY id")?
            .query_map([id], |r| r.get(0))?
            .collect::<Result<_, _>>()?;
        let locations = location_ids
            .into_iter()
            .map(|location| dump_row(conn, "location", location))
            .collect::<Result<Vec<_>, _>>()?;
        let location_keys = locations
            .iter()
            .map(location_key_of_row)
            .collect::<Result<_, _>>()?;
        let data = NoScanProject {
            project: dump_row(conn, "project", id)?,
            locations,
        };
        out.push(SectionRow {
            subject: subject_for_project(conn, ProjectId(id))?.map(|s| s.to_key()),
            location_keys,
            data: serde_json::to_value(data).map_err(|e| IndexError::Sidecar(e.to_string()))?,
        });
    }
    Ok(out)
}

/// The section's restore: re-create the project, its id kept, then each location with its id.
/// Its pending records apply after the rebuild stages them, in the same transaction.
///
/// Answers one per project, the unit the section's count and its noun are in — not the rows
/// written, which would count each project once more per copy.
pub(crate) fn restore_row(
    tx: &Transaction<'_>,
    row: &SectionRow,
    _ctx: &RestoreCtx<'_>,
) -> Result<RestoreOutcome, IndexError> {
    let data: NoScanProject = serde_json::from_value(row.data.clone())
        .map_err(|e| IndexError::Sidecar(format!("a no_scan_projects row: {e}")))?;
    let mut project = data.project;
    // A submodule edge re-derives from its parent's scan, and a merge is never replayed.
    project.insert("parent_project_id".to_owned(), SidecarValue::Null);
    project.insert("merged_into".to_owned(), SidecarValue::Null);
    insert_row(tx, "project", &project, &[])?;
    for location in &data.locations {
        insert_row(tx, "location", location, &[])?;
    }
    Ok(RestoreOutcome::Applied(1))
}
