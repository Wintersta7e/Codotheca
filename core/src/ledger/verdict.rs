//! §38.8.4's `amnesty_verdict` rows — one act, one payout.
//!
//! §44 owns when an award is written and which calls are a card, the page control or a group
//! approval; this owns the row, written in that act's own transaction.

use rusqlite::Transaction;

use super::marginal::{verdict_due, RowDue};
use super::receipt::Receipt;
use crate::clock::LocalInstant;
use crate::index::IndexError;
use crate::protocol::ProjectId;

/// What a card or page verdict's award did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerdictAward {
    /// This call wrote the project's once-ever award, as this `xp_events` id.
    Paid(i64),
    /// The project's award exists: a later verdict, whatever it says, pays nothing.
    AlreadyPaid,
    /// The project has no subject to key a row on; the verdict itself stands.
    NoSubject,
    /// A restored record may yet match the project and bring its own award back (§48.8.4).
    AwaitingRestore,
}

/// Pay `project`'s once-ever award for a verdict committed on a card or the page control.
///
/// In the verdict's own transaction: key `amnesty_verdict:<subject_key>`, receipt
/// `{what: 'verdict', verdict, when, project_name}`.
///
/// The award is found by project, never by key (§38.2), through the reading the marginal price
/// quotes. Its caller is §44's verdict writer.
///
/// # Errors
/// Returns [`IndexError`] when a read fails or SQLite refuses the insert.
pub fn pay_verdict_award(
    tx: &Transaction<'_>,
    project: ProjectId,
    verdict_slug: &str,
    at: LocalInstant,
) -> Result<VerdictAward, IndexError> {
    let subject_key = match verdict_due(tx, project)? {
        RowDue::Write(subject_key) => subject_key,
        // No gate refuses a verdict, so `Refused` never reaches here.
        RowDue::Written(_) | RowDue::Refused => return Ok(VerdictAward::AlreadyPaid),
        RowDue::Held => return Ok(VerdictAward::AwaitingRestore),
        RowDue::NotComputable => return Ok(VerdictAward::NoSubject),
    };
    let project_name: String =
        tx.query_row("SELECT name FROM project WHERE id = ?1", [project.0], |r| {
            r.get(0)
        })?;
    let inserted = tx.execute(
        "INSERT INTO xp_events
            (ts, tz_offset_min, project_id, subject_key, kind, dedupe_key, track, meta)
         VALUES (?1, ?2, ?3, ?4, 'amnesty_verdict', ?5, 'session', ?6)
         ON CONFLICT(dedupe_key) DO NOTHING",
        rusqlite::params![
            at.unix,
            at.offset_min,
            project.0,
            subject_key,
            format!("amnesty_verdict:{subject_key}"),
            Receipt::new("verdict", at, &project_name)
                .extra("verdict", verdict_slug)
                .to_meta()
        ],
    )?;
    Ok(if inserted > 0 {
        VerdictAward::Paid(tx.last_insert_rowid())
    } else {
        VerdictAward::AlreadyPaid
    })
}

/// Pay ONE award for a pre-pass group approval; answers whether this call wrote it.
///
/// In the approval's own transaction: `project_id` NULL, `subject_key` `library`, key
/// `amnesty_verdict:prepass:<admitted_at>:<group>`, receipt `{what: 'prepass', group, projects,
/// when}`.
///
/// The sitting's start instant and the group name make the key, so a replayed approval writes
/// nothing. The members keep their own once-ever eligibility: a later card or page verdict on one
/// pays it. Its caller is §44's group approval.
///
/// # Errors
/// Returns [`IndexError`] when SQLite refuses the insert.
pub fn pay_prepass_award(
    tx: &Transaction<'_>,
    admitted_at: i64,
    group: &str,
    projects: u32,
    at: LocalInstant,
) -> Result<bool, IndexError> {
    let inserted = tx.execute(
        "INSERT INTO xp_events
            (ts, tz_offset_min, project_id, subject_key, kind, dedupe_key, track, meta)
         VALUES (?1, ?2, NULL, 'library', 'amnesty_verdict', ?3, 'session', ?4)
         ON CONFLICT(dedupe_key) DO NOTHING",
        rusqlite::params![
            at.unix,
            at.offset_min,
            format!("amnesty_verdict:prepass:{admitted_at}:{group}"),
            Receipt::library("prepass", at)
                .extra("group", group)
                .extra("projects", projects)
                .to_meta()
        ],
    )?;
    Ok(inserted > 0)
}
