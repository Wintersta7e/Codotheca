//! §39.7–§39.9: the weekly rhythm's one stored thing, `rhythm_week` — a raise-only row per settled
//! local week — and its trip through the sidecar (§48.8.3).
//!
//! The schema refuses every lowering (`0020_rhythm.sql`'s triggers); this module is the one insert
//! and the one update, so a write that would trip a trigger is a defect here, never a user's loss.

use rusqlite::{Connection, OptionalExtension as _, Transaction};

use crate::clock::{parse_local_date, week_start};
use crate::index::sidecar::{RestoreCtx, RestoreOutcome, SectionRow};
use crate::index::IndexError;
use crate::protocol::RhythmCoverage;

/// `rhythm_week.verdict`'s vocabulary. Core-only — the wire carries no verdict — and the column's
/// `CHECK` lists exactly these, which `rhythm_migration.rs` holds by inserting each.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RhythmVerdict {
    /// The week's active days reached its target.
    Counted,
    /// The week settled below its target; a late witness may still raise it to `Counted`.
    Missed,
    /// The week settled before the first settlement could judge it (§39.8): never a failure.
    Waived,
}

impl RhythmVerdict {
    /// Every verdict, in the column's order.
    pub const ALL: [Self; 3] = [Self::Counted, Self::Missed, Self::Waived];

    /// The column's spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Counted => "counted",
            Self::Missed => "missed",
            Self::Waived => "waived",
        }
    }

    /// The verdict a column value spells, or `None` for one this build does not write.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|v| v.as_str() == text)
    }
}

/// The column's spelling of a coverage, which is the schema enum's own.
const fn coverage_str(coverage: RhythmCoverage) -> &'static str {
    match coverage {
        RhythmCoverage::Complete => "complete",
        RhythmCoverage::Partial => "partial",
    }
}

fn parse_coverage(text: &str) -> Option<RhythmCoverage> {
    RhythmCoverage::ALL
        .into_iter()
        .find(|c| coverage_str(*c) == text)
}

/// One settled week, as `rhythm_week` holds it and the sidecar carries it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RhythmWeekRow {
    /// The week's Monday, `YYYY-MM-DD`.
    pub week_start: String,
    /// How many of its seven days were active (§39.6).
    pub active_days: u32,
    /// The rhythm target in force when it settled; never rewritten.
    pub target: u32,
    /// Whether the week counted, missed, or was waived.
    pub verdict: RhythmVerdict,
    /// Whether every project's git history was read for the week.
    pub git_coverage: RhythmCoverage,
    /// When it settled, in Unix seconds.
    pub settled_at: i64,
}

/// Every settled week, oldest first.
///
/// # Errors
/// Fails when SQLite refuses the read, or a stored value is one this build does not write.
pub fn load_weeks(conn: &Connection) -> Result<Vec<RhythmWeekRow>, IndexError> {
    let raw: Vec<(String, i64, i64, String, String, i64)> = conn
        .prepare(
            "SELECT week_start, active_days, target, verdict, git_coverage, settled_at
               FROM rhythm_week ORDER BY week_start",
        )?
        .query_map([], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
            ))
        })?
        .collect::<Result<_, _>>()?;
    raw.into_iter()
        .map(
            |(week_start, days, target, verdict, coverage, settled_at)| {
                let bad = |what: &str| IndexError::Corrupt {
                    detail: format!("rhythm_week {week_start}: {what}"),
                };
                Ok(RhythmWeekRow {
                    active_days: u32::try_from(days).map_err(|_| bad("active_days"))?,
                    target: u32::try_from(target).map_err(|_| bad("target"))?,
                    verdict: RhythmVerdict::parse(&verdict).ok_or_else(|| bad("verdict"))?,
                    git_coverage: parse_coverage(&coverage).ok_or_else(|| bad("git_coverage"))?,
                    settled_at,
                    week_start,
                })
            },
        )
        .collect()
}

/// The ONE insert of a settled week. Its `week_start` must be a Monday written `YYYY-MM-DD`
/// (§39.2 rule 4): the column cannot say so, so the writer does.
pub(crate) fn insert_week(tx: &Transaction<'_>, row: &RhythmWeekRow) -> Result<(), IndexError> {
    if parse_local_date(&row.week_start).is_none_or(|day| week_start(day) != day) {
        return Err(IndexError::Corrupt {
            detail: format!("rhythm_week {:?} is not a Monday", row.week_start),
        });
    }
    tx.execute(
        "INSERT INTO rhythm_week
            (week_start, active_days, target, verdict, git_coverage, settled_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![
            row.week_start,
            row.active_days,
            row.target,
            row.verdict.as_str(),
            coverage_str(row.git_coverage),
            row.settled_at,
        ],
    )?;
    Ok(())
}

/// The ONE update: raises a settled week's active days and coverage, never lowers either, and
/// writes nothing when nothing rises — so an idle settlement moves no change counter and never
/// wakes §43's publisher. The verdict follows the row's own immutable target: `counted` once the
/// raised days reach it, otherwise unchanged, so the verdict `CHECK` and the trigger agree by
/// construction. Answers whether it wrote; an absent week is not written.
pub(crate) fn raise_week(
    tx: &Transaction<'_>,
    week_start: &str,
    active_days: u32,
    coverage: RhythmCoverage,
) -> Result<bool, IndexError> {
    let present: Option<(i64, i64, String, String)> = tx
        .query_row(
            "SELECT active_days, target, verdict, git_coverage FROM rhythm_week
              WHERE week_start = ?1",
            [week_start],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?;
    let Some((old_days, target, old_verdict, old_coverage)) = present else {
        return Ok(false);
    };
    let days = old_days.max(i64::from(active_days));
    let new_coverage = if coverage == RhythmCoverage::Complete {
        coverage_str(RhythmCoverage::Complete)
    } else {
        old_coverage.as_str()
    };
    let verdict = if days >= target {
        RhythmVerdict::Counted.as_str()
    } else {
        old_verdict.as_str()
    };
    if days == old_days && new_coverage == old_coverage && verdict == old_verdict {
        return Ok(false);
    }
    tx.execute(
        "UPDATE rhythm_week SET active_days = ?2, verdict = ?3, git_coverage = ?4
          WHERE week_start = ?1",
        rusqlite::params![week_start, days, verdict, new_coverage],
    )?;
    Ok(true)
}

/// §48.8.3's raise-only restore of one week; answers whether it wrote.
///
/// Absent, the week is inserted as exported; present, it rises to the restored days and coverage
/// through [`raise_week`] — never a lowering, never a second insert.
///
/// # Errors
/// Fails when SQLite refuses the read or the write.
pub fn restore_week(tx: &Transaction<'_>, row: &RhythmWeekRow) -> Result<bool, IndexError> {
    let present: bool = tx.query_row(
        "SELECT EXISTS (SELECT 1 FROM rhythm_week WHERE week_start = ?1)",
        [&row.week_start],
        |r| r.get(0),
    )?;
    if present {
        return raise_week(tx, &row.week_start, row.active_days, row.git_coverage);
    }
    insert_week(tx, row)?;
    Ok(true)
}

fn section_error(error: impl std::fmt::Display) -> IndexError {
    IndexError::Sidecar(format!("a rhythm_week row: {error}"))
}

/// The `rhythm_week` section's export: every settled week, as a row of no project.
///
/// # Errors
/// Fails when the weeks cannot be read or encoded.
pub fn export_section(conn: &Connection) -> Result<Vec<SectionRow>, IndexError> {
    load_weeks(conn)?
        .into_iter()
        .map(|week| {
            Ok(SectionRow {
                subject: None,
                location_keys: Vec::new(),
                data: serde_json::to_value(week).map_err(section_error)?,
            })
        })
        .collect()
}

/// The `rhythm_week` section's restore, through [`restore_week`]: one row when it wrote, none when
/// the present week already stood as high.
///
/// # Errors
/// Fails when the row does not decode as a week, or SQLite refuses the write.
pub fn restore_section(
    tx: &Transaction<'_>,
    row: &SectionRow,
    _ctx: &RestoreCtx<'_>,
) -> Result<RestoreOutcome, IndexError> {
    let week: RhythmWeekRow = serde_json::from_value(row.data.clone()).map_err(section_error)?;
    let wrote = restore_week(tx, &week)?;
    Ok(RestoreOutcome::Applied(u64::from(wrote)))
}
