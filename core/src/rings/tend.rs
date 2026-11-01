//! §39.5's Tend ring over its domain `E`, and §39.6's witness (t), the days of `T`.
//!
//! `T` is the `debt_day` rows that earn. Reads the debt ledger (`debt_item`, `debt_sweep`,
//! `xp_events`), the projects and their copies, and the health switches.

use std::collections::{BTreeMap, BTreeSet};

use rusqlite::Connection;

use super::active::DaySpan;
use crate::health::enrolment::{is_enrolled, surface_suppressed};
use crate::health::switches::{off_sources, read_switches};
use crate::index::IndexError;
use crate::ledger::xp::debt_day_priced;
use crate::projects::current::IS_CURRENT_SQL;
use crate::protocol::{DebtSource, ProjectId, RingStatus, TendRing};
use crate::surfaces::settings::content_scan_enabled;

const DAY_SECS: i64 = 86_400;

/// `E`, the projects the Tend ring asks about (§39.5).
///
/// Not a Reference, not merged away, current (§46.9 — not removed, with at least one unremoved
/// copy; an offline copy keeps it), and not surface-suppressed by §30.5's one predicate.
///
/// The predicate reads the enrolment and the archive flag here; §44's verdict is to enter it
/// through `EnrolmentFacts` and `is_retired`, a conversion §44's Amnesty owns. Never a second
/// list beside §30.5's.
///
/// # Errors
/// Fails when SQLite refuses the read.
pub fn tend_domain(conn: &Connection) -> Result<Vec<ProjectId>, IndexError> {
    let mut stmt = conn.prepare(&format!(
        "SELECT p.id, p.acknowledged_at, p.is_archived FROM project p
          WHERE p.is_reference = 0 AND p.merged_into IS NULL AND {IS_CURRENT_SQL}
          ORDER BY p.id"
    ))?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, Option<i64>>(1)?,
            r.get::<_, i64>(2)?,
        ))
    })?;
    let mut domain = Vec::new();
    for row in rows {
        let (id, acknowledged_at, is_archived) = row?;
        if !surface_suppressed(is_enrolled(acknowledged_at), is_archived != 0) {
            domain.push(ProjectId(id));
        }
    }
    Ok(domain)
}

/// One project of `E` as the *nothing to tend* predicate reads it.
#[derive(Default)]
struct Looked {
    /// An open scored item, or an unverified one.
    blocking_item: bool,
    /// Every stored sweep outcome, by its source's column spelling.
    outcomes: BTreeMap<String, String>,
}

fn looked_at(conn: &Connection, project: ProjectId) -> Result<Looked, IndexError> {
    let blocking_item: bool = conn
        .prepare_cached(
            "SELECT EXISTS (SELECT 1 FROM debt_item WHERE project_id = ?1
                 AND ((state = 'open' AND scoring = 'scored') OR state = 'unverified'))",
        )?
        .query_row([project.0], |r| r.get(0))?;
    let outcomes = conn
        .prepare_cached("SELECT source, outcome FROM debt_sweep WHERE project_id = ?1")?
        .query_map([project.0], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<Result<_, _>>()?;
    Ok(Looked {
        blocking_item,
        outcomes,
    })
}

fn column_slug(source: DebtSource) -> Result<String, IndexError> {
    match serde_json::to_value(source) {
        Ok(serde_json::Value::String(slug)) => Ok(slug),
        _ => Err(IndexError::Sidecar(
            "a generated enum did not serialise as a string".into(),
        )),
    }
}

/// Today's Tend ring, `today` a local day number (§39.5).
///
/// First match: a `T` row dated today closes it, earned — before `|E| = 0`, so a Seal later the
/// same day keeps it; an empty `E` is `NotComputed`; then *nothing to tend*; else `Open`. A
/// source is excluded when switched off or, for `todo_marker`, without the content-scan grant;
/// it never blocks. *Nothing to tend* closes only of what was read: a `partial`, `failed`,
/// missing or stale `skipped_*` sweep row of a permitted source blocks, and so does an open
/// scored or an unverified item. Binary: no item count and no `meta.closed` reaches the ring.
///
/// # Errors
/// Fails when the index cannot be read.
pub fn tend_ring(conn: &Connection, today: i64) -> Result<TendRing, IndexError> {
    let excluded = off_sources(&read_switches(conn)?, content_scan_enabled(conn)?);
    let permitted = DebtSource::ALL
        .into_iter()
        .filter(|s| !excluded.contains(s))
        .map(column_slug)
        .collect::<Result<Vec<_>, _>>()?;
    let mut unobservable_projects = 0_u32;
    let mut all_looked = true;
    let domain = tend_domain(conn)?;
    for &project in &domain {
        let looked = looked_at(conn, project)?;
        let outcome = |source: &String| looked.outcomes.get(source).map(String::as_str);
        if permitted.iter().any(|s| outcome(s) == Some("unobservable")) {
            unobservable_projects += 1;
        }
        all_looked &= !looked.blocking_item
            && permitted
                .iter()
                .all(|s| matches!(outcome(s), Some("complete" | "unobservable")));
    }
    let earned = !tend_days(
        conn,
        DaySpan {
            first: today,
            last: today,
        },
    )?
    .is_empty();
    let status = if earned {
        RingStatus::Closed
    } else if domain.is_empty() {
        RingStatus::NotComputed
    } else if all_looked {
        RingStatus::NothingToTend
    } else {
        RingStatus::Open
    };
    Ok(TendRing {
        status,
        excluded,
        unobservable_projects,
    })
}

/// The days of `span` holding a `T` row: a `debt_day` of a project that is not a Reference and not
/// merged away, which §38 prices above zero.
///
/// Each is dated in its own stored offset, as the ledger dates it. `T` carries none of the Tend
/// domain's suppression or removal clauses: a Seal or a removal later that week withdraws no day
/// the user tended.
///
/// # Errors
/// Fails when SQLite refuses the read.
pub fn tend_days(conn: &Connection, span: DaySpan) -> Result<BTreeSet<i64>, IndexError> {
    let mut stmt = conn.prepare_cached(
        "SELECT x.ts, x.tz_offset_min, x.meta FROM xp_events x JOIN project p ON p.id = x.project_id
          WHERE x.kind = 'debt_day' AND p.is_reference = 0 AND p.merged_into IS NULL
            AND x.ts >= ?1 AND x.ts < ?2",
    )?;
    // Every offset is under a day, so a day either side of the span holds every candidate.
    let from = span.first.saturating_sub(1).saturating_mul(DAY_SECS);
    let to = span.last.saturating_add(2).saturating_mul(DAY_SECS);
    let mut days = BTreeSet::new();
    for row in stmt.query_map([from, to], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, Option<i64>>(1)?,
            r.get::<_, Option<String>>(2)?,
        ))
    })? {
        let (ts, offset_min, meta) = row?;
        let day = ts
            .saturating_add(offset_min.unwrap_or(0).saturating_mul(60))
            .div_euclid(DAY_SECS);
        if (span.first..=span.last).contains(&day) && meta.as_deref().is_none_or(debt_day_priced) {
            days.insert(day);
        }
    }
    Ok(days)
}
