//! §40.6's Lifeline: the run state, the day close, the clear-day test and the daily pass.
//!
//! Everything rides the core's one loop-thread tick. The close of a local date runs on the first
//! tick whose date is later, before that date's pass: a day is clear only when every project of
//! **L** had its pass location read that day and reads `pushed` as `pass` at the close. A day the
//! core did not run is never closed, so the next clear day starts a new run — an unread day is
//! not a clear one, because the app never claims currency it does not have.

use rusqlite::{Connection, OptionalExtension as _, Transaction};
use serde::{Deserialize, Serialize};

use crate::clock::{local_date, parse_local_date};
use crate::index::IndexError;
use crate::protocol::{BadgeKind, CheckState, CompletionCheck, LocationId, ProjectId};

use super::evaluate::{sweep_unlatched, EvalCtx};
use super::population::lifeline_population;
use super::store::resolve_project_ids;
use super::LIFELINE_DAYS;

/// The `app_meta` key holding the run. One writer, the close; never on the wire, and not
/// rebuild-owned, so it rides the settings export.
pub const LIFELINE_RUN_KEY: &str = "badges.lifeline_run";

/// What one tick of the day close did.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DayTick {
    /// The copies the pass should read now: on a rollover, and on a process's first tick.
    pub pass: Vec<PassTarget>,
    /// Whether the local date moved past the open one (or none was open), so the close, the
    /// re-resolution and the sweep ran.
    pub rolled_over: bool,
    /// The kinds this tick awarded.
    pub awarded: Vec<BadgeKind>,
}

/// One project's pass location, as the pass enqueues it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PassTarget {
    /// The project of **L**.
    pub project: ProjectId,
    /// The copy `unpushed_commits` reads, `pick_primary`'s choice.
    pub location: LocationId,
    /// The copy's store, for the scheduler's per-store slot.
    pub store_key: String,
    /// The copy's path as stored, for the caller to resolve its mount class.
    pub path_bytes: Vec<u8>,
}

/// `badges.lifeline_run`: the open date and the run, local `YYYY-MM-DD` or `null`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Run {
    open: Option<String>,
    #[serde(rename = "runStart")]
    start: Option<String>,
    #[serde(rename = "runEnd")]
    end: Option<String>,
}

/// The stored run. A value that does not decode resets it: the run is not an earned thing, and a
/// reset removes nothing (§40.6 rule 4).
fn stored_run(conn: &Connection) -> Result<Run, IndexError> {
    let text: Option<String> = conn
        .query_row(
            "SELECT v FROM app_meta WHERE k = ?1",
            [LIFELINE_RUN_KEY],
            |r| r.get(0),
        )
        .optional()?;
    Ok(text
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default())
}

/// The stored run as day numbers, when it holds `LIFELINE_DAYS` or more: `(runStart, runEnd)`.
///
/// # Errors
/// Fails when SQLite refuses the read.
pub(crate) fn completed_run(conn: &Connection) -> Result<Option<(i64, i64)>, IndexError> {
    let run = stored_run(conn)?;
    let day = |d: &Option<String>| d.as_deref().and_then(parse_local_date);
    Ok(match (day(&run.start), day(&run.end)) {
        (Some(start), Some(end)) if end - start + 1 >= LIFELINE_DAYS => Some((start, end)),
        _ => None,
    })
}

/// §40.6's tick: close the open date when today is later, and on that rollover open today.
///
/// A rollover also runs the re-resolution of awards waiting for their project, then sweeps every
/// unlatched kind. Answers the pass targets on a rollover and on the process's first tick. A clock stepped back
/// closes nothing and changes nothing.
///
/// # Errors
/// Fails when SQLite refuses a read or a write.
pub fn day_tick(
    tx: &Transaction<'_>,
    ctx: &EvalCtx<'_>,
    first_tick_of_process: bool,
) -> Result<DayTick, IndexError> {
    let today = ctx.zone.at(ctx.now).day();
    let mut run = stored_run(tx)?;
    let open = run.open.as_deref().and_then(parse_local_date);
    let rolled_over = open.is_none_or(|d| today > d);
    let awarded = if rolled_over {
        if let Some(d) = open {
            close(tx, &mut run, d, ctx)?;
        }
        run.open = Some(local_date(today));
        write_run(tx, &run)?;
        resolve_project_ids(tx)?;
        // Lifeline's arm reads the run just written, so a run long enough is awarded here, in
        // the close's own transaction, with every other unlatched kind.
        sweep_unlatched(tx, ctx)?
    } else {
        Vec::new()
    };
    let pass = if rolled_over || first_tick_of_process {
        pass_targets(tx)?
    } else {
        Vec::new()
    };
    Ok(DayTick {
        pass,
        rolled_over,
        awarded,
    })
}

/// Close day `d`: a clear day extends a run ending on `d − 1` or starts one; any other ends it.
fn close(tx: &Transaction<'_>, run: &mut Run, d: i64, ctx: &EvalCtx<'_>) -> Result<(), IndexError> {
    if clear(tx, d, ctx)? {
        let continues =
            run.end.as_deref().and_then(parse_local_date) == Some(d - 1) && run.start.is_some();
        if !continues {
            run.start = Some(local_date(d));
        }
        run.end = Some(local_date(d));
    } else {
        run.start = None;
        run.end = None;
    }
    Ok(())
}

fn write_run(tx: &Transaction<'_>, run: &Run) -> Result<(), IndexError> {
    let text = serde_json::to_string(run).map_err(|e| IndexError::Corrupt {
        detail: format!("{LIFELINE_RUN_KEY}: {e}"),
    })?;
    tx.execute(
        "INSERT INTO app_meta (k, v) VALUES (?1, ?2)
         ON CONFLICT(k) DO UPDATE SET v = excluded.v",
        rusqlite::params![LIFELINE_RUN_KEY, text],
    )?;
    Ok(())
}

/// `p`'s pass location: the copy §28.2's `unpushed_commits` arm reads.
fn pass_location(
    conn: &Connection,
    project: ProjectId,
) -> Result<Option<crate::projects::rows::LocationFacts>, IndexError> {
    let locations =
        crate::projects::rows::locations_of(conn, project).map_err(|e| IndexError::Corrupt {
            detail: format!("locations_of: {e}"),
        })?;
    Ok(crate::projects::rows::pick_primary(&locations).cloned())
}

/// §40.6 rule 2: **L** is non-empty, and every project in it had its pass location's refs read
/// on `d` and stores `pushed` as `pass`. `fail`, `unknown`, `na` and a missing row are not clear;
/// the stored state at the close is the day's last reading.
fn clear(conn: &Connection, d: i64, ctx: &EvalCtx<'_>) -> Result<bool, IndexError> {
    let population = lifeline_population(conn)?;
    if population.is_empty() {
        return Ok(false);
    }
    for project in population {
        let read_on_d = pass_location(conn, project)?
            .and_then(|l| l.refstate_observed_at)
            .is_some_and(|at| ctx.zone.at(at).day() == d);
        let pushed = crate::completion::store::load_rows(conn, project)?
            .into_iter()
            .find(|row| row.key == CompletionCheck::Pushed)
            .is_some_and(|row| row.state == CheckState::Pass);
        if !(read_on_d && pushed) {
            return Ok(false);
        }
    }
    Ok(true)
}

/// The pass: every project of **L** on its pass location, skipping a copy that is not current
/// (§46.9) — its path may hold another repository by now.
fn pass_targets(conn: &Connection) -> Result<Vec<PassTarget>, IndexError> {
    let mut targets = Vec::new();
    for project in lifeline_population(conn)? {
        let Some(location) = pass_location(conn, project)? else {
            continue;
        };
        if !crate::projects::current::location_is_current(conn, location.id)? {
            continue;
        }
        let (store_key, path_bytes) = conn.query_row(
            "SELECT store_key, path_bytes FROM location WHERE id = ?1",
            [location.id.0],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        targets.push(PassTarget {
            project,
            location: location.id,
            store_key,
            path_bytes,
        });
    }
    Ok(targets)
}
