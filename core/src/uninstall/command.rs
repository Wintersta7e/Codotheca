//! §24.8's mutating call: **recomputed inside, refused if it changed**.
//!
//! *A verdict rendered thirty seconds ago is a cache*, and **never claim currency you do not
//! have** applies to a safety verdict more than to anything else on the shelf. The renderer passes
//! a `LocationId` and nothing else; **no verdict token crosses a call boundary**, so there is
//! nothing for a caller to replay and nothing to forge.
//!
//! The order below is not negotiable.

use rusqlite::OptionalExtension;

use crate::analyser::identity::LiveIdentity;
use crate::debt::store::{DebtStore, SqliteDebtStore};
use crate::proto::dispatch::CommandFailure;
use crate::protocol::{LocationId, ProjectId, UninstallDisposition};
use crate::removal::{remove_warranted, RemovalOutcome, Trash, Warrant};
use crate::uninstall::preflight::{compute_verdict, VerdictInputs};

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

/// What a completed removal did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Removed {
    /// The copy whose bytes were removed; its row is kept.
    pub location: LocationId,
    /// Whether the bytes went to the trash or were deleted outright.
    pub outcome: RemovalOutcome,
    /// The `location.removed_at` stamp written, in unix seconds.
    pub removed_at: i64,
}

/// Perform one removal, or refuse.
///
/// **There is no override, no confirmation path and no secondary wording that reaches the
/// removal** (AC-P2-24-14). A disposition that is not `safe` ends the call.
///
/// # Errors
/// Refuses when the freshly computed verdict is not `safe`, when the directory's identity no
/// longer matches its row, or when the removal itself fails.
pub fn uninstall_location(
    tx: &rusqlite::Transaction<'_>,
    inputs: &VerdictInputs,
    warrant: &Warrant,
    trash: &dyn Trash,
    identity_now: &LiveIdentity,
) -> Result<Removed, CommandFailure> {
    // 2. The same function the pre-flight calls, not a copy. (1 — the row read and the identity
    //    re-derivation — is the caller's, because it needs the git seam this module does not hold;
    //    its result arrives as `identity_now` and is checked against the warrant's row lineage
    //    inside `remove_warranted`.)
    let (verdict, _seal) = compute_verdict(inputs)?;

    // 3. Not safe ends it. `unknown` ends it too: an absence is not a permission.
    if verdict.disposition != UninstallDisposition::Safe {
        return Err(CommandFailure::protocol(format!(
            "uninstall refused: {:?} — {:?}",
            verdict.disposition, verdict.blockers
        )));
    }

    // 4. The one warranted primitive. §24.7F's Recycle Bin, not a hard delete: a working copy is
    //    the user's, and a hard delete is permitted only for bytes this process wrote itself.
    let outcome = remove_warranted(warrant, trash, identity_now)
        .map_err(|refusal| CommandFailure::protocol(format!("uninstall refused: {refusal:?}")))?;

    // 5. One transaction: `removed_at` and the ten columns commit together, or neither does. A
    //    tree with the bytes gone and the columns still describing them is the state this order
    //    exists to prevent.
    let clears = CLEARED_ON_REMOVAL
        .iter()
        .map(|column| format!("{column} = NULL"))
        .collect::<Vec<_>>()
        .join(", ");
    // 6. `head_oid` is **retained**: a durable fact about what was removed, and what a re-clone
    //    can be checked against.
    tx.execute(
        &format!("UPDATE location SET removed_at = ?2, {clears} WHERE id = ?1"),
        rusqlite::params![inputs.snapshot.id.0, inputs.now],
    )
    .map_err(|error| CommandFailure::internal(error.to_string()))?;

    // 7. **[p3] §28.3's second uninstall guard, in this same transaction.** Removing the bytes
    //    keeps the row, so `presence` still reads `present` and a naive sweep afterwards finds a
    //    readable-looking absence, reports `complete` with zero items, **closes every item and
    //    pays for it**. §28.5's rule 4 is the first guard; this is the second, and both are
    //    needed.
    //
    //    **The mark is `state = 'unverified'` and nothing else.** No closure, no XP, no
    //    `health_delta`, and `last_seen_location_id` is **kept**: it is what the reap later
    //    compares against, and clearing it would turn a reapable item into a permanently
    //    stranded one. The rule has one owner, §28's store, and this is its caller.
    let project: Option<i64> = tx
        .query_row(
            "SELECT project_id FROM location WHERE id = ?1",
            [inputs.snapshot.id.0],
            |r| r.get(0),
        )
        .optional()
        .map_err(|error| CommandFailure::internal(error.to_string()))?;
    if let Some(project) = project {
        SqliteDebtStore
            .mark_unverified(tx, ProjectId(project))
            .map_err(|error| CommandFailure::internal(error.to_string()))?;
    }

    Ok(Removed {
        location: inputs.snapshot.id,
        outcome,
        removed_at: inputs.now,
    })
}

/// The ten columns, for the test that asserts they all read NULL afterwards.
#[must_use]
pub const fn cleared_columns() -> [&'static str; 10] {
    CLEARED_ON_REMOVAL
}
