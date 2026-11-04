//! §39.7–§39.9: the weekly rhythm's one stored thing, `rhythm_week` — a raise-only row per settled
//! local week — and its trip through the sidecar (§48.8.3).
//!
//! The schema refuses every lowering (`0020_rhythm.sql`'s triggers); this module is the one insert
//! and the one update, so a write that would trip a trigger is a defect here, never a user's loss.
//! Its settlement pass is their one production caller.

use std::collections::BTreeSet;

use rusqlite::{Connection, OptionalExtension as _, Transaction};

use super::active::{union_days, DaySpan};
use super::commit_days::git_days;
use super::focus::{focus_days, index_error};
use super::targets::{focus_target_minutes, rhythm_target, settle_rhythm_target};
use super::tend::tend_days;
use super::{RHYTHM_FALLBACK_TARGET, RHYTHM_MIN_ACTIVE_WEEKS, RHYTHM_WINDOW_WEEKS};
use crate::clock::{local_date, parse_local_date, week_start, LocalZone};
use crate::firstrun::first_run_completed_at;
use crate::index::sidecar::{RestoreCtx, RestoreOutcome, SectionRow};
use crate::index::IndexError;
use crate::ledger::level::pending_projects;
use crate::motivation::keys::day_one;
use crate::protocol::{RhythmCoverage, RhythmDay, RhythmState, RhythmTargetBasis};

const DAY_SECS: i64 = 86_400;

/// `rhythm_week.verdict`'s vocabulary. Core-only — the wire carries no verdict — and the column's
/// `CHECK` lists exactly these, which `rhythm_migration.rs` holds by inserting each.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RhythmVerdict {
    /// The week's active days reached its target.
    Counted,
    /// The week settled below its target; a late witness may still raise it to `Counted`.
    Missed,
    /// The week settled below its target but holds day one (§39.9): never a failure.
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

/// What one settlement pass did. The default is an idle pass, which wrote nothing.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SettleReport {
    /// Whether the pass was §39.8's first settlement, which derives the target.
    pub first_settlement: bool,
    /// How many weeks it inserted.
    pub inserted: u32,
    /// How many settled weeks' active days it raised.
    pub raised: u32,
    /// How many settled weeks' git coverage it moved from `partial` to `complete`.
    pub completed: u32,
    /// Whether it replaced a `fallback` target with a derived one, which happens once (§39.8).
    pub target_replaced: bool,
}

/// Day one (§39.9) as a local day number, parsed from §43's one reader of the key. Absent or
/// malformed reads `None`, so no week holds it and nothing fails on it.
///
/// # Errors
/// Fails when SQLite refuses the read.
pub fn day_one_epoch(conn: &Connection) -> Result<Option<i64>, IndexError> {
    Ok(day_one(conn)?.as_deref().and_then(parse_local_date))
}

fn monday_of(row: &RhythmWeekRow) -> Result<i64, IndexError> {
    parse_local_date(&row.week_start).ok_or_else(|| IndexError::Corrupt {
        detail: format!("rhythm_week {:?} is not a date", row.week_start),
    })
}

/// The Mondays of the weeks holding a `commit_day` row written after `mark`.
fn weeks_written_after(tx: &Transaction<'_>, mark: i64) -> Result<BTreeSet<i64>, IndexError> {
    let weeks = tx
        .prepare_cached("SELECT DISTINCT ts FROM xp_events WHERE kind = 'commit_day' AND id > ?1")?
        .query_map([mark], |r| r.get::<_, i64>(0))?
        .map(|ts| ts.map(|ts| week_start(ts.div_euclid(DAY_SECS))))
        .collect::<Result<_, _>>()?;
    Ok(weeks)
}

/// §39.6's active dates over `span`: one read of each witness, then their union. (s) is judged
/// against the Focus target in force now — no column stores the one a settled week saw.
fn active_dates(
    conn: &Connection,
    span: DaySpan,
    zone: &dyn LocalZone,
    now: i64,
) -> Result<BTreeSet<i64>, IndexError> {
    let target_secs = i64::from(focus_target_minutes(conn)?) * 60;
    Ok(union_days(
        &git_days(conn, span)?,
        &focus_days(conn, span, target_secs, zone, now).map_err(index_error)?,
        &tend_days(conn, span)?,
    ))
}

/// §39.6–§39.8's rhythm at the instant: the current week's seven cells, Monday first, from one
/// read of each witness over `[week_start(today), today]`, and the stored target, run and best.
///
/// A cell is `future` after today; else `active` if witnessed; else `waived` if it is day one
/// (§39.9 — never `inactive`, and on day one itself it wins over `pending`); else `pending` if
/// it is today; else `not_computed` while a project's history is pending, because (g) is then
/// unknown and a date is inactive only when all three witnesses are known false; else
/// `inactive`. (s) reads the Focus target in force now. The run and best are absent while
/// nothing is settled — never 0 — and `git_coverage` is `partial` while history is pending.
///
/// # Errors
/// Fails when the index cannot be read, or a stored week is malformed.
pub fn rhythm_state(
    conn: &Connection,
    today: i64,
    day_one: Option<i64>,
    zone: &dyn LocalZone,
    now: i64,
) -> Result<RhythmState, IndexError> {
    let monday = week_start(today);
    let active = active_dates(
        conn,
        DaySpan {
            first: monday,
            last: today,
        },
        zone,
        now,
    )?;
    let history_pending = pending_projects(conn)? > 0;
    let days: Vec<RhythmDay> = (monday..monday + 7)
        .map(|d| {
            if d > today {
                RhythmDay::Future
            } else if active.contains(&d) {
                RhythmDay::Active
            } else if Some(d) == day_one {
                RhythmDay::Waived
            } else if d == today {
                RhythmDay::Pending
            } else if history_pending {
                RhythmDay::NotComputed
            } else {
                RhythmDay::Inactive
            }
        })
        .collect();
    let active_days =
        u32::try_from(days.iter().filter(|&&d| d == RhythmDay::Active).count()).unwrap_or(7);
    let target = rhythm_target(conn)?;
    let rows = load_weeks(conn)?;
    let (run_weeks, best_weeks) = if rows.is_empty() {
        (None, None)
    } else {
        let met = target.is_some_and(|(t, _)| active_days >= t);
        (Some(run_weeks(&rows, met)), Some(best_weeks(&rows)))
    };
    Ok(RhythmState {
        days,
        active_days,
        target: target.map(|(t, _)| t),
        target_basis: target.map(|(_, basis)| basis),
        run_weeks,
        best_weeks,
        git_coverage: if history_pending {
            RhythmCoverage::Partial
        } else {
            RhythmCoverage::Complete
        },
    })
}

/// §39.7's one settlement pass. Idempotent, and a pass with nothing to do writes nothing, so it
/// never moves the change counter §43's publisher reads.
///
/// Every complete week after the newest row up to the week before today is inserted, oldest
/// first, against the target in force; then the settled weeks holding a `commit_day` row with
/// an id above `raise_mark` — and, once nothing is pending, every `partial` week — are re-read
/// and raised where they rise. A narrowing that deleted rows recomputes lower, and nothing is
/// written. `raise_mark` is the highest `xp_events.id` a previous pass has read; 0 re-reads
/// every settled week. An empty table is §39.8's first settlement; a `fallback` target is replaced
/// once, after the pass's inserts. Quiet mode is never read.
///
/// Until this index has finished a scan the pass is idle. `scan_run` is never restored, so a
/// rebuilt index has none until it reads the disk again, while the projects it restored carry no
/// present copy and so leave nothing pending: a week settled then would claim a complete, empty
/// history the index has not read, and `complete` never moves back to `partial`. A cancelled scan
/// read only part of the disk and does not count.
///
/// # Errors
/// Fails when the index cannot be read or written, or a stored week is malformed.
pub fn settle(
    tx: &Transaction<'_>,
    now: i64,
    zone: &dyn LocalZone,
    raise_mark: &mut i64,
) -> Result<SettleReport, IndexError> {
    let mut report = SettleReport::default();
    let scanned: bool = tx.query_row(
        "SELECT EXISTS (SELECT 1 FROM scan_run WHERE ended_at IS NOT NULL AND cancelled = 0)",
        [],
        |r| r.get(0),
    )?;
    if !scanned {
        return Ok(report);
    }
    let settled = load_weeks(tx)?;
    let Some(newest) = settled.last() else {
        return first_settlement(tx, now, zone, raise_mark);
    };
    let last_complete = week_start(zone.at(now).day()) - 7;
    let new_weeks: Vec<i64> = (week_start(monday_of(newest)?) + 7..=last_complete)
        .step_by(7)
        .collect();
    let written = weeks_written_after(tx, *raise_mark)?;
    let any_partial = settled
        .iter()
        .any(|w| w.git_coverage == RhythmCoverage::Partial);
    if !new_weeks.is_empty() || !written.is_empty() || any_partial {
        let coverage = if pending_projects(tx)? == 0 {
            RhythmCoverage::Complete
        } else {
            RhythmCoverage::Partial
        };
        let mut raise = Vec::new();
        for row in &settled {
            let monday = monday_of(row)?;
            if written.contains(&monday)
                || (coverage == RhythmCoverage::Complete
                    && row.git_coverage == RhythmCoverage::Partial)
            {
                raise.push((monday, row));
            }
        }
        let mondays = || new_weeks.iter().copied().chain(raise.iter().map(|r| r.0));
        if let (Some(first), Some(last)) = (mondays().min(), mondays().max()) {
            let active = active_dates(
                tx,
                DaySpan {
                    first,
                    last: last + 6,
                },
                zone,
                now,
            )?;
            let days_in =
                |monday: i64| u32::try_from(active.range(monday..=monday + 6).count()).unwrap_or(7);
            insert_weeks(tx, &new_weeks, &days_in, coverage, now, &mut report)?;
            for (monday, row) in raise {
                let days = days_in(monday);
                if raise_week(tx, &row.week_start, days, coverage)? {
                    report.raised += u32::from(days > row.active_days);
                    report.completed += u32::from(
                        row.git_coverage == RhythmCoverage::Partial
                            && coverage == RhythmCoverage::Complete,
                    );
                }
            }
        }
    }
    if matches!(rhythm_target(tx)?, Some((_, RhythmTargetBasis::Fallback))) {
        report.target_replaced = replace_fallback(tx, last_complete)?;
    }
    *raise_mark = max_event_id(tx)?;
    Ok(report)
}

fn max_event_id(tx: &Transaction<'_>) -> Result<i64, IndexError> {
    Ok(
        tx.query_row("SELECT COALESCE(MAX(id), 0) FROM xp_events", [], |r| {
            r.get(0)
        })?,
    )
}

/// The lower median: the lower middle of an even count. `None` for no values. Sorts `values`.
#[must_use]
pub fn lower_median(values: &mut [u32]) -> Option<u32> {
    values.sort_unstable();
    let middle = values.len().checked_sub(1)? / 2;
    values.get(middle).copied()
}

/// The Monday of the oldest of the window's weeks, the newest being `last_complete`.
fn window_first(last_complete: i64) -> i64 {
    last_complete - 7 * (i64::from(RHYTHM_WINDOW_WEEKS) - 1)
}

/// §39.8's first settlement, over an empty table: only once first run has ended and no
/// project's history is pending — otherwise nothing is written, and the target, the run and
/// best stay absent rather than 0.
///
/// The window's 52 complete weeks are read once per witness and inserted against the target:
/// a target already stored is kept (a `chosen` one is the user's, and a `derived` one is never
/// recomputed); else the lower median of the active weeks, or the fallback below
/// [`RHYTHM_MIN_ACTIVE_WEEKS`] of them. Every row was just computed from the ledgers as they
/// stand, so the raise mark moves to the newest event.
fn first_settlement(
    tx: &Transaction<'_>,
    now: i64,
    zone: &dyn LocalZone,
    raise_mark: &mut i64,
) -> Result<SettleReport, IndexError> {
    let mut report = SettleReport::default();
    if first_run_completed_at(tx)?.is_none() || pending_projects(tx)? != 0 {
        return Ok(report);
    }
    let last_complete = week_start(zone.at(now).day()) - 7;
    let first = window_first(last_complete);
    let weeks: Vec<i64> = (first..=last_complete).step_by(7).collect();
    let active = active_dates(
        tx,
        DaySpan {
            first,
            last: last_complete + 6,
        },
        zone,
        now,
    )?;
    let days_in =
        |monday: i64| u32::try_from(active.range(monday..=monday + 6).count()).unwrap_or(7);
    let kept = rhythm_target(tx)?.filter(|(_, basis)| *basis != RhythmTargetBasis::Fallback);
    if kept.is_none() {
        let mut active_weeks: Vec<u32> = weeks
            .iter()
            .map(|&monday| days_in(monday))
            .filter(|&days| days > 0)
            .collect();
        let derived = lower_median(&mut active_weeks)
            .filter(|_| active_weeks.len() >= RHYTHM_MIN_ACTIVE_WEEKS);
        match derived {
            Some(derived) => settle_rhythm_target(tx, derived, RhythmTargetBasis::Derived)?,
            None => settle_rhythm_target(tx, RHYTHM_FALLBACK_TARGET, RhythmTargetBasis::Fallback)?,
        }
    }
    insert_weeks(
        tx,
        &weeks,
        &days_in,
        RhythmCoverage::Complete,
        now,
        &mut report,
    )?;
    report.first_settlement = true;
    *raise_mark = max_event_id(tx)?;
    Ok(report)
}

/// Replaces a `fallback` target once the rows of the trailing window hold enough active weeks,
/// by their lower median capped at the fallback — the app never raises a target. Its basis
/// becomes `derived`, so no later pass re-checks it. Whether it wrote.
fn replace_fallback(tx: &Transaction<'_>, last_complete: i64) -> Result<bool, IndexError> {
    let window = window_first(last_complete)..=last_complete;
    let mut active_weeks = Vec::new();
    for row in load_weeks(tx)? {
        if row.active_days > 0 && window.contains(&monday_of(&row)?) {
            active_weeks.push(row.active_days);
        }
    }
    if active_weeks.len() < RHYTHM_MIN_ACTIVE_WEEKS {
        return Ok(false);
    }
    let Some(derived) = lower_median(&mut active_weeks) else {
        return Ok(false);
    };
    settle_rhythm_target(
        tx,
        derived.min(RHYTHM_FALLBACK_TARGET),
        RhythmTargetBasis::Derived,
    )?;
    Ok(true)
}

/// Inserts `weeks`, oldest first, against the target in force — the fallback written first when
/// a restored table arrived without its target. The week holding day one never settles `missed`.
fn insert_weeks(
    tx: &Transaction<'_>,
    weeks: &[i64],
    days_in: &dyn Fn(i64) -> u32,
    coverage: RhythmCoverage,
    now: i64,
    report: &mut SettleReport,
) -> Result<(), IndexError> {
    if weeks.is_empty() {
        return Ok(());
    }
    let target = if let Some((target, _)) = rhythm_target(tx)? {
        target
    } else {
        settle_rhythm_target(tx, RHYTHM_FALLBACK_TARGET, RhythmTargetBasis::Fallback)?;
        RHYTHM_FALLBACK_TARGET
    };
    let day_one_week = day_one_epoch(tx)?.map(week_start);
    for &monday in weeks {
        let active_days = days_in(monday);
        let verdict = if active_days >= target {
            RhythmVerdict::Counted
        } else if day_one_week == Some(monday) {
            RhythmVerdict::Waived
        } else {
            RhythmVerdict::Missed
        };
        insert_week(
            tx,
            &RhythmWeekRow {
                week_start: local_date(monday),
                active_days,
                target,
                verdict,
                git_coverage: coverage,
                settled_at: now,
            },
        )?;
        report.inserted += 1;
    }
    Ok(())
}

/// §39.7's run: the consecutive `counted` weeks ending at the newest row, plus the current week.
///
/// A `waived` week bridges; a `missed` week or a calendar gap ends the run. The current week
/// adds one when it is met. `rows` oldest first, as [`load_weeks`] answers them.
#[must_use]
pub fn run_weeks(rows: &[RhythmWeekRow], current_met: bool) -> u32 {
    let mut run = 0_u32;
    let mut later: Option<i64> = None;
    for row in rows.iter().rev() {
        let Some(monday) = parse_local_date(&row.week_start) else {
            break;
        };
        if later.is_some_and(|next| next != monday + 7) {
            break;
        }
        later = Some(monday);
        match row.verdict {
            RhythmVerdict::Counted => run += 1,
            RhythmVerdict::Waived => {}
            RhythmVerdict::Missed => break,
        }
    }
    run + u32::from(current_met)
}

/// §39.7's best rhythm: the longest run over the rows alone, in weeks. The rows only rise, so
/// it never falls; no copy of it is stored.
#[must_use]
pub fn best_weeks(rows: &[RhythmWeekRow]) -> u32 {
    let (mut best, mut run) = (0_u32, 0_u32);
    let mut earlier: Option<i64> = None;
    for row in rows {
        let monday = parse_local_date(&row.week_start);
        if monday.is_none() || earlier.is_some_and(|prev| Some(prev + 7) != monday) {
            run = 0;
        }
        earlier = monday;
        match row.verdict {
            RhythmVerdict::Counted => {
                run += 1;
                best = best.max(run);
            }
            RhythmVerdict::Waived => {}
            RhythmVerdict::Missed => run = 0,
        }
    }
    best
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
