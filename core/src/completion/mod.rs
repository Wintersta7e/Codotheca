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

use crate::clock::LocalInstant;
use crate::index::completion::{set_completion, Completion};
use crate::index::IndexError;
use crate::ledger::latch::latch_completion_checks;
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
    Rewritten {
        /// The counts the new rows project onto.
        counts: Counts,
    },
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
/// **A project's first write also applies what a rebuild staged for it** (§48.8.4): a ruling
/// restores onto its check's row, so it cannot land before that row exists.
///
/// **Then the latch (§38.8.2), on a rewrite and an unchanged recompute alike** — the unchanged
/// return is exactly where a third commit-day opening the gate under a passing check would
/// otherwise be lost. It reads the rows computed here; a skipped reading latches nothing.
///
/// # Errors
/// Fails when SQLite cannot be read or refuses a write.
pub fn evaluate_and_write(
    tx: &Transaction<'_>,
    project: ProjectId,
    at: LocalInstant,
) -> Result<Written, IndexError> {
    match recompute(tx, project, at.unix)? {
        Ok((written, rows)) => {
            latch_completion_checks(tx, project, &rows, at)?;
            Ok(written)
        }
        Err(why) => Ok(Written::Skipped(why)),
    }
}

/// The recompute without the latch: what was written, and the rows as they now stand computed.
fn recompute(
    tx: &Transaction<'_>,
    project: ProjectId,
    now: i64,
) -> Result<Result<(Written, Vec<CheckRow>), inputs::NotScorable>, IndexError> {
    let gathered = match inputs::gather(tx, project)? {
        Ok(gathered) => gathered,
        Err(why) => return Ok(Err(why)),
    };
    let rows = evaluate(&gathered, now);

    let stored = store::load_rows(tx, project)?;
    if stored.len() == rows.len() && stored.iter().zip(rows.iter()).all(|(a, b)| same(a, b)) {
        return Ok(Ok((Written::Unchanged, rows.to_vec())));
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
    let written = Written::Rewritten {
        counts: Counts::of(&rows),
    };

    // A rebuilt index restores the user's N/A rulings onto these rows, which exist only from a
    // project's first write. After the projection, which the restore rewrites through
    // `restore_ruling`; never on a later write, which that call is. A restore that applied
    // anything recomputed the rows under it, so the latch reads that computation — a check the
    // user ruled N/A is never paid as the pass it read before the ruling landed.
    if stored.is_empty()
        && !crate::index::pending::match_pending(tx, project, now)?
            .applied
            .is_empty()
    {
        if let Ok((_, restored)) = recompute(tx, project, now)? {
            return Ok(Ok((written, restored)));
        }
    }

    Ok(Ok((written, rows.to_vec())))
}

/// Everything the diff compares, and deliberately **not** `observed_at`.
fn same(a: &CheckRow, b: &CheckRow) -> bool {
    a.key == b.key
        && a.state == b.state
        && a.user_na == b.user_na
        && a.unknown_reason == b.unknown_reason
}

/// Write the user's ruling for one key and recompute in the same transaction, latching what
/// the recompute finds passing.
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
    at: LocalInstant,
) -> Result<Written, IndexError> {
    store::set_user_na(tx, project, key, na)?;
    evaluate_and_write(tx, project, at)
}

/// A ruling restored from a sidecar, written and recomputed in the caller's transaction, with
/// **no latch**: a restore knows its instant and not the zone a latch's date is read in.
///
/// Nothing is lost by it. The restore inside a project's first write is latched by that write,
/// over the rows this recompute leaves; any other restore is latched by the project's next
/// settle, which latches on an unchanged recompute too.
///
/// # Errors
/// Fails when SQLite cannot be read or refuses a write.
pub(crate) fn restore_ruling(
    tx: &Transaction<'_>,
    project: ProjectId,
    key: CompletionCheck,
    na: Option<bool>,
    now: i64,
) -> Result<Written, IndexError> {
    store::set_user_na(tx, project, key, na)?;
    Ok(recompute(tx, project, now)?.map_or_else(Written::Skipped, |(written, _)| written))
}
