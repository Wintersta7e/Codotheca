//! §38.7.2: the one path that deletes git rows on a caller's behalf.
//!
//! Git rows are a function of history, so a change to whose history counts deletes them and the
//! history job writes them again. The path ratchets the floor first, from the reading it is about
//! to change, and deletes only where the history can be read again; anywhere else the rows are
//! frozen until a copy returns.

use rusqlite::{Connection, Transaction};

use super::floor::ratchet_from_reading;
use crate::index::IndexError;
use crate::protocol::ProjectId;

/// Which of the three callers asked for a recompute. Every cause takes the same path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecomputeCause {
    /// `identity.confirm` changed which projects are the user's.
    IdentityConfirm,
    /// Two projects merged, and both sides' history is now the survivor's.
    Merge,
    /// A Reference verdict was set or taken back.
    Reference,
}

/// What one recompute did.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RecomputeReport {
    /// Git rows deleted, across every project.
    pub deleted: usize,
    /// Projects whose git rows went and whose history job is queued to write them again.
    pub queued: Vec<ProjectId>,
    /// Projects with no readable history, whose rows were kept as they are.
    pub frozen: Vec<ProjectId>,
}

/// §38.7.1: whether `project`'s history can be read again — some copy is neither removed nor
/// away. An offline copy is not readable; its rows wait for the drive.
///
/// # Errors
/// Returns [`IndexError`] when the read fails.
pub fn history_readable(conn: &Connection, project: ProjectId) -> Result<bool, IndexError> {
    let readable: bool = conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM location
                         WHERE project_id = ?1 AND removed_at IS NULL AND presence = 'present')",
        [project.0],
        |r| r.get(0),
    )?;
    Ok(readable)
}

/// What happens to one project's git rows.
enum Branch {
    /// Its history can be read again: delete the rows and queue the history job.
    Delete,
    /// It cannot: keep the rows as they are.
    Freeze,
}

/// The one decision between deleting and freezing. A captured log of a removed copy is a third
/// branch, recomputing from the capture in this transaction, and it lands here with the capture.
fn branch_for(tx: &Transaction<'_>, project: ProjectId) -> Result<Branch, IndexError> {
    Ok(if history_readable(tx, project)? {
        Branch::Delete
    } else {
        Branch::Freeze
    })
}

/// Delete the git rows of every readable project in `projects` and queue its history job, after
/// ratcheting the floor once from the reading as it stands — covering the frozen projects too.
///
/// The only code that deletes `xp_events` rows on a caller's behalf, in the caller's transaction.
/// `cause` names the caller; the path does not depend on it.
///
/// # Errors
/// Returns [`IndexError`] when a read, the ratchet, a delete or the job update fails.
pub fn recompute_git_rows(
    tx: &Transaction<'_>,
    projects: &[ProjectId],
    _cause: RecomputeCause,
    now: i64,
) -> Result<RecomputeReport, IndexError> {
    ratchet_from_reading(tx, now)?;
    let mut report = RecomputeReport::default();
    for &project in projects {
        match branch_for(tx, project)? {
            Branch::Delete => {
                report.deleted += tx.execute(
                    "DELETE FROM xp_events WHERE project_id = ?1 AND track = 'git'",
                    [project.0],
                )?;
                tx.execute(
                    "UPDATE project_job_state
                        SET state = 'queued', reason = 'user_requested', at = ?2, fail_count = 0
                      WHERE project_id = ?1 AND job = 'j4'",
                    rusqlite::params![project.0, now],
                )?;
                report.queued.push(project);
            }
            Branch::Freeze => report.frozen.push(project),
        }
    }
    Ok(report)
}
