//! §45's deletion analyser: the one function that decides whether a governed act may destroy a
//! byte, and every construction of an `UninstallBlocker`.
//!
//! This change lands its network seam first — [`remote`], the verifying read of one configured
//! remote — because `Intent::Fetch` retires in the same change and its one caller needs a
//! replacement. The analyser's steps follow in the changes that make each of them true: step 1,
//! [`identity`], and the one row read it compares against, [`read_location_row`].

pub mod identity;
pub mod remote;

use std::path::PathBuf;

use crate::git::StoreKey;
use crate::proto::dispatch::CommandFailure;
use crate::protocol::LocationId;

/// The `location` row, as the analyser may read it (§45.7).
///
/// **`project.is_shallow` is not here, and neither is any cached observation of the tree.**
/// Shallowness is read live (step 1); the row supplies what the live reads are compared
/// against. `store_key` and `trusted` are invocation plumbing — the slot pool and
/// `-c safe.directory` — not verdict inputs (D-4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocationRow {
    /// The row read.
    pub id: LocationId,
    /// `path_bytes`, decoded: resolved and re-statted, never trusted as a repository.
    pub path: PathBuf,
    /// `project.lineage_key`: what step 1's live derivation is compared with.
    pub lineage_key: Option<String>,
    /// `removed_at`, which refuses.
    pub removed_at: Option<i64>,
    /// When ref state was last observed; `None` means the app has never looked (§24.7G).
    pub refstate_observed_at: Option<i64>,
    /// When the worktree was last observed; the same question.
    pub worktree_observed_at: Option<i64>,
    /// The slot pool the reads take their turn in.
    pub store: StoreKey,
    /// `trusted_at` is set: git reads it with `-c safe.directory`.
    pub trusted: bool,
}

/// **The one row read** — `location` joined to its project's `lineage_key`, in the caller's
/// transaction.
///
/// # Errors
/// `PROTOCOL` when the id names no location; `INTERNAL` for any other SQLite fault.
pub fn read_location_row(
    tx: &rusqlite::Transaction<'_>,
    id: LocationId,
) -> Result<LocationRow, CommandFailure> {
    tx.query_row(
        "SELECT l.path_bytes, p.lineage_key, l.removed_at, l.refstate_observed_at,
                l.worktree_observed_at, l.store_key, l.trusted_at
           FROM location l JOIN project p ON p.id = l.project_id
          WHERE l.id = ?1",
        [id.0],
        |r| {
            Ok(LocationRow {
                id,
                path: crate::paths::path_from_bytes(&r.get::<_, Vec<u8>>(0)?),
                lineage_key: r.get(1)?,
                removed_at: r.get(2)?,
                refstate_observed_at: r.get(3)?,
                worktree_observed_at: r.get(4)?,
                store: StoreKey::new(r.get::<_, String>(5)?),
                trusted: r.get::<_, Option<i64>>(6)?.is_some(),
            })
        },
    )
    .map_err(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => {
            CommandFailure::protocol(format!("no location {}", id.0))
        }
        other => CommandFailure::internal(other.to_string()),
    })
}
