//! §39.3's Focus: which segments count, and the wall-clock union of their intervals.
//!
//! One union serves both readers — the ring's [`focus_seconds`] and the `focus` writer's
//! [`marginal_seconds`] — so the shares a date's closes store add up to the ring's figure for
//! those segments exactly. Two projects launched together are one person's time: the union is
//! taken across projects, never summed per project.
//!
//! Playtime (§9) is not read here. It stays the per-project sum of all credit, launch-only
//! included ([`super::store::playtime_seconds`]): one source, two values, two owners. Nothing
//! here names a table but `session`, `session_segment` and `project`.

use rusqlite::{Connection, OptionalExtension as _};

use crate::clock::{day_bounds, LocalZone};
use crate::session::{SessionError, SEGMENT_IDLE_SECS};

/// One local date's share of a closing segment (§39.3's `m(s, d)`): the seconds of that date no
/// other closed counted segment already covers, and the first of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DatedContribution {
    /// The local day number, days since 1970-01-01 in the zone in force (as [`day_bounds`]).
    pub day: i64,
    /// Seconds the segment adds to that date's Focus; never `0` — a date it adds nothing to is
    /// absent.
    pub seconds: i64,
    /// The first of those seconds, in epoch seconds.
    pub first_instant: i64,
}

/// Whether a segment is in `C` by how it was opened and touched (§39.3's table).
///
/// `Some` is the stored flag. `None` is a row written before the column: counted when it is not
/// its session's first segment — every later one is opened by a signal — or when it ran past the
/// idle tail, which only a signal extends.
#[must_use]
pub const fn is_counted(
    signalled: Option<bool>,
    first_in_session: bool,
    credited_seconds: i64,
) -> bool {
    match signalled {
        Some(signalled) => signalled,
        None => !first_in_session || credited_seconds > SEGMENT_IDLE_SECS,
    }
}

/// `Focus(day)`: the seconds of local day `day` inside the union of the counted segments.
///
/// Each segment's interval is its credit (§39.3); an open segment's runs to its watermark.
/// Nothing after `now` is counted: a segment stamped past the reading, by a wall clock moved
/// back, has not happened yet.
///
/// # Errors
/// `SessionError::Sqlite` when the segments cannot be read.
pub fn focus_seconds(
    conn: &Connection,
    zone: &dyn LocalZone,
    day: i64,
    now: i64,
) -> Result<i64, SessionError> {
    let (start, end) = day_bounds(zone, day);
    let end = end.min(now);
    if start >= end {
        return Ok(0);
    }
    let spans = union(
        counted(conn, start, end)?
            .iter()
            .map(|c| (c.start.max(start), c.end.min(end))),
    );
    Ok(spans.iter().map(|&(from, to)| to - from).sum())
}

/// §39.3's marginal measure for `segment_id`: for each local date its interval touches, the
/// seconds of that date it covers that no other closed counted segment does.
///
/// Called as the segment closes, the other closed segments are exactly those closed before it,
/// so the shares a date's closes store telescope to `Focus(d)` over them, whatever the close
/// order. A segment not in `C` — launch-only, or a Reference project's — has no share at all.
///
/// # Errors
/// `SessionError::Sqlite` when the segments cannot be read.
pub fn marginal_seconds(
    conn: &Connection,
    zone: &dyn LocalZone,
    segment_id: i64,
) -> Result<Vec<DatedContribution>, SessionError> {
    let interval: Option<(i64, i64)> = conn
        .query_row(
            "SELECT started_at, started_at + credited_seconds FROM session_segment WHERE id = ?1",
            [segment_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some((start, end)) = interval else {
        return Ok(Vec::new());
    };
    let segments = counted(conn, start, end)?;
    if !segments.iter().any(|c| c.id == segment_id) {
        return Ok(Vec::new());
    }
    let cover = union(
        segments
            .iter()
            .filter(|c| c.closed && c.id != segment_id)
            .map(|c| (c.start, c.end)),
    );
    let mut shares = Vec::new();
    let mut day = zone.at(start).day();
    loop {
        let (day_start, day_end) = day_bounds(zone, day);
        if let Some((seconds, first_instant)) =
            uncovered(start.max(day_start), end.min(day_end), &cover)
        {
            shares.push(DatedContribution {
                day,
                seconds,
                first_instant,
            });
        }
        if day_end >= end {
            return Ok(shares);
        }
        day = day.saturating_add(1);
    }
}

/// A counted segment's interval `[start, end)`.
struct Counted {
    id: i64,
    start: i64,
    end: i64,
    closed: bool,
}

/// The segments in `C` whose interval meets `[from, to)`: a non-Reference project's at the
/// reading, counted by [`is_counted`]. A session's first segment is its earliest-started one.
fn counted(conn: &Connection, from: i64, to: i64) -> Result<Vec<Counted>, SessionError> {
    let mut stmt = conn.prepare_cached(
        "SELECT g.id, g.started_at, g.started_at + g.credited_seconds, g.closed_by IS NOT NULL,
                g.signalled, g.credited_seconds,
                NOT EXISTS (SELECT 1 FROM session_segment f
                             WHERE f.session_id = g.session_id
                               AND (f.started_at, f.id) < (g.started_at, g.id))
           FROM session_segment g
           JOIN session s ON s.id = g.session_id
           JOIN project p ON p.id = s.project_id
          WHERE p.is_reference = 0
            AND g.started_at < ?2 AND g.started_at + g.credited_seconds > ?1",
    )?;
    let rows = stmt.query_map([from, to], |r| {
        let signalled: Option<i64> = r.get(4)?;
        let counts = is_counted(signalled.map(|v| v != 0), r.get(6)?, r.get(5)?);
        Ok(counts.then_some(Counted {
            id: r.get(0)?,
            start: r.get(1)?,
            end: r.get(2)?,
            closed: r.get(3)?,
        }))
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.extend(row?);
    }
    Ok(out)
}

/// The union of half-open spans as disjoint spans in order; an empty span vanishes.
fn union(spans: impl Iterator<Item = (i64, i64)>) -> Vec<(i64, i64)> {
    let mut spans: Vec<(i64, i64)> = spans.filter(|&(from, to)| from < to).collect();
    spans.sort_unstable();
    let mut out: Vec<(i64, i64)> = Vec::with_capacity(spans.len());
    for (from, to) in spans {
        match out.last_mut() {
            Some(last) if from <= last.1 => last.1 = last.1.max(to),
            _ => out.push((from, to)),
        }
    }
    out
}

/// The seconds of `[from, to)` outside `cover` (disjoint, in order) and the first of them, or
/// `None` when `cover` leaves none.
fn uncovered(from: i64, to: i64, cover: &[(i64, i64)]) -> Option<(i64, i64)> {
    let mut seconds = 0;
    let mut first = None;
    let mut at = from;
    for &(covered_from, covered_to) in cover {
        if at >= to || covered_from >= to {
            break;
        }
        if covered_to <= at {
            continue;
        }
        if covered_from > at {
            seconds += covered_from - at;
            first.get_or_insert(at);
        }
        at = at.max(covered_to);
    }
    if at < to {
        seconds += to - at;
        first.get_or_insert(at);
    }
    first.map(|first| (seconds, first))
}
