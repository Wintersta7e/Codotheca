//! `badge_award`'s one `INSERT`, its latched set, and its trip through the sidecar (§40.7,
//! §48.8.5): a global section restored as a latch.
//!
//! The export carries every column but `project_id`, because ids do not survive a rebuild;
//! `subject_key` carries the project instead. A restore inserts a missing kind whole and never
//! touches a held one — `badge_award_no_reinsert` ignores it — and sets `project_id` only when
//! exactly one live project holds the subject now.

use rusqlite::{Connection, Transaction};
use serde::{Deserialize, Serialize};

use crate::index::pending::resolve_subject_unique;
use crate::index::sidecar::{RestoreCtx, RestoreOutcome, SectionRow};
use crate::index::subject::ProjectSubject;
use crate::index::IndexError;
use crate::protocol::{BadgeKind, ProjectId};

use super::kind_text;

/// An award as the evaluator computed it, before it is written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NewAward {
    /// The badge.
    pub(crate) kind: BadgeKind,
    /// The moment it was earned; a day-grain earn holds the day's midnight.
    pub(crate) earned_at: i64,
    /// That moment's offset in minutes; `0` for a day-grain earn.
    pub(crate) tz_offset_min: i64,
    /// The local date it was earned on.
    pub(crate) earned_on: String,
    /// The project it names, for the two kinds that name one.
    pub(crate) project: Option<ProjectId>,
    /// That project's subject at award time.
    pub(crate) subject_key: Option<String>,
    /// The encoded evidence.
    pub(crate) evidence: String,
    /// When the core writes the row.
    pub(crate) awarded_at: i64,
    /// Its local date.
    pub(crate) awarded_on: String,
}

/// The kinds already awarded. A stored kind this build does not know is refused rather than
/// skipped: skipping it would let the evaluator try to award it again.
///
/// # Errors
/// Fails when SQLite refuses the read, or a stored kind is not a `BadgeKind`.
pub(crate) fn latched(conn: &Connection) -> Result<Vec<BadgeKind>, IndexError> {
    let mut st = conn.prepare_cached("SELECT kind FROM badge_award")?;
    let kinds = st
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    kinds
        .into_iter()
        .map(|text| {
            serde_json::from_value(serde_json::Value::String(text.clone())).map_err(|_| {
                IndexError::Corrupt {
                    detail: format!("badge_award holds a kind this build does not know: {text}"),
                }
            })
        })
        .collect()
}

/// Write `award`, the evaluator's one way in. Answers whether a row went in: a held kind is
/// ignored by `badge_award_no_reinsert`, so a second award of a kind writes nothing.
///
/// # Errors
/// Fails when SQLite refuses the write.
pub(crate) fn insert_award(tx: &Transaction<'_>, award: &NewAward) -> Result<bool, IndexError> {
    let row = SidecarBadgeAward {
        kind: kind_text(award.kind),
        earned_at: award.earned_at,
        tz_offset_min: award.tz_offset_min,
        earned_on: award.earned_on.clone(),
        subject_key: award.subject_key.clone(),
        evidence: award.evidence.clone(),
        awarded_at: award.awarded_at,
        awarded_on: award.awarded_on.clone(),
        landed_at: None,
        landed_on: None,
    };
    insert(tx, &row, award.project)
}

/// The product's one `INSERT INTO badge_award`, shared by the evaluator and the restore.
fn insert(
    tx: &Transaction<'_>,
    row: &SidecarBadgeAward,
    project: Option<ProjectId>,
) -> Result<bool, IndexError> {
    let wrote = tx.execute(
        "INSERT INTO badge_award (kind, earned_at, tz_offset_min, earned_on, project_id,
                                  subject_key, evidence, awarded_at, awarded_on, landed_at,
                                  landed_on)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        rusqlite::params![
            row.kind,
            row.earned_at,
            row.tz_offset_min,
            row.earned_on,
            project.map(|p| p.0),
            row.subject_key,
            row.evidence,
            row.awarded_at,
            row.awarded_on,
            row.landed_at,
            row.landed_on,
        ],
    )?;
    Ok(wrote > 0)
}

/// One award as the sidecar carries it: every column of its row but `project_id`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SidecarBadgeAward {
    /// The badge, as `badge_award.kind` spells it.
    pub kind: String,
    /// The moment it was earned, in Unix seconds; a day-grain earn holds the day's midnight.
    pub earned_at: i64,
    /// The offset of that moment, in minutes; `0` for a day-grain earn.
    pub tz_offset_min: i64,
    /// The local date it was earned on, `YYYY-MM-DD`.
    pub earned_on: String,
    /// The subject of the project it names, as it was when awarded; `None` for a library-wide one.
    pub subject_key: Option<String>,
    /// The serialised evidence, never re-derived.
    pub evidence: String,
    /// When the core wrote the row, in Unix seconds.
    pub awarded_at: i64,
    /// The local date the core wrote it on.
    pub awarded_on: String,
    /// When it was first shown as a moment; `None` if never.
    pub landed_at: Option<i64>,
    /// The local date it was first shown on; `None` if never.
    pub landed_on: Option<String>,
}

/// Every award, by kind, as the sidecar carries it.
///
/// # Errors
/// Fails when SQLite refuses the read.
pub fn export_rows(conn: &Connection) -> Result<Vec<SidecarBadgeAward>, IndexError> {
    let mut st = conn.prepare(
        "SELECT kind, earned_at, tz_offset_min, earned_on, subject_key, evidence, awarded_at,
                awarded_on, landed_at, landed_on
         FROM badge_award ORDER BY kind",
    )?;
    let rows = st
        .query_map([], |r| {
            Ok(SidecarBadgeAward {
                kind: r.get(0)?,
                earned_at: r.get(1)?,
                tz_offset_min: r.get(2)?,
                earned_on: r.get(3)?,
                subject_key: r.get(4)?,
                evidence: r.get(5)?,
                awarded_at: r.get(6)?,
                awarded_on: r.get(7)?,
                landed_at: r.get(8)?,
                landed_on: r.get(9)?,
            })
        })?
        .collect::<Result<_, _>>()?;
    Ok(rows)
}

/// Insert each award whose kind is not held, and answer how many went in.
///
/// A held kind is left exactly as it is: the plain `INSERT` meets `badge_award_no_reinsert`,
/// which ignores it. `project_id` is set only when exactly one live project holds the award's
/// subject now (§48.8.4's never-guess rule); otherwise it stays NULL until the subject is
/// rediscovered.
///
/// # Errors
/// Fails when SQLite refuses a read or the write — a kind outside the CHECK, among others.
pub fn restore_rows(tx: &Transaction<'_>, rows: &[SidecarBadgeAward]) -> Result<u64, IndexError> {
    let mut inserted = 0;
    for row in rows {
        let project = match row.subject_key.as_deref().and_then(ProjectSubject::parse) {
            Some(subject) => resolve_subject_unique(tx, &subject)?,
            None => None,
        };
        inserted += u64::from(insert(tx, row, project)?);
    }
    Ok(inserted)
}

fn section_error(error: impl std::fmt::Display) -> IndexError {
    IndexError::Sidecar(format!("a badge_award row: {error}"))
}

/// The `badge_award` section's export: one row of no subject per award, its project carried in
/// `subject_key` inside the data.
///
/// # Errors
/// Fails when the awards cannot be read or encoded.
pub fn export_section(conn: &Connection) -> Result<Vec<SectionRow>, IndexError> {
    export_rows(conn)?
        .into_iter()
        .map(|award| {
            Ok(SectionRow {
                subject: None,
                location_keys: Vec::new(),
                data: serde_json::to_value(award).map_err(section_error)?,
            })
        })
        .collect()
}

/// The `badge_award` section's restore, through [`restore_rows`].
///
/// Answers `Applied(1)` when the kind was absent and `Applied(0)` when it was held. Never
/// `Pending`: the section is global, and an award whose subject no single project holds is
/// restored with no project.
///
/// # Errors
/// Fails when the row does not decode as an award, or SQLite refuses the write.
pub fn restore_section(
    tx: &Transaction<'_>,
    row: &SectionRow,
    _ctx: &RestoreCtx<'_>,
) -> Result<RestoreOutcome, IndexError> {
    let award: SidecarBadgeAward =
        serde_json::from_value(row.data.clone()).map_err(section_error)?;
    Ok(RestoreOutcome::Applied(restore_rows(
        tx,
        std::slice::from_ref(&award),
    )?))
}
