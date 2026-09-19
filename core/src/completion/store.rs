//! `project_check` — the read, the write-all-ten, and the user's own ruling.
//!
//! **Ten rows or none.** There is no partial row set, and a project holding 1–9 rows is a corrupt
//! state `AC-P3-31-2` asserts against.

use rusqlite::{Connection, Transaction};

use crate::index::IndexError;
use crate::protocol::{CheckState, CompletionCheck, ProjectId, UnknownReason};

use super::evaluate::CheckRow;

/// A generated enum's own wire spelling, read back through serde rather than restated (R24).
fn slug<T: serde::Serialize>(value: &T) -> String {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(raw)) => raw,
        _ => String::new(),
    }
}

fn from_slug<T: serde::de::DeserializeOwned>(raw: &str) -> Option<T> {
    serde_json::from_value(serde_json::Value::String(raw.to_owned())).ok()
}

/// One project's stored rows, in `CompletionCheck` declaration order.
///
/// **0 or 10, never between.** A shorter set is returned as it stands rather than padded: the
/// criterion that asserts the invariant has to be able to see a violation, and a reader that
/// silently filled the gaps would hide exactly the state it exists to catch.
///
/// # Errors
/// Fails when SQLite cannot be read.
pub fn load_rows(conn: &Connection, project: ProjectId) -> Result<Vec<CheckRow>, IndexError> {
    let mut stored = Vec::new();
    {
        let mut st = conn.prepare(
            "SELECT check_key, state, user_na, unknown_reason, observed_at
               FROM project_check WHERE project_id = ?1",
        )?;
        let rows = st.query_map([project.0], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<i64>>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, i64>(4)?,
            ))
        })?;
        for row in rows {
            stored.push(row?);
        }
    }

    let mut out = Vec::with_capacity(stored.len());
    // Declaration order, so every consumer sees the same list without re-sorting. `project_check`
    // is WITHOUT ROWID on `(project_id, check_key)`, which orders by the key's TEXT and not by
    // the enum — two different orders, and the wire contract is the enum's.
    for key in CompletionCheck::ALL {
        let wanted = slug(&key);
        let Some((_, state, user_na, reason, observed_at)) =
            stored.iter().find(|(k, ..)| *k == wanted)
        else {
            continue;
        };
        let Some(state) = from_slug::<CheckState>(state) else {
            // A word this build cannot name was written by a newer one. Refusing is the only
            // honest answer; guessing at a state is how `unknown` becomes a zero.
            return Err(IndexError::Corrupt {
                detail: format!("project_check.state holds {state:?}"),
            });
        };
        out.push(CheckRow {
            key,
            state,
            user_na: user_na.map(|v| v != 0),
            unknown_reason: reason.as_deref().and_then(from_slug::<UnknownReason>),
            observed_at: *observed_at,
        });
    }
    Ok(out)
}

/// Replace this project's whole row set, in the caller's transaction.
///
/// # Errors
/// Fails when SQLite refuses a write.
pub fn write_all_ten(
    tx: &Transaction<'_>,
    project: ProjectId,
    rows: &[CheckRow; 10],
) -> Result<(), IndexError> {
    // Delete-then-insert rather than upsert: the set is fixed at ten and written whole, so an
    // upsert would leave a stale eleventh row alive if the key set ever changed.
    tx.execute(
        "DELETE FROM project_check WHERE project_id = ?1",
        [project.0],
    )?;
    for row in rows {
        tx.execute(
            "INSERT INTO project_check
                (project_id, check_key, state, user_na, unknown_reason, observed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                project.0,
                slug(&row.key),
                slug(&row.state),
                row.user_na.map(i64::from),
                row.unknown_reason.as_ref().map(slug),
                row.observed_at,
            ],
        )?;
    }
    Ok(())
}

/// §31.4's first stored fact: the user's own ruling.
///
/// `None` clears the override and returns the key to the archetype's proposal; `Some(false)` is
/// the user **overriding** a proposal, which is a third value and not the absence of a write.
///
/// It writes `user_na` alone and leaves every other column standing, because the state is
/// recomputed by the evaluator in the same transaction rather than guessed at here.
///
/// # Errors
/// Fails when SQLite refuses a write.
pub fn set_user_na(
    tx: &Transaction<'_>,
    project: ProjectId,
    key: CompletionCheck,
    na: Option<bool>,
) -> Result<(), IndexError> {
    tx.execute(
        "UPDATE project_check SET user_na = ?3 WHERE project_id = ?1 AND check_key = ?2",
        rusqlite::params![project.0, slug(&key), na.map(i64::from)],
    )?;
    Ok(())
}

/// The projection, recounted **from the rows** rather than carried alongside them.
///
/// One owner for the pair, which is what R12 asks for and what makes `AC-P3-31-1`'s recount a
/// test of the writer rather than of a second formatter.
#[must_use]
pub fn recount(rows: &[CheckRow]) -> (u32, u32) {
    let lit = rows.iter().filter(|r| r.state == CheckState::Pass).count();
    let evaluable = rows
        .iter()
        .filter(|r| matches!(r.state, CheckState::Pass | CheckState::Fail))
        .count();
    (
        u32::try_from(lit).unwrap_or(u32::MAX),
        u32::try_from(evaluable).unwrap_or(u32::MAX),
    )
}
