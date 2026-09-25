//! §22.4 — the scan direction. The matcher's rule, applied where a scan would create a row.
//!
//! > **Every outcome of `decide` that would create a `project` row instead HYDRATES the single
//! > not-cloned project carrying this repository's folded `remote_key`, when exactly one exists.**
//!
//! **Why this is not already solved.** `load_candidates` keys on lineage and returns an empty set
//! whenever lineage is NULL. A not-cloned project has never had a commit read from it, so its
//! `lineage_key` is NULL and **it is in no candidate set** — which is why the ordinary sequence
//! *connect → sync → clone in a terminal → rescan* reaches `IdentityDecision::New` and mints a
//! second tile for a repository the library already knows about.
//!
//! **Hydration is not a merge.** One `project` row existed for this repository throughout:
//! nothing is absorbed, nothing is tombstoned, no `merged_into` is set, no `project_redirect` and
//! no `merge_record` row is written, no `projects.merged` event is emitted, and `projects.merge`
//! keeps its zero call sites. A build in which this calls `merge_projects` is wrong twice — it
//! tombstones a row that never existed, and `choose_survivor` picks the earliest `created_at`, so
//! a blueprint synced on Monday would absorb a local project with sessions and notes cloned on
//! Tuesday.

use rusqlite::{params, Transaction};

use super::alias::{fold_key, stored_spellings, HostAliases};
use super::decide::IdentityEvidence;
use super::IdentityError;

/// How many not-cloned projects carry this repository's folded key.
///
/// `Many` is not an invitation to pick one. §22.5's rule is that two competing identities are an
/// ambiguity, and choosing between them by any rule at all would make the answer depend on which
/// row the walk reached first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HydrationTarget {
    /// No not-cloned project carries the key; the scan creates a row.
    None,
    /// Exactly one does — the project to hydrate.
    One(i64),
    /// Two or more do, ordered `(created_at, id)`; the scan creates a row and flags them all.
    Many(Vec<i64>),
}

/// The read §22.4's lookup runs, once per stored spelling of the folded key.
///
/// **No index is added for this** — `idx_project_remote` and `location(project_id)` already serve
/// it (§1.11, §22.4) — which is only true if the statement lets them. A scan reaches this once per
/// repository that would create a row, so a table scan here is `O(n²)` over a first scan of the
/// library. Exposed so a test can `EXPLAIN QUERY PLAN` the statement that actually runs.
pub const HYDRATION_TARGET_SQL: &str = "SELECT id, remote_key, created_at
   FROM project
  WHERE remote_key = ?1 AND merged_into IS NULL
    AND NOT EXISTS (SELECT 1 FROM location WHERE location.project_id = project.id)";

/// The not-cloned projects on this folded key, ordered `(created_at, id)`.
///
/// **Not-cloned is zero `location` rows**, which is the only honest reading: a project with a
/// copy on disk is not the row a scan of that disk should hydrate.
///
/// The rows are narrowed by index on each **stored** spelling — one equality per declared host,
/// never a `LIKE` — and the fold is then applied in Rust to both sides, by the one
/// implementation, so a row a spelling read reached whose folded key does not in fact match is
/// dropped.
///
/// # Errors
/// Fails with [`IdentityError::Sqlite`] when the read is refused.
pub fn find_hydration_target(
    tx: &Transaction<'_>,
    folded_key: &str,
    aliases: &HostAliases,
) -> Result<HydrationTarget, IdentityError> {
    let mut st = tx.prepare(HYDRATION_TARGET_SQL)?;
    let mut matched: Vec<(i64, i64)> = Vec::new();
    for spelling in stored_spellings(folded_key, aliases) {
        let rows = st.query_map(params![spelling], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
            ))
        })?;
        for row in rows {
            let (project_id, stored, created_at) = row?;
            if fold_key(&stored, aliases).as_deref() != Some(folded_key) {
                continue;
            }
            if !matched.iter().any(|(seen, _)| *seen == project_id) {
                matched.push((project_id, created_at));
            }
        }
    }
    matched.sort_by_key(|(project_id, created_at)| (*created_at, *project_id));
    let matched: Vec<i64> = matched
        .into_iter()
        .map(|(project_id, _)| project_id)
        .collect();
    Ok(match matched.as_slice() {
        [] => HydrationTarget::None,
        [only] => HydrationTarget::One(*only),
        _ => HydrationTarget::Many(matched),
    })
}

/// Write the four columns a clone teaches a not-cloned row, and no others.
///
/// | Column | |
/// |---|---|
/// | `lineage_key` | NULL → the observed key. **Still NULL for a shallow or unborn clone, and hydration fires anyway** — the row is matched on `remote_key`, not on lineage |
/// | `is_shallow` | the DDL default `0` → the observed value. `0` on a not-cloned row is a default, not an observation |
/// | `is_fork` | may be set to `1`; **never cleared** (§22.8) |
/// | `updated_at` | `now` |
/// | `seed_basename` | **never written** |
/// | `name` | **never written** |
///
/// **The omission is the point.** Stated as a corollary of the bare-name seed rule it invites a
/// later author to "correct" a hydrated project's `seed_basename` to the directory basename it
/// now has on disk — a change that looks like tidying and re-rolls every card the user pressed
/// `Install` on. §7.4's rename clause does not apply and must not be reached for: hydration is
/// not a rename, there is no old path, and a not-cloned project has no location that could be
/// `missing`.
///
/// # Errors
/// [`IdentityError::HydrateWouldOrphanXp`] when the target already carries git-track `xp_events`
/// rows. It **fails the transaction** rather than warning: a row that has earned git XP is not a
/// row nothing has ever looked at, and hydrating it would attach this repository's history to
/// another repository's ledger.
pub fn hydrate(
    tx: &Transaction<'_>,
    project_id: i64,
    evidence: &IdentityEvidence,
    is_shallow: bool,
    is_fork: bool,
    now: i64,
) -> Result<(), IdentityError> {
    // §1.7's own discriminator, and the same predicate `merge.rs:339` uses — **not** the count at
    // `:332`, which has no `project_id` clause and would fire on any git XP anywhere.
    let count: i64 = tx.query_row(
        "SELECT COUNT(*) FROM xp_events WHERE project_id = ?1 AND track = 'git'",
        params![project_id],
        |r| r.get(0),
    )?;
    if count != 0 {
        return Err(IdentityError::HydrateWouldOrphanXp { project_id, count });
    }

    tx.execute(
        "UPDATE project
            SET lineage_key = ?2,
                is_shallow  = ?3,
                is_fork     = max(is_fork, ?4),
                updated_at  = ?5
          WHERE id = ?1",
        params![
            project_id,
            evidence.lineage_key,
            i64::from(is_shallow),
            i64::from(is_fork),
            now,
        ],
    )?;
    Ok(())
}
