//! §40.8's landing: the one award due on a local date, the one writer that records it shown, and
//! the list every surface reads.
//!
//! Retro and live are derived at read and never stored. The core, not the renderer, enforces one
//! landing per local date, the order and the fortnight: a badge the renderer shows that [`due`]
//! did not name has been shown and has not landed. Quiet mode is not an input — nothing shown
//! means nothing lands, and `due` computes identically.

use rusqlite::{Connection, OptionalExtension as _, Transaction};

use crate::clock::parse_local_date;
use crate::index::IndexError;
use crate::motivation::keys::{self, KEY_MOTIVATION_DAY_ONE};
use crate::protocol::{BadgeAward, BadgeKind, ProjectId};

use super::evaluate::EvalCtx;
use super::store::kind_of;
use super::{evidence, kind_text, BadgeError, LANDING_ORDER, TRICKLE_DAYS};

/// The only kinds day one may land, in the order it lands them: nothing else lands on day one.
const DAY_ONE_KINDS: [BadgeKind; 3] = [
    BadgeKind::Necromancer,
    BadgeKind::Polyglot,
    BadgeKind::Comeback,
];

/// What [`mark_landed`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkLanded {
    /// The award was today's `due`, and its landing is now recorded.
    Written,
    /// Nothing was written: the award had landed already, or it is not today's `due`.
    NoWrite,
}

/// One award as the landing rules read it, its dates as day numbers.
struct Row {
    kind: BadgeKind,
    earned_at: i64,
    earned_on: i64,
    awarded_on: i64,
    landed_on: Option<i64>,
}

/// `kind`'s place in [`LANDING_ORDER`], which names every kind once.
fn rank(kind: BadgeKind) -> usize {
    LANDING_ORDER
        .iter()
        .position(|k| *k == kind)
        .unwrap_or(LANDING_ORDER.len())
}

fn day_of(kind: BadgeKind, column: &str, text: &str) -> Result<i64, IndexError> {
    parse_local_date(text).ok_or_else(|| IndexError::Corrupt {
        detail: format!(
            "badge_award {} holds a {column} that is not a local date: {text}",
            kind_text(kind)
        ),
    })
}

/// Day one as a day number; `None` while §43 has not written it.
fn day_one(conn: &Connection) -> Result<Option<i64>, IndexError> {
    keys::day_one(conn)?
        .map(|text| {
            parse_local_date(&text).ok_or_else(|| IndexError::Corrupt {
                detail: format!("{KEY_MOTIVATION_DAY_ONE} is not a local date: {text}"),
            })
        })
        .transpose()
}

fn rows(conn: &Connection) -> Result<Vec<Row>, IndexError> {
    let mut st = conn.prepare_cached(
        "SELECT kind, earned_at, earned_on, awarded_on, landed_on FROM badge_award",
    )?;
    let raw = st
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, Option<String>>(4)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    raw.into_iter()
        .map(|(kind, earned_at, earned_on, awarded_on, landed_on)| {
            let kind = kind_of(&kind)?;
            Ok(Row {
                kind,
                earned_at,
                earned_on: day_of(kind, "earned_on", &earned_on)?,
                awarded_on: day_of(kind, "awarded_on", &awarded_on)?,
                landed_on: landed_on
                    .map(|text| day_of(kind, "landed_on", &text))
                    .transpose()?,
            })
        })
        .collect()
}

/// The one award to land on the local date of `ctx.now`, if any — §40.8's six rules, first match
/// wins. Read by `badges.list`, by [`mark_landed`], and by any surface that shows a badge as a
/// moment.
///
/// # Errors
/// Fails when SQLite refuses the read, or a stored kind or date is not one this build reads.
pub fn due(conn: &Connection, ctx: &EvalCtx<'_>) -> Result<Option<BadgeKind>, BadgeError> {
    // 1. Nothing is due before day one, or while it is unwritten.
    let Some(first) = day_one(conn)? else {
        return Ok(None);
    };
    let today = ctx.zone.at(ctx.now).day();
    if today < first {
        return Ok(None);
    }
    let rows = rows(conn)?;
    // 2. At most one landing per local date.
    if rows.iter().any(|r| r.landed_on == Some(today)) {
        return Ok(None);
    }
    let last = first + TRICKLE_DAYS - 1;
    let retro = |r: &Row| r.awarded_on <= last && (r.earned_on < first || r.awarded_on <= first);
    let first_retro_of = |kinds: &[BadgeKind]| {
        kinds.iter().copied().find(|kind| {
            rows.iter()
                .any(|r| r.kind == *kind && r.landed_on.is_none() && retro(r))
        })
    };
    // 3. Day one lands only the first of its three, or nothing.
    if today == first {
        return Ok(first_retro_of(&DAY_ONE_KINDS));
    }
    // 4. A live award is news: the earliest earned, ties by the landing order.
    if let Some(live) = rows
        .iter()
        .filter(|r| r.landed_on.is_none() && !retro(r))
        .min_by_key(|r| (r.earned_at, rank(r.kind)))
    {
        return Ok(Some(live.kind));
    }
    // 5. Inside the fortnight, the retro awards trickle in the landing order.
    if today <= last {
        return Ok(first_retro_of(&LANDING_ORDER));
    }
    // 6. A retro award left after the fortnight stays listed and is never announced.
    Ok(None)
}

/// Record `kind` shown as a moment at `ctx.now` — the only writer of `landed_at` and `landed_on`.
///
/// A landed award, or one that is not today's [`due`], is answered [`MarkLanded::NoWrite`] and
/// left as it is, so a replay is safe.
///
/// # Errors
/// [`BadgeError::NotAwarded`] when `kind` holds no award; otherwise fails when SQLite refuses the
/// read or the write.
pub fn mark_landed(
    tx: &Transaction<'_>,
    kind: BadgeKind,
    ctx: &EvalCtx<'_>,
) -> Result<MarkLanded, BadgeError> {
    let landed_at: Option<Option<i64>> = tx
        .query_row(
            "SELECT landed_at FROM badge_award WHERE kind = ?1",
            [kind_text(kind)],
            |r| r.get(0),
        )
        .optional()
        .map_err(IndexError::from)?;
    match landed_at {
        None => Err(BadgeError::NotAwarded(kind)),
        Some(None) if due(tx, ctx)? == Some(kind) => {
            tx.execute(
                "UPDATE badge_award SET landed_at = ?2, landed_on = ?3 WHERE kind = ?1",
                rusqlite::params![kind_text(kind), ctx.now, ctx.zone.at(ctx.now).date()],
            )
            .map_err(IndexError::from)?;
            Ok(MarkLanded::Written)
        }
        // Landed already, or not today's `due`.
        Some(_) => Ok(MarkLanded::NoWrite),
    }
}

/// Every `badge_award` row as `(kind, earned_on, project_id, evidence, landed)`, undecoded.
type StoredAward = (String, String, Option<i64>, String, bool);

fn stored_awards(conn: &Connection) -> Result<Vec<StoredAward>, IndexError> {
    let mut st = conn.prepare_cached(
        "SELECT kind, earned_on, project_id, evidence, landed_at IS NOT NULL FROM badge_award",
    )?;
    let raw = st
        .query_map([], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(raw)
}

/// The kind most recently landed — the latest `landed_at` — or `None` while nothing has landed.
/// §43's line reads it: a landed award stays a line until its click is acknowledged.
///
/// # Errors
/// Fails when SQLite refuses the read, or the stored kind is not one this build reads.
pub fn last_landed(conn: &Connection) -> Result<Option<BadgeKind>, IndexError> {
    let newest: Option<String> = conn
        .query_row(
            "SELECT kind FROM badge_award WHERE landed_at IS NOT NULL
              ORDER BY landed_at DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .optional()?;
    newest.as_deref().map(kind_of).transpose()
}

/// Every award, by `earnedOn` then the landing order.
///
/// Evidence is decoded strictly: a row that does not decode fails the read naming its kind, never
/// a silently dropped award. Awarded rows only — no unearned kind, threshold, run or count.
///
/// # Errors
/// Fails when SQLite refuses the read, or a stored kind or evidence document is not one this
/// build reads.
pub fn list_awards(conn: &Connection) -> Result<Vec<BadgeAward>, BadgeError> {
    let raw = stored_awards(conn)?;
    let mut awards = raw
        .into_iter()
        .map(|(kind, earned_on, project, stored, landed)| {
            let kind = kind_of(&kind)?;
            let evidence = evidence::decode(&stored)
                .map_err(|e| BadgeError::Evidence(format!("the {} award: {e}", kind_text(kind))))?;
            Ok(BadgeAward {
                kind,
                earned_on,
                project_id: project.map(ProjectId),
                evidence,
                landed,
            })
        })
        .collect::<Result<Vec<_>, BadgeError>>()?;
    awards.sort_by(|a, b| {
        a.earned_on
            .cmp(&b.earned_on)
            .then_with(|| rank(a.kind).cmp(&rank(b.kind)))
    });
    Ok(awards)
}
