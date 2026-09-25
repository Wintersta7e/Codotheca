//! §24.8's mutating call's **one write**: the row, after the bytes are gone.
//!
//! The act's order is the handler's (`uninstall::handle`): the row read under the lock, §45's
//! analysis off it, not `safe` ends it, step 9's re-read, the warranted removal — and then, under
//! the lock again, this, in one transaction. **No git read runs under the lock.** A crash between
//! the removal and this commit leaves the bytes gone and the row describing them, today's window
//! (§46.6's journal closes it).

use rusqlite::OptionalExtension;

use crate::debt::store::{DebtStore, SqliteDebtStore};
use crate::proto::dispatch::CommandFailure;
use crate::protocol::{LocationId, ProjectId};

/// §24.6b's ten columns: they describe a directory that no longer exists.
///
/// **NULL, never 0.** A zeroed `untracked_count` on a removed copy is a claim that it had no
/// untracked files, which nobody observed. All ten are already nullable, so this needs no
/// migration.
const CLEARED_ON_REMOVAL: [&str; 10] = [
    "is_dirty",
    "untracked_count",
    "ahead",
    "behind",
    "stash_count",
    "interrupted_op",
    "branch",
    "worktree_observed_at",
    "refstate_observed_at",
    "refstate_basis",
];

/// Record one removal: `removed_at` and the ten NULLs, and §28.3's debt guard, in `tx`.
///
/// # Errors
/// `INTERNAL` when the row cannot be written.
pub fn commit_removal(
    tx: &rusqlite::Transaction<'_>,
    location: LocationId,
    now: i64,
) -> Result<(), CommandFailure> {
    // `removed_at` and the ten columns commit together, or neither does. A tree with the bytes
    // gone and the columns still describing them is the state this transaction exists to prevent.
    let clears = CLEARED_ON_REMOVAL
        .iter()
        .map(|column| format!("{column} = NULL"))
        .collect::<Vec<_>>()
        .join(", ");
    // `head_oid` is **retained**: a durable fact about what was removed, and what a re-clone can
    // be checked against.
    tx.execute(
        &format!("UPDATE location SET removed_at = ?2, {clears} WHERE id = ?1"),
        rusqlite::params![location.0, now],
    )
    .map_err(|error| CommandFailure::internal(error.to_string()))?;

    // **[p3] §28.3's second uninstall guard, in this same transaction.** Removing the bytes keeps
    // the row, so `presence` still reads `present` and a naive sweep afterwards finds a
    // readable-looking absence, reports `complete` with zero items, **closes every item and pays
    // for it**. §28.5's rule 4 is the first guard; this is the second, and both are needed.
    //
    // **The mark is `state = 'unverified'` and nothing else.** No closure, no XP, no
    // `health_delta`, and `last_seen_location_id` is **kept**: it is what the reap later compares
    // against, and clearing it would turn a reapable item into a permanently stranded one. The
    // rule has one owner, §28's store, and this is its caller.
    let project: Option<i64> = tx
        .query_row(
            "SELECT project_id FROM location WHERE id = ?1",
            [location.0],
            |r| r.get(0),
        )
        .optional()
        .map_err(|error| CommandFailure::internal(error.to_string()))?;
    if let Some(project) = project {
        SqliteDebtStore
            .mark_unverified(tx, ProjectId(project))
            .map_err(|error| CommandFailure::internal(error.to_string()))?;
    }
    Ok(())
}

/// The ten columns, for the test that asserts they all read NULL afterwards.
#[must_use]
pub const fn cleared_columns() -> [&'static str; 10] {
    CLEARED_ON_REMOVAL
}
