//! `project_job_state` — durable per-project-per-job scheduling state (§4.1, §1.9).

use rusqlite::{Connection, Transaction};

use super::{JobKind, JobOutcome, JobState};
use crate::index::IndexError;
use crate::protocol::ProjectId;

/// §4.1: J1 and J2 vary with lock contention and page cache, so retrying is right. Three
/// attempts, then `deferred_slow` — which is durable, not permanent, and [`reset_for`] clears it.
pub const MAX_TRANSIENT_FAILS: u32 = 3;

/// §3.5's bounded backoff: doubling from one second, capped at a minute.
#[must_use]
pub fn backoff_secs(fail_count: u32) -> i64 {
    let shifted = 1_i64.checked_shl(fail_count.min(30)).unwrap_or(60);
    shifted.min(60)
}

/// Why a `deferred_slow` or `failed` job becomes runnable again.
///
/// `deferred_slow` is never permanent, and this enum is the closed list of things that make a
/// previously-hopeless job worth another attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResetCause {
    /// §6: the ref-state basis moved, so every cacheable answer is stale anyway.
    FingerprintChanged,
    /// The store the location sits on came back.
    StoreReturned,
    /// A newer git may succeed where the old one refused.
    GitUpgraded,
    /// A newer build may read what the old one could not.
    AppUpgraded,
    /// `projects.requeue` (§2.4), and plan 08's merge, which requeues the survivor.
    UserRequested,
}

impl ResetCause {
    /// Written into `reason`, so a later reader can see why the row was revived.
    #[must_use]
    pub fn slug(self) -> &'static str {
        match self {
            ResetCause::FingerprintChanged => "fingerprint_changed",
            ResetCause::StoreReturned => "store_returned",
            ResetCause::GitUpgraded => "git_upgraded",
            ResetCause::AppUpgraded => "app_upgraded",
            ResetCause::UserRequested => "user_requested",
        }
    }
}

/// Returns the row to persist and, when the job should run again, the epoch second it becomes
/// runnable. A chunk boundary is runnable immediately; a transient failure waits out its backoff.
#[must_use]
pub fn apply_outcome(
    row: &JobStateRow,
    outcome: &JobOutcome,
    now: i64,
) -> (JobStateRow, Option<i64>) {
    let mut next = row.clone();
    next.at = now;
    match outcome {
        JobOutcome::Done => {
            next.state = JobState::Done;
            next.fail_count = 0;
            next.reason = None;
            next.cursor = None;
            (next, None)
        }
        JobOutcome::Degraded => {
            next.state = JobState::Done;
            next.fail_count = 0;
            // untracked_count stays NULL on the location row; this is why it is NULL.
            next.reason = Some("degraded_tracked_only".to_owned());
            next.cursor = None;
            (next, None)
        }
        JobOutcome::Partial {
            cursor,
            done,
            total,
        } => {
            next.state = JobState::Queued;
            next.cursor = Some(cursor.clone());
            next.progress_done = Some(*done);
            next.progress_total = *total;
            // A chunked job never accumulates a fail count: it did not fail, it yielded.
            (next, Some(now))
        }
        JobOutcome::TransientFail { reason } => {
            next.fail_count = next.fail_count.saturating_add(1);
            next.reason = Some(reason.clone());
            if next.fail_count >= MAX_TRANSIENT_FAILS {
                next.state = JobState::DeferredSlow;
                (next, None)
            } else {
                next.state = JobState::Queued;
                let when = now.saturating_add(backoff_secs(next.fail_count));
                (next, Some(when))
            }
        }
        JobOutcome::HardFail { error_kind, detail } => {
            next.state = JobState::Failed;
            next.fail_count = next.fail_count.saturating_add(1);
            next.reason = Some(format!("{error_kind}: {detail}"));
            (next, None)
        }
    }
}

/// §4.1: `deferred_slow` resets on a fingerprint change, a store return, or a git/app upgrade.
/// It is never permanent. `projects.requeue` (§2.4) arrives here as [`ResetCause::UserRequested`].
///
/// Takes the caller's transaction so a merge and its requeue commit together — plan 08 Task 17
/// is the reason this signature is what it is.
pub fn reset_for(
    tx: &Transaction<'_>,
    project: ProjectId,
    cause: ResetCause,
    now: i64,
) -> Result<usize, IndexError> {
    let n = tx.execute(
        "UPDATE project_job_state
            SET state = 'queued', fail_count = 0, reason = ?3, at = ?2
          WHERE project_id = ?1 AND state IN ('deferred_slow', 'failed')",
        rusqlite::params![project.0, now, cause.slug()],
    )?;
    Ok(n)
}

/// What §8.2's coverage caption may say about one project.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JobCoverage {
    /// §8.2's `unchecked` is the count of projects for which this is false.
    pub has_j1_result: bool,
    /// §8.2's `indexedCount` counts projects for which this is true — never the section count.
    pub inventory_done: bool,
}

/// Read one project's coverage.
pub fn coverage_for(conn: &Connection, project: ProjectId) -> Result<JobCoverage, IndexError> {
    let rows = load(conn, project)?;
    let done = |k: JobKind| rows.iter().any(|r| r.job == k && r.state == JobState::Done);
    Ok(JobCoverage {
        has_j1_result: done(JobKind::J1Refstate),
        inventory_done: done(JobKind::J3Inventory),
    })
}

/// One row of `project_job_state`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobStateRow {
    /// Which of the six.
    pub job: JobKind,
    /// Where it stands.
    pub state: JobState,
    /// Consecutive transient failures.
    pub fail_count: u32,
    /// Diagnostic, never shown raw (§2.4).
    pub reason: Option<String>,
    /// When the row was last written, epoch seconds.
    pub at: i64,
    /// §4.1's persisted chunk cursor for J3 and J4.
    pub cursor: Option<String>,
    /// Units finished. `None` is *not computed*, never a zero.
    pub progress_done: Option<i64>,
    /// Units in total. `None` is *not computed*, never a zero.
    pub progress_total: Option<i64>,
}

impl JobStateRow {
    /// A row with no history behind it.
    #[must_use]
    pub fn fresh(job: JobKind, state: JobState, at: i64) -> JobStateRow {
        JobStateRow {
            job,
            state,
            fail_count: 0,
            reason: None,
            at,
            cursor: None,
            progress_done: None,
            progress_total: None,
        }
    }
}

/// Every job row for one project.
pub fn load(conn: &Connection, project: ProjectId) -> Result<Vec<JobStateRow>, IndexError> {
    let mut stmt = conn.prepare(
        "SELECT job, state, fail_count, reason, at, cursor, progress_done, progress_total
             FROM project_job_state WHERE project_id = ?1
             ORDER BY job",
    )?;
    let rows = stmt.query_map([project.0], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, i64>(2)?,
            r.get::<_, Option<String>>(3)?,
            r.get::<_, i64>(4)?,
            r.get::<_, Option<String>>(5)?,
            r.get::<_, Option<i64>>(6)?,
            r.get::<_, Option<i64>>(7)?,
        ))
    })?;

    let mut out = Vec::new();
    for row in rows {
        let (job, state, fail_count, reason, at, cursor, done, total) = row?;
        let (Some(job), Some(state)) = (JobKind::from_slug(&job), JobState::from_slug(&state))
        else {
            // An unknown slug is a row from a newer schema, or `j0`/`j5`, which this scheduler
            // does not queue. Skip it rather than guess.
            continue;
        };
        out.push(JobStateRow {
            job,
            state,
            fail_count: u32::try_from(fail_count).unwrap_or(u32::MAX),
            reason,
            at,
            cursor,
            progress_done: done,
            progress_total: total,
        });
    }
    Ok(out)
}

/// Insert or replace one job row.
///
/// Takes the caller's transaction rather than opening its own, so a merge and its requeue land
/// together — plan 08's writers have the same shape and for the same reason.
pub fn put(tx: &Transaction<'_>, project: ProjectId, row: &JobStateRow) -> Result<(), IndexError> {
    tx.execute(
        "INSERT INTO project_job_state
            (project_id, job, state, fail_count, reason, at, cursor, progress_done, progress_total)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT(project_id, job) DO UPDATE SET
            state = excluded.state, fail_count = excluded.fail_count, reason = excluded.reason,
            at = excluded.at, cursor = excluded.cursor,
            progress_done = excluded.progress_done, progress_total = excluded.progress_total",
        rusqlite::params![
            project.0,
            row.job.slug(),
            row.state.slug(),
            i64::from(row.fail_count),
            row.reason,
            row.at,
            row.cursor,
            row.progress_done,
            row.progress_total,
        ],
    )?;
    Ok(())
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
mod tests {
    use super::*;
    use crate::jobs::JobOutcome;

    #[test]
    fn transient_failure_backs_off_then_defers() {
        let mut row = JobStateRow::fresh(JobKind::J2Status, JobState::Queued, 100);
        let fail = || JobOutcome::TransientFail {
            reason: "lock".into(),
        };

        let (r1, when) = apply_outcome(&row, &fail(), 100);
        assert_eq!(r1.fail_count, 1);
        assert_eq!(r1.state, JobState::Queued);
        assert_eq!(when, Some(100 + backoff_secs(1)));
        row = r1;

        let (r2, _) = apply_outcome(&row, &fail(), 200);
        row = r2;
        let (r3, when) = apply_outcome(&row, &fail(), 300);
        assert_eq!(r3.fail_count, MAX_TRANSIENT_FAILS);
        assert_eq!(r3.state, JobState::DeferredSlow);
        // deferred_slow is not a retry schedule. It waits for a reset.
        assert_eq!(when, None);
    }

    #[test]
    fn a_chunked_job_never_counts_a_failure_and_always_requeues() {
        // §4.1: J3 and J4 scale deterministically. Retrying-then-stranding means the biggest
        // repositories are never inventoried and the era headers omit them silently.
        let row = JobStateRow::fresh(JobKind::J3Inventory, JobState::Running, 10);
        let outcome = JobOutcome::Partial {
            cursor: "src/a.rs".into(),
            done: 400,
            total: Some(9000),
        };
        let (next, when) = apply_outcome(&row, &outcome, 10);
        assert_eq!(next.fail_count, 0);
        assert_eq!(next.state, JobState::Queued);
        assert_eq!(next.cursor.as_deref(), Some("src/a.rs"));
        assert_eq!(next.progress_done, Some(400));
        assert_eq!(next.progress_total, Some(9000));
        assert_eq!(
            when,
            Some(10),
            "a chunk boundary is immediately runnable again"
        );
    }

    #[test]
    fn a_completed_chunked_job_clears_its_cursor() {
        let mut row = JobStateRow::fresh(JobKind::J4History, JobState::Running, 5);
        row.cursor = Some("12000".into());
        row.fail_count = 2;
        let (next, when) = apply_outcome(&row, &JobOutcome::Done, 6);
        assert_eq!(next.state, JobState::Done);
        assert_eq!(next.cursor, None);
        assert_eq!(next.fail_count, 0);
        assert_eq!(when, None);
    }

    #[test]
    fn degraded_is_a_completed_reading_not_a_failure() {
        let row = JobStateRow::fresh(JobKind::J2Status, JobState::Running, 7);
        let (next, when) = apply_outcome(&row, &JobOutcome::Degraded, 7);
        assert_eq!(next.state, JobState::Done);
        assert_eq!(next.reason.as_deref(), Some("degraded_tracked_only"));
        assert_eq!(when, None);
    }

    #[test]
    fn backoff_is_bounded_exponential() {
        assert_eq!(backoff_secs(0), 1);
        assert_eq!(backoff_secs(1), 2);
        assert_eq!(backoff_secs(2), 4);
        assert_eq!(backoff_secs(9), 60, "capped, per §3.5's bounded backoff");
    }
}
