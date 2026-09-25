//! §28.4 — `debt_day`: **one `xp_events` row per project per local date.**
//!
//! **NEVER REWARD VOLUME.** Deleting one file holding forty markers is one keystroke, and a
//! per-item key pays it 40×. The bound is a UNIQUE constraint on `dedupe_key`, not a cap bolted
//! on afterwards.
//!
//! **`track = 'session'` is not a compromise and must not be "fixed" to a new value.** `track`
//! means *recomputable / not recomputable*, and `'session'` is the name that class already
//! carries. A new value such as `'observed'` would be silently dropped by the sidecar's export
//! filter (`core/src/index/sidecar.rs:322-327`) and mislabelled by the restore's hard-coded
//! literal (`:755-758`), so a rebuild-from-sidecar would **take XP away** — the exact promise
//! `level_floor` exists to keep. The accepted cost, stated so nobody removes it: `track` reads
//! `'session'` for a row whose kind is not a session.

use std::collections::BTreeSet;

use rusqlite::{OptionalExtension as _, Transaction};

use super::store::{DebtCloseReason, SweepEffect};
use super::{enum_from_text, enum_text, DebtError};
use crate::git::local_day;
use crate::jobs::j4_history::local_date;
use crate::protocol::{DebtSource, ProjectId};

/// What one call did, so the caller can report a day without re-reading it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DebtDayPayout {
    /// True only when this call inserted the day's row. The payout is the row's existence.
    pub wrote_row: bool,
    /// Every `Fixed` closure of this project on this local date, including earlier calls'.
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
/// **`Invalidated` closures are excluded from the count and from `sources` before the row is
/// written**, and a day whose only closures are invalidated writes **no row at all**.
///
/// # Errors
/// Fails when SQLite refuses the read or the write, the day's stored `meta` is not JSON, or it
/// names a source this build's schema does not declare.
pub fn pay_debt_day(
    tx: &Transaction<'_>,
    project: ProjectId,
    subject_key: &str,
    effect: &SweepEffect,
    now: i64,
    tz_offset_min: i32,
) -> Result<DebtDayPayout, DebtError> {
    let paid: Vec<DebtSource> = effect
        .closed
        .iter()
        .filter(|(_, reason)| *reason == DebtCloseReason::Fixed)
        .map(|(key, _)| key.source)
        .collect();

    if paid.is_empty() {
        return Ok(DebtDayPayout {
            wrote_row: false,
            closed_today: 0,
            sources: Vec::new(),
        });
    }

    let date = local_date(local_day(now, tz_offset_min));
    let dedupe = debt_day_dedupe_key(subject_key, &date);

    // The day so far, read back before the merge: `meta` is the day's sentence and a sentence
    // describing only the first closure undercounts the day it claims to describe.
    let existing: Option<String> = tx
        .query_row(
            "SELECT meta FROM xp_events WHERE dedupe_key = ?1",
            [&dedupe],
            |r| r.get(0),
        )
        .optional()?;

    // The generated enums derive no `Ord` — they are wire vocabularies, not ordered ones — so
    // the day's source set is held as its wire spelling, which is also the order `meta` reads in.
    let mut closed_today = u32::try_from(paid.len()).unwrap_or(u32::MAX);
    let mut source_text: BTreeSet<String> = BTreeSet::new();
    for source in &paid {
        source_text.insert(enum_text(source)?);
    }
    if let Some(raw) = existing.as_deref() {
        let previous: serde_json::Value = serde_json::from_str(raw)
            .map_err(|e| DebtError::Codec(format!("xp_events.meta is not JSON: {e}")))?;
        if let Some(n) = previous.get("closed").and_then(serde_json::Value::as_u64) {
            closed_today = closed_today.saturating_add(u32::try_from(n).unwrap_or(u32::MAX));
        }
        if let Some(list) = previous
            .get("sources")
            .and_then(serde_json::Value::as_array)
        {
            for value in list {
                if let Some(spelling) = value.as_str() {
                    source_text.insert(spelling.to_owned());
                }
            }
        }
    }

    let mut sources = Vec::with_capacity(source_text.len());
    for raw in &source_text {
        sources.push(
            enum_from_text::<DebtSource>(raw)
                .ok_or_else(|| DebtError::Codec(format!("xp_events.meta names {raw:?}")))?,
        );
    }
    let meta = serde_json::json!({
        "sources": source_text.iter().collect::<Vec<_>>(),
        "closed": closed_today,
    })
    .to_string();

    // **`ts`, `tz_offset_min`, `kind`, `track`, `subject_key` and `dedupe_key` are never
    // updated.** The payout is the row's existence and that never changes; only the day's
    // sentence moves. `ts` is the closing observation's own moment and not a synthetic midnight,
    // because a debt transition happens at a time.
    let inserted = tx.execute(
        "INSERT INTO xp_events
            (ts, tz_offset_min, project_id, subject_key, kind, dedupe_key, track, meta)
         VALUES (?1, ?2, ?3, ?4, 'debt_day', ?5, 'session', ?6)
         ON CONFLICT(dedupe_key) DO UPDATE SET meta = excluded.meta",
        rusqlite::params![now, tz_offset_min, project.0, subject_key, dedupe, meta],
    )?;

    Ok(DebtDayPayout {
        wrote_row: existing.is_none() && inserted > 0,
        closed_today,
        sources,
    })
}
