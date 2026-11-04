//! §43.4's reason: what a rise is owed to, read from the ledger rows written since the level
//! was last shown. **A reason is never fabricated** — a rise no counted row explains carries none.

use rusqlite::Transaction;

use crate::index::IndexError;
use crate::ledger::price::is_revival_gap;
use crate::ledger::xp::language_firsts;
use crate::protocol::{MotivationReason, ProjectId, XpTerm};

const DAY: i64 = 86_400;

/// The rows a reason counts, for `xp_events` aliased `x`: neither silent nor backfilled. The one
/// spelling, spliced into every statement that asks whether a row could make or name a line.
pub(crate) const COUNTED_ROW_SQL: &str = "json_extract(x.meta, '$.silent') IS NOT 1
                AND json_extract(x.meta, '$.backfilled') IS NOT 1";

/// One ledger row the reason counts.
struct Counted {
    kind: String,
    project: Option<i64>,
    ts: i64,
    /// `meta.project_name`, the name as of the write.
    receipt_name: Option<String>,
    /// The project's name now.
    live_name: Option<String>,
    /// `meta.when`, the row's local date.
    when: Option<String>,
}

/// The reason a rise is owed to, read from the ledger rows after `since`.
///
/// Only rows whose receipt marks them neither silent nor backfilled count. One row names its
/// term, its project and its date; several are counted; none, or no mark at all, is no reason.
///
/// A `commit_day` names its project's current name, because its receipt keeps only the date;
/// every other kind names the project as its receipt recorded it.
///
/// # Errors
/// Fails when the index cannot be read, or a counted row holds a kind this build does not price.
pub fn reason_since(
    tx: &Transaction<'_>,
    since: Option<i64>,
) -> Result<Option<MotivationReason>, IndexError> {
    let Some(since) = since else {
        return Ok(None);
    };
    let rows = tx
        .prepare(&format!(
            "SELECT x.kind, x.project_id, x.ts,
                    json_extract(x.meta, '$.project_name'), p.name,
                    json_extract(x.meta, '$.when')
               FROM xp_events x
               LEFT JOIN project p ON p.id = x.project_id
              WHERE x.id > ?1 AND {COUNTED_ROW_SQL}"
        ))?
        .query_map([since], |r| {
            Ok(Counted {
                kind: r.get(0)?,
                project: r.get(1)?,
                ts: r.get(2)?,
                receipt_name: r.get(3)?,
                live_name: r.get(4)?,
                when: r.get(5)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let [one] = rows.as_slice() else {
        return Ok((!rows.is_empty()).then(|| MotivationReason {
            transitions: u32::try_from(rows.len()).unwrap_or(u32::MAX),
            term: None,
            project_name: None,
            on: None,
        }));
    };
    let term = term_of(tx, one)?;
    let project_name = if one.kind == "commit_day" {
        one.live_name.clone()
    } else {
        one.receipt_name.clone()
    };
    Ok(Some(MotivationReason {
        transitions: 1,
        term: Some(term),
        project_name,
        on: one.when.clone(),
    }))
}

/// The XP term a row is paid under. A `commit_day` is a revival when it ends its project's
/// dormancy, a language when it dates a language's first appearance, and otherwise a commit-day.
fn term_of(tx: &Transaction<'_>, row: &Counted) -> Result<XpTerm, IndexError> {
    Ok(match row.kind.as_str() {
        "commit_day" => commit_day_term(tx, row)?,
        "release" => XpTerm::Release,
        "focus" => XpTerm::Focus,
        "debt_day" => XpTerm::DebtDay,
        "completion_check" => XpTerm::CompletionCheck,
        "amnesty_verdict" => XpTerm::AmnestyVerdict,
        "clean_archive" => XpTerm::CleanArchive,
        other => {
            return Err(IndexError::Corrupt {
                detail: format!("xp_events holds a kind no term prices: {other}"),
            })
        }
    })
}

fn commit_day_term(tx: &Transaction<'_>, row: &Counted) -> Result<XpTerm, IndexError> {
    let Some(project) = row.project else {
        return Ok(XpTerm::CommitDay);
    };
    let day = row.ts.div_euclid(DAY);
    let previous: Option<i64> = tx.query_row(
        "SELECT MAX(ts) FROM xp_events
          WHERE kind = 'commit_day' AND project_id = ?1 AND ts < ?2",
        rusqlite::params![project, row.ts],
        |r| r.get(0),
    )?;
    if previous.is_some_and(|prev| is_revival_gap(prev.div_euclid(DAY), day)) {
        return Ok(XpTerm::Revival);
    }
    let first_of_a_language = language_firsts(tx)?
        .iter()
        .any(|first| first.project_id == ProjectId(project) && first.day == day);
    Ok(if first_of_a_language {
        XpTerm::Language
    } else {
        XpTerm::CommitDay
    })
}
