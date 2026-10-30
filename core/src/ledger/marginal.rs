//! §38.11: the marginal price — the payout path run dry.
//!
//! Each row a closure can write has one reading here, a [`RowDue`], and its writer acts on the
//! same reading [`marginal_xp`] prices: a quote and the rows the closure then writes agree, or
//! both are wrong together.

use rusqlite::{Connection, OptionalExtension as _};

use super::pending::awaiting_restore;
use super::price::{completion_gate, debt_day_gate, prices};
use crate::clock::LocalInstant;
use crate::completion::store::slug;
use crate::debt::xp::debt_day_dedupe_key;
use crate::index::subject::subject_for_project;
use crate::index::IndexError;
use crate::protocol::{CompletionCheck, DebtScoring, ProjectId};

/// The rows one closure would write (§41.5 names them; §38 prices them).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Closure {
    /// The scoring of the item the closure fixes, when it fixes one: a `debt_day` for today.
    pub debt_day: Option<DebtScoring>,
    /// The check the closure lights, when it lights one: a `completion_check`.
    pub lights: Option<CompletionCheck>,
    /// Whether the closure commits a card or page verdict: an `amnesty_verdict`.
    pub verdict: bool,
}

/// What a row's writer would do with it now. A gate answers in the same terms: `None` lets the
/// row through, or the gate's [`RowDue::Refused`] or [`RowDue::NotComputable`] is the row's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowDue {
    /// Written now: priced from the table.
    Write {
        /// The project's subject key.
        subject_key: String,
        /// The row's key, which no row holds yet.
        dedupe_key: String,
    },
    /// Already written, as this row — the project's own, or a twin's holding the key it would
    /// write: priced `0`. A `debt_day` writer still updates its sentence.
    Written(i64),
    /// Held until a restored record has had its chance to match (§48.8.4): priced `0`.
    Held,
    /// Refused by a gate: priced `0`.
    Refused,
    /// No subject to key the row on, or a gate that cannot be evaluated: no price.
    NotComputable,
}

/// The XP `closure` would pay `project` if it closed at `at`.
///
/// Each of its rows' price after its latch, key, cap and gate, summed. `None` — not computable,
/// never `0` — when a row has no subject to be keyed on or its gate cannot be evaluated
/// (`authored_by_user IS NULL`).
///
/// Its callers are §41's quest quotes; no surface quotes a verdict, which is priced here all the
/// same.
///
/// # Errors
/// Returns [`IndexError`] when a read fails.
pub fn marginal_xp(
    conn: &Connection,
    project: ProjectId,
    closure: &Closure,
    at: LocalInstant,
) -> Result<Option<u32>, IndexError> {
    let table = prices();
    let mut rows = Vec::with_capacity(3);
    if let Some(scoring) = closure.debt_day {
        let price = if debt_day_pays(scoring) {
            table.debt_day
        } else {
            0
        };
        rows.push((debt_day_due(conn, project, at)?, price));
    }
    if let Some(check) = closure.lights {
        rows.push((
            completion_check_due(conn, project, check)?,
            table.completion_check,
        ));
    }
    if closure.verdict {
        rows.push((verdict_due(conn, project)?, table.amnesty_verdict));
    }
    let mut total = 0_i64;
    for (due, price) in rows {
        match due {
            RowDue::Write { .. } => total = total.saturating_add(price),
            RowDue::Written(_) | RowDue::Held | RowDue::Refused => {}
            RowDue::NotComputable => return Ok(None),
        }
    }
    Ok(Some(u32::try_from(total.max(0)).unwrap_or(u32::MAX)))
}

/// Whether fixing an item of this scoring pays a `debt_day` at all (§38.8.1 gate 1): a `scored`
/// item's, never a `shown_only` one's.
pub(crate) const fn debt_day_pays(scoring: DebtScoring) -> bool {
    matches!(scoring, DebtScoring::Scored)
}

/// A `debt_day` on `at`'s local date (§39's day): held, then the project's gates
/// ([`debt_day_gate`]), then the day's row found by project and date — never by key (§38.2): the
/// subject moves from `path:` to `lineage:` at a first commit and a merge reparents rows without
/// rewriting their keys. A row written before receipts carries no `when`, and is dated from its
/// own instant and offset.
pub(crate) fn debt_day_due(
    conn: &Connection,
    project: ProjectId,
    at: LocalInstant,
) -> Result<RowDue, IndexError> {
    if awaiting_restore(conn, project)? {
        return Ok(RowDue::Held);
    }
    if let Some(refused) = debt_day_gate(conn, project)? {
        return Ok(refused);
    }
    let day: Option<i64> = conn
        .query_row(
            "SELECT id FROM xp_events
              WHERE project_id = ?1 AND kind = 'debt_day'
                AND COALESCE(json_extract(meta, '$.when'),
                             date(ts + COALESCE(tz_offset_min, 0) * 60, 'unixepoch')) = ?2
              ORDER BY id LIMIT 1",
            rusqlite::params![project.0, at.date()],
            |r| r.get(0),
        )
        .optional()?;
    written_or_keyed(conn, project, day, |subject_key| {
        debt_day_dedupe_key(subject_key, &at.date())
    })
}

/// A `completion_check` for `check` (§38.8.2): held, then the latch's gate
/// ([`completion_gate`]), then the project's row for that check, found by project and check —
/// never by key, for the reason [`debt_day_due`] gives.
pub(crate) fn completion_check_due(
    conn: &Connection,
    project: ProjectId,
    check: CompletionCheck,
) -> Result<RowDue, IndexError> {
    if awaiting_restore(conn, project)? {
        return Ok(RowDue::Held);
    }
    if let Some(refused) = completion_gate(conn, project)? {
        return Ok(refused);
    }
    let latched: Option<i64> = conn
        .query_row(
            "SELECT id FROM xp_events
              WHERE project_id = ?1 AND kind = 'completion_check'
                AND json_extract(meta, '$.what') = ?2
              ORDER BY id LIMIT 1",
            rusqlite::params![project.0, slug(&check)],
            |r| r.get(0),
        )
        .optional()?;
    written_or_keyed(conn, project, latched, |subject_key| {
        format!("completion_check:{subject_key}:{}", slug(&check))
    })
}

/// A card or page `amnesty_verdict` (§38.8.4): held, then a subject to key it on, then the
/// project's once-ever award, found by project and `meta.what = 'verdict'`. No gate refuses one:
/// every verdict pays the same.
pub(crate) fn verdict_due(conn: &Connection, project: ProjectId) -> Result<RowDue, IndexError> {
    if awaiting_restore(conn, project)? {
        return Ok(RowDue::Held);
    }
    if subject_for_project(conn, project)?.is_none() {
        return Ok(RowDue::NotComputable);
    }
    let award: Option<i64> = conn
        .query_row(
            "SELECT id FROM xp_events
              WHERE project_id = ?1 AND kind = 'amnesty_verdict'
                AND json_extract(meta, '$.what') = 'verdict'
              ORDER BY id LIMIT 1",
            [project.0],
            |r| r.get(0),
        )
        .optional()?;
    written_or_keyed(conn, project, award, |subject_key| {
        format!("amnesty_verdict:{subject_key}")
    })
}

/// The row already written — found by project, or a twin's holding the key — or a new one keyed
/// on the project's subject, its key built by `key`.
fn written_or_keyed(
    conn: &Connection,
    project: ProjectId,
    found: Option<i64>,
    key: impl FnOnce(&str) -> String,
) -> Result<RowDue, IndexError> {
    if let Some(id) = found {
        return Ok(RowDue::Written(id));
    }
    let Some(subject) = subject_for_project(conn, project)? else {
        return Ok(RowDue::NotComputable);
    };
    let subject_key = subject.to_key();
    let dedupe_key = key(&subject_key);
    let holder: Option<i64> = conn
        .query_row(
            "SELECT id FROM xp_events WHERE dedupe_key = ?1",
            [&dedupe_key],
            |r| r.get(0),
        )
        .optional()?;
    Ok(holder.map_or(
        RowDue::Write {
            subject_key,
            dedupe_key,
        },
        RowDue::Written,
    ))
}
