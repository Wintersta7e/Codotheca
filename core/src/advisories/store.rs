//! Reads and writes for §32's nine tables.

use rusqlite::{Connection, Transaction};

use crate::advisories::lockfiles::{LockfileHit, LockfileWalk};
use crate::advisories::{enum_text, AdvisoryError};
use crate::index::IndexError;
use crate::protocol::{DependencyReadState, Ecosystem, ProjectId};
use crate::provider::PackageVersion;

/// Drop everything one project's last lockfile read wrote.
///
/// A re-read **replaces**: a lockfile that has been deleted must not leave its triples behind,
/// which would be a dependency this project no longer resolves, dated as if it did.
///
/// # Errors
/// Fails when SQLite refuses the delete.
pub fn clear_project_read(tx: &Transaction<'_>, project: ProjectId) -> Result<(), AdvisoryError> {
    for table in [
        "project_dependency",
        "project_lockfile",
        "project_dependency_scan",
    ] {
        tx.execute(
            &format!("DELETE FROM {table} WHERE project_id = ?1"),
            [project.0],
        )
        .map_err(IndexError::from)?;
    }
    Ok(())
}

/// One `project_lockfile` row: **the file, and whether it could be read.**
///
/// # Errors
/// Fails when SQLite refuses the write.
pub fn write_lockfile_row(
    tx: &Transaction<'_>,
    project: ProjectId,
    hit: &LockfileHit,
    state: DependencyReadState,
    now: i64,
) -> Result<(), AdvisoryError> {
    let ecosystem = enum_text(&hit.ecosystem)?;
    let read_state = enum_text(&state)?;
    tx.execute(
        "INSERT INTO project_lockfile
           (project_id, source_path, ecosystem, read_state, size_bytes, observed_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(project_id, source_path) DO UPDATE SET
           ecosystem = excluded.ecosystem, read_state = excluded.read_state,
           size_bytes = excluded.size_bytes, observed_at = excluded.observed_at",
        rusqlite::params![
            project.0,
            hit.source_path,
            ecosystem,
            read_state,
            i64::try_from(hit.size_bytes).unwrap_or(i64::MAX),
            now
        ],
    )
    .map_err(IndexError::from)?;
    Ok(())
}

/// The record that **the walk ran** — the row whose *absence* is *the scan has not run*.
///
/// §32.8's first verdict row is lit only because *ran and found no lockfile* is distinguishable
/// from *has not run*; collapsing them makes every unscanned project claim to have no dependencies.
///
/// # Errors
/// Fails when SQLite refuses the write.
pub fn write_scan_row(
    tx: &Transaction<'_>,
    project: ProjectId,
    walk: &LockfileWalk,
    now: i64,
) -> Result<(), AdvisoryError> {
    tx.execute(
        "INSERT INTO project_dependency_scan
           (project_id, observed_at, files_matched, dirs_entered, complete)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(project_id) DO UPDATE SET
           observed_at = excluded.observed_at, files_matched = excluded.files_matched,
           dirs_entered = excluded.dirs_entered, complete = excluded.complete",
        rusqlite::params![
            project.0,
            now,
            i64::try_from(walk.files.len()).unwrap_or(i64::MAX),
            i64::try_from(walk.dirs_entered).unwrap_or(i64::MAX),
            i64::from(walk.complete)
        ],
    )
    .map_err(IndexError::from)?;
    Ok(())
}

/// Write one file's triples. Returns the **number of rows written**, which is not the length of
/// `pairs`: one lockfile routinely resolves a package at a version another file already wrote.
///
/// # Errors
/// Fails when SQLite refuses the write.
pub fn write_triples(
    tx: &Transaction<'_>,
    project: ProjectId,
    source_path: &str,
    eco: Ecosystem,
    pairs: &[PackageVersion],
    now: i64,
) -> Result<usize, AdvisoryError> {
    let ecosystem = enum_text(&eco)?;
    let mut written = 0usize;
    for item in pairs {
        // `DO NOTHING`, not `DO UPDATE`: the first file to resolve a triple is the one whose path
        // is recorded, and `source_path` is diagnostic — rewriting it on every duplicate would
        // make the column report whichever file the walk happened to reach last.
        written += tx
            .execute(
                "INSERT INTO project_dependency
                   (project_id, ecosystem, package_name, version, source_path, observed_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(project_id, ecosystem, package_name, version) DO NOTHING",
                rusqlite::params![
                    project.0,
                    ecosystem,
                    item.name,
                    item.version,
                    source_path,
                    now
                ],
            )
            .map_err(IndexError::from)?;
    }
    Ok(written)
}

/// The `x-ratelimit-resource` the sweep's own last **settled** response named, if one has.
///
/// **This is what the pre-issue budget read is keyed by**, and the reason is §32.3's second
/// defect. `DEFAULT_RESOURCE` is a process-wide constant read for every task before issuing. It is
/// right for the six authenticated REST reads phase 2 ships, but for this task it is an assumption:
/// nothing in this tree recorded which pool the advisories endpoint answers from.
///
/// If the endpoint names a resource other than `core`, [`crate::sync::budget::mirror`] writes
/// `(NULL, '<that name>')` while the pre-issue read looks up `(NULL, 'core')` — a pool the sweep
/// never writes. `may_spend` then answers `Unknown` for ever, and **`Unknown` spends**, so every
/// request issues with no brake at all until the source refuses. For an on-demand task that costs
/// one request and self-corrects; for a scheduled sweep it is the behaviour that gets the IP
/// rate-limited for every other unauthenticated call the app makes.
///
/// `None` until one response has named one, where the verdict is `Unknown`, which spends — one
/// request, self-correcting. **A response carrying no `x-ratelimit-resource` at all is not
/// mirrored and is not given a synthetic key here either**: with no resource header there is no
/// pool and no brake, and that is a property of the endpoint rather than a bug to work around.
///
/// # Errors
/// Fails when SQLite cannot be read.
pub fn last_settled_resource(conn: &Connection) -> Result<Option<String>, IndexError> {
    let found = conn
        .query_row(
            "SELECT resource FROM advisory_sweep
              WHERE settled_at IS NOT NULL AND resource IS NOT NULL
              ORDER BY settled_at DESC, id DESC
              LIMIT 1",
            [],
            |row| row.get::<_, String>(0),
        )
        .ok();
    Ok(found)
}
