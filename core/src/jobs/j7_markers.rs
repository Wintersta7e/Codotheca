//! J7 — the content scan (§29). The only thing in this product that reads the contents of a
//! user's source files, and it does so only behind a grant the user turns on.
//!
//! **Incremental by construction.** The cache key is the blob id, so invalidation cannot be
//! subtly wrong: a commit touching three files leaves three objects to read, a `fetch` leaves
//! none, and two repositories holding the same vendored file share one row.
//!
//! **A project whose scan has not completed publishes no count** — not a partial one, not a zero.
//! `complete_head_oid` is set only in the transaction that takes `blobs_pending` to 0 at
//! `head_oid`, so there is no flag to remember to set: the absence of a completed head *is* the
//! absence of the number.

use std::collections::BTreeSet;

use rusqlite::{Connection, Transaction};

use super::classify::language_of_path;
use super::content_scan::{missing_blobs, record_read};
use super::markers::{J7_BLOB_BYTE_CAP, J7_CHUNK_BLOBS, J7_CHUNK_BYTES, J7_SCANNER_VERSION};
use super::presence::{presence_for, PresenceAnswers, PresenceState, PREDICATE_VERSION};
use super::{JobError, JobOutcome};
use crate::git::{GitBackend, JobContext, RepoHandle, TreeEntry};
use crate::index::IndexError;
use crate::protocol::{LocationId, ProjectId};

/// §29.2's rules 1–3, in order, over one enumeration.
///
/// Rule 3 is the `programming` flag, **not a second list** — a second list is one value stated
/// twice. Every `markup(...)` entry is therefore excluded: a checklist in a README is not debt,
/// and a lockfile is §32's input, read by name.
///
/// The order is `path_bytes`, so the ordinal a cursor names is stable under a pinned head.
#[must_use]
pub fn filtered_entries(entries: &[TreeEntry]) -> Vec<TreeEntry> {
    let mut kept: Vec<TreeEntry> = entries
        .iter()
        .filter(|e| e.kind == "blob" && (e.mode == "100644" || e.mode == "100755"))
        .filter(|e| {
            let path = String::from_utf8_lossy(&e.path);
            language_of_path(&path).is_some_and(|lang| lang.programming)
        })
        .cloned()
        .collect();
    kept.sort_by(|a, b| a.path.cmp(&b.path));
    kept
}

/// One project's stored scan row, or `None` when J7 has never observed it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentScanRow {
    /// The head the row was written against.
    pub head_oid: String,
    /// The head the last **complete** scan covered. NULL until one completes.
    pub complete_head_oid: Option<String>,
    /// Filtered entries at `head_oid`. `None` is *not enumerated*, never 0.
    pub blobs_total: Option<i64>,
    /// Filtered entries not yet consumed. `None` is *not enumerated*, never 0.
    pub blobs_pending: Option<i64>,
}

/// Read one project's scan row.
///
/// # Errors
/// Fails when SQLite refuses the read.
pub fn content_scan_row(
    conn: &Connection,
    project: ProjectId,
) -> Result<Option<ContentScanRow>, IndexError> {
    conn.query_row(
        "SELECT head_oid, complete_head_oid, blobs_total, blobs_pending
           FROM project_content_scan WHERE project_id = ?1",
        [project.0],
        |row| {
            Ok(ContentScanRow {
                head_oid: row.get(0)?,
                complete_head_oid: row.get(1)?,
                blobs_total: row.get(2)?,
                blobs_pending: row.get(3)?,
            })
        },
    )
    .map(Some)
    .or_else(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => Ok(None),
        other => Err(IndexError::from(other)),
    })
}

/// `location.head_oid` for one location. `None` is an **unborn HEAD** — a fresh `git init`, a
/// branch with no commits, an empty repository — and J7 has no basis to form.
///
/// # Errors
/// Fails when SQLite refuses the read.
pub fn head_oid_of(conn: &Connection, location: LocationId) -> Result<Option<String>, IndexError> {
    conn.query_row(
        "SELECT head_oid FROM location WHERE id = ?1",
        [location.0],
        |row| row.get::<_, Option<String>>(0),
    )
    .or_else(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => Ok(None),
        other => Err(other),
    })
    .map_err(IndexError::from)
}

/// Write the enumeration pass's answers: the four presence tri-states and `blobs_total`.
///
/// **The presence answers land before any blob is read**, so a project whose content scan never
/// completes still answers all four. A head that moved clears the marker fields in the same
/// statement — a scan straddling two heads is a reading of neither.
///
/// # Errors
/// Fails when SQLite refuses the write.
pub fn write_enumeration(
    tx: &Transaction<'_>,
    project: ProjectId,
    head_oid: &str,
    answers: PresenceAnswers,
    blobs_total: Option<i64>,
    now: i64,
) -> Result<(), IndexError> {
    tx.execute(
        "INSERT INTO project_content_scan
           (project_id, head_oid, complete_head_oid, blobs_total, blobs_pending,
            predicate_version, has_readme, has_license, has_tests, has_ci,
            presence_observed_at, enumerated_at, completed_at)
         VALUES (?1, ?2, NULL, ?3, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9, NULL)
         ON CONFLICT(project_id) DO UPDATE SET
            head_oid = excluded.head_oid,
            complete_head_oid = CASE
                WHEN project_content_scan.head_oid = excluded.head_oid
                THEN project_content_scan.complete_head_oid ELSE NULL END,
            completed_at = CASE
                WHEN project_content_scan.head_oid = excluded.head_oid
                THEN project_content_scan.completed_at ELSE NULL END,
            blobs_total = excluded.blobs_total,
            blobs_pending = CASE
                WHEN project_content_scan.head_oid = excluded.head_oid
                THEN project_content_scan.blobs_pending ELSE excluded.blobs_pending END,
            predicate_version = excluded.predicate_version,
            has_readme = excluded.has_readme,
            has_license = excluded.has_license,
            has_tests = excluded.has_tests,
            has_ci = excluded.has_ci,
            presence_observed_at = excluded.presence_observed_at,
            enumerated_at = excluded.enumerated_at",
        rusqlite::params![
            project.0,
            head_oid,
            blobs_total,
            PREDICATE_VERSION,
            answers.readme.slug(),
            answers.license.slug(),
            answers.tests.slug(),
            answers.ci.slug(),
            now,
        ],
    )?;
    Ok(())
}

/// Record one chunk's worth of reads and move `blobs_pending` down by what it covered.
///
/// **`complete_head_oid` is set only here, in the same transaction that takes `blobs_pending` to
/// 0** — the per-project marker aggregate exists if and only if the two heads match.
///
/// # Errors
/// Fails when SQLite refuses a write.
fn commit_chunk(
    tx: &Transaction<'_>,
    project: ProjectId,
    head_oid: &str,
    reads: &[crate::git::BlobRead],
    consumed: i64,
    now: i64,
) -> Result<i64, IndexError> {
    for read in reads {
        record_read(tx, read, J7_SCANNER_VERSION, now)?;
    }
    tx.execute(
        "UPDATE project_content_scan
            SET blobs_pending = max(0, coalesce(blobs_pending, 0) - ?2)
          WHERE project_id = ?1 AND head_oid = ?3",
        rusqlite::params![project.0, consumed, head_oid],
    )?;
    let pending: i64 = tx.query_row(
        "SELECT coalesce(blobs_pending, 0) FROM project_content_scan WHERE project_id = ?1",
        [project.0],
        |row| row.get(0),
    )?;
    if pending == 0 {
        tx.execute(
            "UPDATE project_content_scan
                SET complete_head_oid = ?2, completed_at = ?3
              WHERE project_id = ?1 AND head_oid = ?2",
            rusqlite::params![project.0, head_oid, now],
        )?;
    }
    Ok(pending)
}

/// Where the next chunk resumes: an **ordinal into the filtered enumeration**, decimal.
///
/// Not an ordinal into the miss list, which shrinks as rows land; and **not a path**, because
/// `project_job_state.cursor` is `TEXT` and a Linux path is arbitrary bytes.
#[must_use]
pub fn cursor_ordinal(cursor: Option<&str>) -> usize {
    cursor.and_then(|c| c.parse::<usize>().ok()).unwrap_or(0)
}

/// What one J7 run is about: the project, the copy on disk it reads, and where it resumes.
#[derive(Debug, Clone, Copy)]
pub struct ScanRun<'a> {
    /// The project the row is filed against.
    pub project: ProjectId,
    /// §5.1's primary location — the one the scheduler hands a `PrimaryOnly` job.
    pub location: LocationId,
    /// `project_job_state.cursor`, an ordinal into the filtered enumeration.
    pub cursor: Option<&'a str>,
    /// The caller's clock, like every other writer in this tree.
    pub now: i64,
}

/// Run one chunk of J7 against one project.
///
/// # Errors
/// Fails when git refuses a read or SQLite refuses a write.
pub fn run_j7(
    index: &std::sync::Mutex<crate::index::Index>,
    git: &dyn GitBackend,
    repo: &RepoHandle,
    ctx: &JobContext<'_>,
    run: ScanRun<'_>,
) -> Result<JobOutcome, JobError> {
    let ScanRun {
        project,
        location,
        cursor,
        now,
    } = run;
    // 1. The head comparison, before any git invocation. An unborn HEAD writes no row at all —
    //    the row's absence is *J7 has never observed this project*, and no fourth tri-state value
    //    is invented for it.
    let (head_oid, stored) = super::read(index, |conn| {
        Ok((
            head_oid_of(conn, location)?,
            content_scan_row(conn, project)?,
        ))
    })?;
    let Some(head_oid) = head_oid else {
        return Ok(JobOutcome::Done);
    };
    if stored
        .as_ref()
        .is_some_and(|row| row.complete_head_oid.as_deref() == Some(head_oid.as_str()))
    {
        return Ok(JobOutcome::Done);
    }
    // A moved head restarts at ordinal 0 against the new head.
    let moved = stored.as_ref().is_some_and(|row| row.head_oid != head_oid);
    let from = if moved { 0 } else { cursor_ordinal(cursor) };

    // 2. The enumeration. A failure stores `not_read` for all four rather than `absent`: a
    //    timeout looks exactly like a missing file.
    let entries = match git.head_tree(repo, ctx) {
        Ok(entries) => entries,
        Err(err) => {
            super::write(index, |tx| {
                write_enumeration(
                    tx,
                    project,
                    &head_oid,
                    PresenceAnswers::not_read(),
                    None,
                    now,
                )
            })?;
            return Err(JobError::from(err));
        }
    };
    let answers = presence_for(&entries);
    let filtered = filtered_entries(&entries);
    let total = i64::try_from(filtered.len()).unwrap_or(i64::MAX);
    super::write(index, |tx| {
        write_enumeration(tx, project, &head_oid, answers, Some(total), now)
    })?;

    // 3. One chunk of misses. The window is bounded by count here and by bytes inside the read,
    //    whichever comes first (§29.6).
    let window: Vec<&TreeEntry> = filtered.iter().skip(from).take(J7_CHUNK_BLOBS).collect();
    if window.is_empty() {
        let pending = super::write(index, |tx| {
            commit_chunk(tx, project, &head_oid, &[], 0, now)
        })?;
        return Ok(finish(pending, from, total));
    }
    let cached_before: BTreeSet<String> = {
        let wanted: Vec<String> = window.iter().map(|e| e.oid.clone()).collect();
        let missing = super::read(index, |conn| {
            missing_blobs(conn, &wanted, J7_SCANNER_VERSION)
        })?;
        let missing: BTreeSet<String> = missing.into_iter().collect();
        wanted
            .into_iter()
            .filter(|oid| !missing.contains(oid))
            .collect()
    };
    let mut wanted: Vec<String> = Vec::new();
    for entry in &window {
        if !cached_before.contains(&entry.oid) && !wanted.contains(&entry.oid) {
            wanted.push(entry.oid.clone());
        }
    }
    let batch = git.read_blobs(repo, &wanted, J7_BLOB_BYTE_CAP, J7_CHUNK_BYTES, ctx)?;

    // 4. How far the cursor may advance: over every window entry whose blob was already cached or
    //    was answered by this batch, stopping at the first one the byte budget cut off.
    let answered: BTreeSet<&String> = wanted.iter().take(batch.covered).collect();
    let mut consumed = 0_usize;
    for entry in &window {
        if cached_before.contains(&entry.oid) || answered.contains(&entry.oid) {
            consumed += 1;
        } else {
            break;
        }
    }
    let pending = super::write(index, |tx| {
        commit_chunk(
            tx,
            project,
            &head_oid,
            &batch.reads,
            i64::try_from(consumed).unwrap_or(i64::MAX),
            now,
        )
    })?;
    Ok(finish(pending, from + consumed, total))
}

/// `Done` once nothing is pending, `Partial` otherwise. **`total` is never a zero** — it is the
/// enumeration's own count, and the enumeration has completed by the time this is reached.
fn finish(pending: i64, done: usize, total: i64) -> JobOutcome {
    if pending == 0 {
        JobOutcome::Done
    } else {
        JobOutcome::Partial {
            cursor: done.to_string(),
            done: i64::try_from(done).unwrap_or(i64::MAX),
            total: Some(total),
        }
    }
}

/// The four presence answers for one project, or `None` when J7 has never observed it.
///
/// # Errors
/// Fails when SQLite refuses the read.
pub fn presence_for_project(
    conn: &Connection,
    project: ProjectId,
) -> Result<Option<PresenceAnswers>, IndexError> {
    conn.query_row(
        "SELECT has_readme, has_license, has_tests, has_ci
           FROM project_content_scan WHERE project_id = ?1",
        [project.0],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        },
    )
    .map(Some)
    .or_else(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => Ok(None),
        other => Err(IndexError::from(other)),
    })
    .map(|found| {
        found.map(|(readme, license, tests, ci)| PresenceAnswers {
            // A spelling this build cannot name was written by a newer one. `NotRead` is the
            // honest reading of it: unknown, and never a false.
            readme: PresenceState::from_slug(&readme).unwrap_or(PresenceState::NotRead),
            license: PresenceState::from_slug(&license).unwrap_or(PresenceState::NotRead),
            tests: PresenceState::from_slug(&tests).unwrap_or(PresenceState::NotRead),
            ci: PresenceState::from_slug(&ci).unwrap_or(PresenceState::NotRead),
        })
    })
}
