//! §28.4 — `debt_day`: **one `xp_events` row per project per local date.**
//!
//! **NEVER REWARD VOLUME.** Deleting one file holding forty markers is one keystroke, and a
//! per-item key pays it 40×. The bound is a UNIQUE constraint on `dedupe_key`, not a cap bolted
//! on afterwards.
//!
//! **`track = 'session'` is not a compromise and must not be "fixed" to a new value.** `track`
//! means *recomputable / not recomputable*, and `'session'` is the name that class already
//! carries. A new value such as `'observed'` would be silently dropped by the sidecar's export
//! filter (`core/src/index/sidecar.rs:464-469`) and mislabelled by the restore's hard-coded
//! literal (`:926-929`), so a rebuild-from-sidecar would **take XP away** — the exact promise
//! `level_floor` exists to keep. The accepted cost, stated so nobody removes it: `track` reads
//! `'session'` for a row whose kind is not a session.

use std::collections::BTreeSet;

use rusqlite::Transaction;

use super::store::{DebtCloseReason, DebtClosure, SweepEffect};
use super::{enum_from_text, enum_text, DebtError};
use crate::clock::LocalInstant;
use crate::ledger::marginal::{debt_day_due, debt_day_pays, RowDue};
use crate::ledger::receipt::Receipt;
use crate::ledger::retained::{insert_retained, Retained, RetainedRow};
use crate::protocol::{DebtSource, ProjectId};

/// A call that wrote nothing: no paying closure, or a project the gates refuse.
const fn unpaid() -> DebtDayPayout {
    DebtDayPayout {
        wrote_row: false,
        closed_today: 0,
        sources: Vec::new(),
    }
}

/// What one call did, so the caller can report a day without re-reading it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DebtDayPayout {
    /// True only when this call inserted the day's row. The payout is the row's existence.
    pub wrote_row: bool,
    /// Every paying closure — `Fixed`, of a `scored` item — of this project on this local date,
    /// including earlier calls'.
    pub closed_today: u32,
    /// The day's sources, sorted and deduplicated.
    pub sources: Vec<DebtSource>,
}

/// §28.4's idempotency key, `debt_day:<subject_key>:<local-date>`.
///
/// **`<subject_key>`, and not §1.7's `<lineage_key>:<remote_key>`.** That shape is safe for
/// `commit_day` only because a project with no lineage has no commits —
/// `core/src/jobs/j4_history.rs` returns early on `None`. **A project with no lineage can
/// absolutely have markers**, and §1.7's shape would collapse every such project onto
/// `debt_day:::<date>`, where this column's UNIQUE plus `ON CONFLICT DO NOTHING` turns the
/// collision into **silent non-payment** rather than an error.
///
/// The only place the string is built.
#[must_use]
pub fn debt_day_dedupe_key(subject_key: &str, local_date: &str) -> String {
    format!("debt_day:{subject_key}:{local_date}")
}

/// Pay at most one row for this project on this local date, in the **caller's** transaction.
///
/// The ledger row and the item deletions commit together: a tree with the items gone and no
/// payout, or a payout with the items still open, is the state the ordering exists to prevent.
///
/// **A closure pays only when it is `Fixed` and its item was `scored` when it closed** (§38.8.1
/// gate 1, §28.6). Every other closure — `Invalidated`, or of a `shown_only` item — is excluded
/// from the count and from `sources` before the row is written, and a day with no paying closure
/// writes **no row at all**.
///
/// **The day is read through `debt_day_due`, the one reading the marginal price quotes too**:
/// nothing while a restored record may still match the project (§48.8.4) — the restore brings
/// the old day back, never a second one; then the project's gates (§38.8.1 gate 2): a subject to
/// key the row on, enrolled (§30.5's `is_enrolled`), and not Reference (§38.3a), read at the
/// closing observation. A refused closure leaves nothing behind that a later call could pay. The
/// subject is read from the project row, so no caller can hand the writer a key that is not the
/// project's own.
///
/// **The day is `at`'s own local date** (§39.2 rule 5): the caller frames its closing instant in
/// the zone in force at it, and the row stores that instant and that offset. The day's row is
/// found by project and that date, never by key (§38.2), and its `meta` carries the receipt.
///
/// # Errors
/// Fails when SQLite refuses the read or the write, the day's stored `meta` is not JSON, or it
/// names a source this build's schema does not declare.
pub fn pay_debt_day(
    tx: &Transaction<'_>,
    project: ProjectId,
    effect: &SweepEffect,
    at: LocalInstant,
) -> Result<DebtDayPayout, DebtError> {
    let paying: Vec<&DebtClosure> = effect
        .closed
        .iter()
        .filter(|c| c.reason == DebtCloseReason::Fixed && debt_day_pays(c.scoring))
        .collect();
    let paid: Vec<DebtSource> = paying.iter().map(|c| c.key.source).collect();

    if paid.is_empty() {
        return Ok(unpaid());
    }
    // The generated enums derive no `Ord` — they are wire vocabularies, not ordered ones — so
    // the day's source set is held as its wire spelling, which is also the order `meta` reads in.
    let mut closed_today = u32::try_from(paid.len()).unwrap_or(u32::MAX);
    let mut source_text: BTreeSet<String> = BTreeSet::new();
    for source in &paid {
        source_text.insert(enum_text(source)?);
    }
    // §40.10: a paid closure whose item was first seen on this payout's own local date is a
    // same-day repair. The day was framed by the sweep before the item's row was deleted.
    let mut same_day: BTreeSet<String> = BTreeSet::new();
    for closure in paying.iter().filter(|c| c.first_seen_day == at.day()) {
        same_day.insert(enum_text(&closure.key.source)?);
    }

    // The day so far — found by project and local date, never by key (§38.2), or a twin's day
    // holding the key this one would write.
    let day = match debt_day_due(tx, project, at)? {
        RowDue::Write {
            subject_key,
            dedupe_key,
        } => {
            let meta = day_meta(tx, project, at, &source_text, &same_day, closed_today)?;
            match insert_retained(
                tx,
                &RetainedRow {
                    at,
                    project,
                    subject_key: &subject_key,
                    kind: "debt_day",
                    dedupe_key: &dedupe_key,
                    meta: &meta,
                },
            )? {
                Retained::Inserted(_) => return payout(true, closed_today, &source_text),
                Retained::Held(id) => id,
            }
        }
        RowDue::Written(id) => id,
        RowDue::Held | RowDue::Refused | RowDue::NotComputable => return Ok(unpaid()),
    };

    // `meta` is read back before the merge because a sentence describing only the latest closure
    // undercounts the day it describes.
    let raw: String = tx.query_row("SELECT meta FROM xp_events WHERE id = ?1", [day], |r| {
        r.get(0)
    })?;
    let previous: serde_json::Value = serde_json::from_str(&raw)
        .map_err(|e| DebtError::Codec(format!("xp_events.meta is not JSON: {e}")))?;
    if let Some(n) = previous.get("closed").and_then(serde_json::Value::as_u64) {
        closed_today = closed_today.saturating_add(u32::try_from(n).unwrap_or(u32::MAX));
    }
    for (key, set) in [("sources", &mut source_text), ("same_day", &mut same_day)] {
        if let Some(list) = previous.get(key).and_then(serde_json::Value::as_array) {
            for value in list {
                if let Some(spelling) = value.as_str() {
                    set.insert(spelling.to_owned());
                }
            }
        }
    }

    // **`ts`, `tz_offset_min`, `kind`, `track`, `subject_key` and `dedupe_key` are never
    // updated.** The payout is the row's existence and that never changes; only the day's
    // sentence moves, on the lowest id of the day. `ts` is the closing observation's own moment
    // and not a synthetic midnight, because a debt transition happens at a time.
    let meta = day_meta(tx, project, at, &source_text, &same_day, closed_today)?;
    tx.execute(
        "UPDATE xp_events SET meta = ?2 WHERE id = ?1",
        rusqlite::params![day, meta],
    )?;
    payout(false, closed_today, &source_text)
}

/// The day's receipt: its sources, its same-day repairs' sources — always written, an empty
/// list when none — and how many paying closures it holds.
fn day_meta(
    tx: &Transaction<'_>,
    project: ProjectId,
    at: LocalInstant,
    source_text: &BTreeSet<String>,
    same_day: &BTreeSet<String>,
    closed_today: u32,
) -> Result<String, DebtError> {
    let project_name: String =
        tx.query_row("SELECT name FROM project WHERE id = ?1", [project.0], |r| {
            r.get(0)
        })?;
    Ok(Receipt::day(at, &project_name)
        .extra("sources", source_text.iter().cloned().collect::<Vec<_>>())
        .extra("closed", closed_today)
        .extra("same_day", same_day.iter().cloned().collect::<Vec<_>>())
        .to_meta())
}

/// The call's report, its sources read back from their wire spelling.
fn payout(
    wrote_row: bool,
    closed_today: u32,
    source_text: &BTreeSet<String>,
) -> Result<DebtDayPayout, DebtError> {
    let mut sources = Vec::with_capacity(source_text.len());
    for raw in source_text {
        sources.push(
            enum_from_text::<DebtSource>(raw)
                .ok_or_else(|| DebtError::Codec(format!("xp_events.meta names {raw:?}")))?,
        );
    }
    Ok(DebtDayPayout {
        wrote_row,
        closed_today,
        sources,
    })
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
mod tests {
    use super::*;
    use crate::debt::identity::DebtKey;
    use crate::protocol::DebtScoring;

    /// 2024-01-01, a local day number.
    const DAY: i64 = 19_723;

    fn closure(source: DebtSource, first_seen_day: i64) -> DebtClosure {
        DebtClosure {
            key: DebtKey::singleton("lineage:p|remote:", source),
            reason: DebtCloseReason::Fixed,
            scoring: DebtScoring::Scored,
            first_seen_day,
        }
    }

    fn effect(closed: Vec<DebtClosure>) -> SweepEffect {
        SweepEffect {
            closed,
            ..SweepEffect::default()
        }
    }

    /// §40.10: two payouts on one local date merge `same_day` as a set — each paid closure whose
    /// item was first seen that date adds its source once, and one first seen earlier adds none.
    #[test]
    fn two_payouts_on_one_date_merge_same_day_as_a_set() {
        let dir = tempfile::tempdir().unwrap();
        let mut index = crate::index::Index::open_at(dir.path(), DAY * 86_400).unwrap();
        let at = |hour: i64| LocalInstant {
            unix: DAY * 86_400 + hour * 3_600,
            offset_min: 0,
        };
        let meta: String = index
            .with_tx(|tx| {
                tx.execute(
                    "INSERT INTO project (name, seed_basename, lineage_key, acknowledged_at,
                                          created_at, updated_at)
                     VALUES ('p', 'p', 'p', 1, 0, 0)",
                    [],
                )?;
                let project = ProjectId(tx.last_insert_rowid());
                let first = effect(vec![
                    closure(DebtSource::CiRed, DAY),
                    closure(DebtSource::MissingReadme, DAY - 1),
                ]);
                pay_debt_day(tx, project, &first, at(9)).unwrap();
                let second = effect(vec![
                    closure(DebtSource::TodoMarker, DAY),
                    closure(DebtSource::CiRed, DAY),
                ]);
                pay_debt_day(tx, project, &second, at(15)).unwrap();
                Ok(tx.query_row(
                    "SELECT meta FROM xp_events WHERE kind = 'debt_day'",
                    [],
                    |r| r.get(0),
                )?)
            })
            .unwrap();
        let meta: serde_json::Value = serde_json::from_str(&meta).unwrap();
        eprintln!("the day's meta: {meta}");
        assert_eq!(
            meta["same_day"],
            serde_json::json!(["ci_red", "todo_marker"])
        );
        assert_eq!(
            meta["sources"],
            serde_json::json!(["ci_red", "missing_readme", "todo_marker"])
        );
    }
}
