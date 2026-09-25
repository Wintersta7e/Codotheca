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
use super::markers::{
    Marker, J7_BLOB_BYTE_CAP, J7_CHUNK_BLOBS, J7_CHUNK_BYTES, J7_SCANNER_VERSION,
};
use super::presence::{presence_for, PresenceAnswers, PresenceState, PREDICATE_VERSION};
use super::{JobError, JobOutcome};
use crate::git::{GitBackend, JobContext, RepoHandle, TreeEntry};
use crate::index::IndexError;
use crate::protocol::{DebtSource, DebtSweepOutcome, LocationId, ProjectId};

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

/// Where this run starts in the filtered enumeration, or `None` when the stored scan already
/// answers for this head and nothing needs reading.
///
/// A moved head restarts at ordinal 0 against the new head. [p3] **An unchanged head vouches for
/// the tree, not for evidence the store has since withdrawn**: a copy that went away and came
/// back has its marker sweep recorded `unobservable` and its items `unverified`
/// (`DebtStore::mark_root_unobserved`), and skipping here would leave them uncounted until the
/// next commit. That one case rescans from ordinal 0 — every blob already cached, so no blob is
/// read — and every other unchanged head still invokes git zero times.
///
/// **Only at a root that is there to re-observe** (§28.5's rule 4). The copy can leave again, or
/// be uninstalled, before this job runs, and an opened page still queues it against that copy:
/// a rescan there reads nothing, and would clear the completed scan and store `not_read` over the
/// four presence answers it established. The withdrawn sweep waits for the copy's next return.
fn start_ordinal(
    index: &std::sync::Mutex<crate::index::Index>,
    gates: ContentGates,
    project: ProjectId,
    location: LocationId,
    head_oid: &str,
    stored: Option<&ContentScanRow>,
    cursor: Option<&str>,
) -> Result<Option<usize>, JobError> {
    let complete = stored.is_some_and(|row| row.complete_head_oid.as_deref() == Some(head_oid));
    let withdrawn = complete
        && gates.reads_blobs()
        && super::read(index, |conn| {
            let unobserved =
                crate::debt::sweep::stored_outcome(conn, project, DebtSource::TodoMarker)
                    .map_err(debt_to_index)?
                    == Some(DebtSweepOutcome::Unobservable);
            Ok(unobserved
                && crate::debt::sweep::root_is_observable(conn, location).map_err(debt_to_index)?)
        })?;
    if complete && !withdrawn {
        return Ok(None);
    }
    if withdrawn {
        super::write(index, |tx| restart_scan(tx, project))?;
    }
    let moved = stored.is_some_and(|row| row.head_oid != head_oid);
    Ok(Some(if moved || withdrawn {
        0
    } else {
        cursor_ordinal(cursor)
    }))
}

/// Take a completed scan back to its start at the same head, so the next chunks re-observe the
/// whole tree: the completion marker goes, and every blob is pending again until a chunk covers
/// it. The cache is untouched, so a re-covered blob is a lookup and not a read.
fn restart_scan(tx: &Transaction<'_>, project: ProjectId) -> Result<(), IndexError> {
    tx.execute(
        "UPDATE project_content_scan
            SET complete_head_oid = NULL, completed_at = NULL, blobs_pending = blobs_total
          WHERE project_id = ?1",
        [project.0],
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

/// §29.7's three predicates, checked in order.
///
/// Gate 1 decides whether J7 runs at all. Gates 2 and 3 gate **only the blob read**: the
/// enumeration still runs, because it reads names, which §10.1's shipped paragraph already
/// licenses — so a suppressed or ungranted project still answers all four presence predicates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContentGates {
    /// §4.1a's classification. **`None` is *not computed* and is not Reference**, so the gate is
    /// `Some(false)` rather than a falsy check. This is what removes the measured worst case — a
    /// one-commit clone of someone else's 14,000-file project — from the workload.
    pub is_reference: Option<bool>,
    /// §29.8's grant. Off → no blob is read and no `blob_scan` or `blob_finding` row is written.
    pub granted: bool,
    /// §30.5's predicate: not enrolled, or `is_archived = 1`.
    ///
    /// **This is the compute gate and it gates the blob read alone.** It does not gate the
    /// enumeration — J3 already enumerates every non-Reference project unsuppressed, so
    /// suppressing J7's would suppress work that already runs — and it gates nothing bounded.
    ///
    /// `surface_suppressed` gates rendering, ranking and notification and gates nothing here.
    pub compute_suppressed: bool,
}

impl ContentGates {
    /// Whether J7 runs at all. Predicate 1 alone; the other two gate the blob read.
    #[must_use]
    pub fn runs(self) -> bool {
        self.is_reference == Some(false)
    }

    /// Whether the blob read runs. All three, in §29.7's order.
    #[must_use]
    pub fn reads_blobs(self) -> bool {
        self.runs() && self.granted && !self.compute_suppressed
    }
}

/// Read §29.7's three predicates for one project.
///
/// # Errors
/// Fails when SQLite refuses a read.
pub fn gates_for(conn: &Connection, project: ProjectId) -> Result<ContentGates, IndexError> {
    Ok(ContentGates {
        is_reference: super::scheduler::is_reference(conn, project)?,
        granted: crate::surfaces::settings::content_scan_enabled(conn)?,
        compute_suppressed: {
            // Both columns come off the row `gates_for` is already reading for.
            let (acknowledged_at, is_archived) = conn
                .query_row(
                    "SELECT acknowledged_at, is_archived FROM project WHERE id = ?1",
                    [project.0],
                    |r| Ok((r.get::<_, Option<i64>>(0)?, r.get::<_, i64>(1)? != 0)),
                )
                .or_else(|e| match e {
                    rusqlite::Error::QueryReturnedNoRows => Ok((None, false)),
                    other => Err(IndexError::from(other)),
                })?;
            crate::health::enrolment::compute_suppressed(
                crate::health::enrolment::is_enrolled(acknowledged_at),
                is_archived,
            )
        },
    })
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
    /// The machine's offset, from `JobDeps`. §28.4's `debt_day` key is a **local** date and the
    /// item build runs from here, so the run carries it rather than reading a zone.
    pub tz_offset_min: i32,
    /// [p3] §34.2's provenance of the job running this scan, recorded on any `health_delta` row
    /// the item build writes.
    pub detected_in: crate::protocol::HealthDetectedIn,
    /// [p3] Where the item build hands the event for a delta it wrote, **once its transaction has
    /// committed**, for the runner to announce. `None` records the rows and announces nothing.
    pub announce: Option<&'a std::cell::RefCell<Vec<crate::protocol::ProjectHealthDelta>>>,
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
        tz_offset_min: _,
        detected_in: _,
        announce,
    } = run;
    let gates = super::read(index, |conn| gates_for(conn, project))?;
    // Predicate 1 stays in the job as defence-in-depth: `next_jobs_after` skips Reference, and
    // `projects.get` and `projects.requeue` are two entry points that do not go through it.
    if !gates.runs() {
        return Ok(JobOutcome::Done);
    }
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
    let Some(from) = start_ordinal(
        index,
        gates,
        project,
        location,
        &head_oid,
        stored.as_ref(),
        cursor,
    )?
    else {
        return Ok(JobOutcome::Done);
    };

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
        let (pending, delta) = super::write(index, |tx| {
            let pending = commit_chunk(tx, project, &head_oid, &[], 0, now)?;
            let delta = build_debt_items(tx, &run, gates, &filtered)?;
            Ok((pending, delta))
        })?;
        hand_out(announce, delta);
        return Ok(finish(pending, from, total));
    }
    let cached_before = cached_blobs(index, &window)?;
    let mut wanted: Vec<String> = Vec::new();
    for entry in &window {
        if !cached_before.contains(&entry.oid) && !wanted.contains(&entry.oid) {
            wanted.push(entry.oid.clone());
        }
    }
    // Gates 2 and 3. The enumeration above has already run and its answers are already stored;
    // what stops here is the blob read and nothing else.
    if !gates.reads_blobs() {
        return Ok(JobOutcome::Done);
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
    let (pending, delta) = super::write(index, |tx| {
        let pending = commit_chunk(
            tx,
            project,
            &head_oid,
            &batch.reads,
            i64::try_from(consumed).unwrap_or(i64::MAX),
            now,
        )?;
        let delta = build_debt_items(tx, &run, gates, &filtered)?;
        Ok((pending, delta))
    })?;
    hand_out(announce, delta);
    Ok(finish(pending, from + consumed, total))
}

/// §28's item build, in the **same transaction** as the chunk that produced the evidence.
///
/// **It runs here and not from `JobRunner::settle`, and that is forced by the tree.**
/// [`occurrences_for_project`] takes the HEAD enumeration because nothing stores it — §29.6
/// re-runs `ls-tree` per chunk deliberately — and `settle` holds no enumeration and no repository
/// handle. This is the one place the entries exist beside a transaction.
///
/// [p3] **It is also one of §34's three `health_delta` callers**, for the same reason: the marker
/// items open and close here, so the before-snapshot, the item write and the row all share this
/// transaction. It returns the event to announce once the caller's transaction has committed.
fn build_debt_items(
    tx: &Transaction<'_>,
    run: &ScanRun<'_>,
    gates: ContentGates,
    filtered: &[TreeEntry],
) -> Result<Option<crate::protocol::ProjectHealthDelta>, IndexError> {
    let ScanRun {
        project,
        location,
        now,
        tz_offset_min,
        detected_in,
        ..
    } = *run;
    let layers_before = crate::restoration::LayerValues::read(tx, project)?;
    let occurrences = occurrences_for_project(tx, filtered)?;
    let store = crate::debt::store::SqliteDebtStore;
    let effect = crate::debt::markers::build_items(
        tx,
        project,
        Some(location),
        gates,
        &occurrences,
        now,
        &store,
    )
    .map_err(debt_to_index)?;

    // The ledger row and the item deletions commit together: a tree with the items gone and no
    // payout, or a payout with the items still open, is the state the ordering prevents.
    let subject = crate::index::subject::subject_for_project(tx, project)?
        .map(|s| s.to_key())
        .unwrap_or_default();
    crate::debt::xp::pay_debt_day(tx, project, &subject, &effect, now, tz_offset_min)
        .map_err(debt_to_index)?;
    crate::restoration::record_after_write(
        tx,
        project,
        &layers_before,
        &effect.closed,
        detected_in,
        now,
    )
}

/// Hand a committed delta to the runner. Called only after `super::write` has returned `Ok`, so
/// nothing reaches the announcer that a rollback could still remove.
fn hand_out(
    announce: Option<&std::cell::RefCell<Vec<crate::protocol::ProjectHealthDelta>>>,
    delta: Option<crate::protocol::ProjectHealthDelta>,
) {
    if let (Some(out), Some(delta)) = (announce, delta) {
        out.borrow_mut().push(delta);
    }
}

fn debt_to_index(error: crate::debt::DebtError) -> IndexError {
    match error {
        crate::debt::DebtError::Index(inner) => inner,
        // A stored value this build's schema does not declare. `InvalidQuery` is the closest
        // `rusqlite` shape that carries no column of its own; the detail is what a reader needs.
        crate::debt::DebtError::Codec(detail) => {
            IndexError::Sqlite(rusqlite::Error::InvalidParameterName(detail))
        }
    }
}

/// Which of the window's blobs already carry a cache row at this scanner version.
///
/// Extracted so `run_j7` reads as its five numbered steps; it is the read half of step 3.
fn cached_blobs(
    index: &std::sync::Mutex<crate::index::Index>,
    window: &[&TreeEntry],
) -> Result<BTreeSet<String>, JobError> {
    let wanted: Vec<String> = window.iter().map(|e| e.oid.clone()).collect();
    let missing = super::read(index, |conn| {
        missing_blobs(conn, &wanted, J7_SCANNER_VERSION)
    })?;
    let missing: BTreeSet<String> = missing.into_iter().collect();
    Ok(wanted
        .into_iter()
        .filter(|oid| !missing.contains(oid))
        .collect())
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

/// §29.9's outcome, as J7 determines it.
///
/// **Two variants, and deliberately not §28's generated `DebtSweepOutcome`** (R31): that one
/// carries more — `skipped_suppressed` among them — is declared by §28, and is what §28 maps this
/// onto. Two variants rather than a `bool`, because a `bool` needs a convention a reader has to
/// be told about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentSweepOutcome {
    /// Every filtered blob at `head_oid` has a cache row.
    Complete,
    /// A chunk boundary, a closed gate or a moved head. **The evidence set behind it is
    /// incomplete, never empty**, which is what stops §28's closure path being offered an empty
    /// one.
    Partial,
}

/// What §28 needs to write a `debt_sweep` row, without J7 storing one.
///
/// **`basis` is the literal `"head"`, stated and not stored** (§29.1, R129/F3): J7's basis is
/// `head` for every row without exception, so a column would be a per-row copy of a value the
/// table already determines. §28 maps this string onto the generated `ObservationBasis` it owns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentSweepState {
    /// Derived from `complete_head_oid == head_oid`, never from a stored flag.
    pub outcome: ContentSweepOutcome,
    /// Always `"head"`.
    pub basis: &'static str,
    /// The head this reading is of.
    pub head_oid: String,
    /// Filtered entries at `head_oid`. `None` is *not enumerated*, never 0.
    pub blobs_total: Option<i64>,
    /// How many of them this scan has covered.
    pub blobs_read: i64,
    /// How many are left. `None` is *not enumerated*, never 0.
    pub blobs_pending: Option<i64>,
}

/// One occurrence, as §28 consumes it.
///
/// **`ordinal_in_blob` is per blob and §28.1's `ordinal` is per project, and the two must never
/// be conflated**: a blob's ordinal is stable in every project that holds that blob, a project's
/// is not, and an identity built from the wrong one moves when an unrelated file is added.
/// **The per-project ordinal is assigned by §28 at item-build time and not here.**
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentOccurrence {
    /// The content address the finding is cached under.
    pub blob_oid: String,
    /// Raw path bytes, from the enumeration. A blob reachable at two paths contributes twice.
    pub path_bytes: Vec<u8>,
    /// 1-based.
    pub line: u32,
    /// 1-based, in bytes.
    pub column: u32,
    /// One of `TODO`, `FIXME`, `HACK`.
    pub marker: Marker,
    /// §28.1's hash of the capped text.
    pub salient_sha256: String,
    /// §28.1's normalised, capped text.
    pub salient_text_capped: String,
}

/// §29.9's sweep hand-over, or `None` when J7 has never observed the project.
///
/// **`None` is not an outcome.** It is *never observed*, which §31 renders as `unknown` and §28
/// must not read as *everything closed*.
///
/// # Errors
/// Fails when SQLite refuses the read.
pub fn content_sweep_state(
    conn: &Connection,
    project: ProjectId,
) -> Result<Option<ContentSweepState>, IndexError> {
    let Some(row) = content_scan_row(conn, project)? else {
        return Ok(None);
    };
    let outcome = if row.complete_head_oid.as_deref() == Some(row.head_oid.as_str()) {
        ContentSweepOutcome::Complete
    } else {
        ContentSweepOutcome::Partial
    };
    let blobs_read = match (row.blobs_total, row.blobs_pending) {
        (Some(total), Some(pending)) => total.saturating_sub(pending).max(0),
        _ => 0,
    };
    Ok(Some(ContentSweepState {
        outcome,
        basis: "head",
        head_oid: row.head_oid,
        blobs_total: row.blobs_total,
        blobs_read,
        blobs_pending: row.blobs_pending,
    }))
}

/// §29.9's occurrence hand-over: every cached finding for `entries`, ordered on
/// `(path_bytes, line, column)` over the whole enumeration.
///
/// **The caller supplies the enumeration, because nothing stores it.** §29.6 re-runs `ls-tree` at
/// the start of every chunk deliberately — it is deterministic under a pinned head and costs no
/// table and no cursor codec — so a second, stored copy of the tree is exactly what §29.5 avoided.
/// `entries` is what [`filtered_entries`] returned for this project's `head_oid`.
///
/// **This is the half only J7 can supply**, because only J7 holds the enumeration the ordering is
/// over; §28 derives the per-project ordinal from it and declares no second shape for the tuple.
///
/// # Errors
/// Fails when SQLite refuses a read.
pub fn occurrences_for_project(
    conn: &Connection,
    entries: &[TreeEntry],
) -> Result<Vec<ContentOccurrence>, IndexError> {
    let mut out = Vec::new();
    for entry in entries {
        for found in super::content_scan::findings_for_blob(conn, &entry.oid, J7_SCANNER_VERSION)? {
            out.push(ContentOccurrence {
                blob_oid: entry.oid.clone(),
                path_bytes: entry.path.clone(),
                line: found.line,
                column: found.column,
                marker: found.marker,
                salient_sha256: found.salient_sha256,
                salient_text_capped: found.salient_text_capped,
            });
        }
    }
    out.sort_by(|a, b| {
        a.path_bytes
            .cmp(&b.path_bytes)
            .then(a.line.cmp(&b.line))
            .then(a.column.cmp(&b.column))
    });
    Ok(out)
}
