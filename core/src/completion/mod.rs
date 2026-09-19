//! §31 — per-project completion.
//!
//! **Ten per-check rows, never two integers.** `project_check` is the only owner of check state;
//! `project.completion_lit` / `project.completion_applicable` are a projection recomputed from
//! those rows in the same transaction, by one function with exactly one production call site.
//!
//! The module is split so that the part that decides is testable without a database:
//!
//! - [`evaluate`] is **pure** — a shaped input struct to ten rows, no database, no remote, and no
//!   clock beyond the `now` handed to it.
//! - `inputs` is the only file here that reads a table or reaches into the remote module. The
//!   path is deliberately not spelled out in this file: `remote_no_completion_writer` scans for
//!   it as a plain substring, and a file naming it may name no completion column — which is the
//!   seam being enforced, stated here rather than rediscovered.
//! - `store` owns `project_check`.
//! - [`proposal`] owns §31.4's archetype proposal, in both vocabularies.
//!
//! **Six of the ten checks read §28's stored answer and re-derive nothing** (R124). §28.10 left
//! the direction open — *"§31 owns the check side and reads the item, **or** owns the predicate
//! and §28 reads it, but never both"* — and R124 closed it toward §28, which owns the singleton
//! item evaluator. Two evaluations of one predicate is R12 with a user-visible disagreement at
//! the end of it: a tick reading `pass` beside an open item saying otherwise, each green in its
//! own tests.

pub mod evaluate;
pub mod inputs;
pub mod proposal;
pub mod store;

use rusqlite::Transaction;

use crate::index::completion::{set_completion, Completion};
use crate::index::IndexError;
use crate::protocol::{CompletionCheck, ProjectId};

use evaluate::{evaluate, CheckRow, Counts};

/// What one recomputation did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Written {
    /// `gather` declined: Reference, authorship not computed, not cloned, or a frozen reading.
    /// **Nothing is written and nothing is cleared** — a stored reading stands.
    Skipped(inputs::NotScorable),
    /// The ten computed rows equal the ten stored ones, so nothing was written and every
    /// `observed_at` is where it was.
    Unchanged,
    /// Ten rows and the projection were replaced.
    Rewritten { counts: Counts },
}

/// Recompute one project's ten rows and its projection, in the caller's transaction.
///
/// **The one entry point both hooks call**, and the only production call site of
/// [`set_completion`].
///
/// **Never incremental.** A partial numerator over a growing denominator would render three
/// different tiers in five seconds on first run, and the tier is *material*. One write, one
/// transaction, all ten rows plus the projection.
///
/// **The diff gate** compares the computed rows against the stored ones on
/// `(check_key, state, user_na, unknown_reason)` — **`observed_at` is excluded**, which is what
/// makes *attempted* a derivation rather than a stored column (R123). A no-change recompute
/// writes nothing and leaves `observed_at` where it was: it is when a check's state was last
/// **established**, which is the conservative direction, because an older timestamp never
/// over-claims currency.
///
/// # Errors
/// Fails when SQLite cannot be read or refuses a write.
pub fn evaluate_and_write(
    tx: &Transaction<'_>,
    project: ProjectId,
    now: i64,
) -> Result<Written, IndexError> {
    let gathered = match inputs::gather(tx, project)? {
        Ok(gathered) => gathered,
        Err(why) => return Ok(Written::Skipped(why)),
    };
    let rows = evaluate(&gathered, now);

    let stored = store::load_rows(tx, project)?;
    if stored.len() == rows.len() && stored.iter().zip(rows.iter()).all(|(a, b)| same(a, b)) {
        return Ok(Written::Unchanged);
    }

    store::write_all_ten(tx, project, &rows)?;
    let (lit, evaluable) = store::recount(&rows);
    // **`evaluable == 0` is not a zero; it is `NotComputed`.** The guard inside `set_completion`
    // refuses `applicable = 0` outright, so the choice is made here rather than left to an error
    // the caller would have to interpret.
    let value = if evaluable == 0 {
        Completion::NotComputed
    } else {
        Completion::Computed {
            lit,
            applicable: evaluable,
        }
    };
    set_completion(tx, project, value)?;

    Ok(Written::Rewritten {
        counts: Counts::of(&rows),
    })
}

/// Everything the diff compares, and deliberately **not** `observed_at`.
fn same(a: &CheckRow, b: &CheckRow) -> bool {
    a.key == b.key
        && a.state == b.state
        && a.user_na == b.user_na
        && a.unknown_reason == b.unknown_reason
}

/// Write the user's ruling for one key and recompute in the same transaction.
///
/// §31.5 names three triggers for the evaluator and R123 kept only the two settle hooks; this is
/// the third, and it is the only one that survives as a trigger rather than a hook.
///
/// # Errors
/// Fails when SQLite cannot be read or refuses a write.
pub fn set_check_na(
    tx: &Transaction<'_>,
    project: ProjectId,
    key: CompletionCheck,
    na: Option<bool>,
    now: i64,
) -> Result<Written, IndexError> {
    store::set_user_na(tx, project, key, na)?;
    evaluate_and_write(tx, project, now)
}
