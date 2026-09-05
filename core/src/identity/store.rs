//! Applying an identity decision to the index (§1.1), and the `location` writer R1 assigns
//! here.
//!
//! R1 gives both to this module because the `location` row *is* the project↔path association
//! `resolve_identity` has just decided: a decision committed without its row is a repository
//! that was discovered and then vanished. `crate::scan::store::SqliteScanStore::upsert_location`
//! delegates rather than writing, so the scanner keeps one seam and location writes never split
//! across two owners.

use rusqlite::{params, OptionalExtension as _, Transaction};

use super::alias::{fold_key, stored_spellings, HostAliases};
use super::decide::{
    decide, evidence_from, Candidate, IdentityDecision, IdentityEvidence, IdentityProbe,
};
use super::hydrate::{find_hydration_target, hydrate, HydrationTarget};
use super::{AssociationKind, IdentityError};
use crate::derive::LocationKind;
use crate::index::path::{display_paths_for_ui, DisplayPathTable, StoredPath};
use crate::protocol::Presence;
use crate::scan::discover::RepoKind;
use crate::scan::run::platform_of;

/// Everything one `location` row needs except its ids (§1.3).
///
/// **This is R27's shape, which supersedes R1's**, and each correction is a column that would
/// otherwise be written wrong:
///
/// * `presence` is explicit — the column has no default, so a writer without this field has to
///   hard-code `'present'`.
/// * `volume_key` is `Option` and the column is nullable — `None` means no stable identifier
///   exists, and mapping it to `''` invents one.
/// * there is no `MountFacts`: the store *class* is a property of the mount right now, not of
///   the location, and a persisted copy goes stale the moment a drive is remounted elsewhere.
/// * `common_dir_bytes` is raw bytes, not a folded key — `path_bytes` and `path_key` are separate
///   columns because that distinction is semantic.
/// * `repo_kind` replaces `is_worktree`, which could not hold `RepoKind`'s four variants; the
///   difference between a linked worktree and a separate git dir is what §1.5 decides lineage on.
///
/// **No serde yet, and R27 asks for it.** Its stated reason is that plan 18's in-distro worker
/// sends one back over the protocol. `StoredPath`'s fields are private and two of the three are
/// *derived* from the third plus a `PathPlatform`, so a derived impl would put the key and the
/// display string on the wire as independent values that can disagree with the bytes — the drift
/// this project keeps finding. The encoding has to carry `(bytes, platform)` and nothing else,
/// and that is a decision for the change that gives plan 18 a wire type to put it in, not a guess
/// made here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocationInput {
    pub kind: LocationKind,
    /// `None` for everything but WSL. The column is `NOT NULL` and holds `''` in that case —
    /// v1 used NULL, and SQLite treats NULLs as distinct in a UNIQUE index, so the
    /// `(kind, distro, path_key)` constraint silently permitted duplicates.
    pub distro: Option<String>,
    /// `path_bytes`, `path_key` and `path_display` in one value, keyed for the platform the
    /// path belongs to rather than the host (R2).
    pub path: StoredPath,
    pub store_key: String,
    /// `None` where no stable identifier exists — a bind mount, overlayfs, tmpfs. Absent is not
    /// unknown-and-therefore-empty: a location with no volume key can never be recognised
    /// across a remount, and callers must handle that rather than invent one.
    pub volume_key: Option<String>,
    pub presence: Presence,
    pub repo_kind: RepoKind,
    pub common_dir_bytes: Option<Vec<u8>>,
    /// `location.scan_generation` — the run that last saw this path (§4.6).
    pub generation: i64,
    pub last_seen_at: Option<i64>,
}

/// What `resolve_identity` did, for the caller's event and its own log line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentityOutcome {
    pub project_id: i64,
    /// True when a `project` row was inserted, false when an existing one absorbed the location.
    pub created: bool,
    /// The evidence that attached this location, `None` when a project was created.
    pub association: Option<AssociationKind>,
    /// `project.ambiguous_lineage` as it now stands.
    pub ambiguous: bool,
    pub is_fork: bool,
}

/// Every non-tombstoned project on this lineage, ordered so that "earliest" is deterministic.
pub fn load_candidates(
    tx: &Transaction<'_>,
    lineage_key: &str,
) -> Result<Vec<Candidate>, IdentityError> {
    let mut st = tx.prepare(
        "SELECT id, remote_key, created_at
           FROM project
          WHERE lineage_key = ?1 AND merged_into IS NULL
          ORDER BY created_at, id",
    )?;
    let rows = st.query_map(params![lineage_key], |r| {
        Ok(Candidate {
            project_id: r.get(0)?,
            remote_key: r.get(1)?,
            created_at: r.get(2)?,
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

/// The project that already owns a location with this `git-common-dir` — definitive evidence.
pub fn worktree_owner(
    tx: &Transaction<'_>,
    common_dir_key: &[u8],
) -> Result<Option<i64>, IdentityError> {
    let found = tx
        .query_row(
            "SELECT project_id FROM location
              WHERE common_dir_key = ?1 ORDER BY id LIMIT 1",
            params![common_dir_key],
            |r| r.get::<_, i64>(0),
        )
        .optional()?;
    Ok(found)
}

/// Write the `location` row for a path [`resolve_identity`] has just assigned to `project_id`.
///
/// **The only production writer of a `location` row** (R1). Idempotent on
/// `(kind, distro, path_key)`, which is §1.3's UNIQUE index: a rescan of an already-indexed path
/// updates that row and returns the same id. It has no delete arm and no counterpart that
/// removes a row — a path that stops being found becomes `presence = 'missing'` through the
/// presence pass, and phase 1 has no destructive operation (§17).
///
/// `project_id` is re-pointed on conflict. That is not §8.5.2's forbidden auto-detach: evidence
/// only ever accumulates, `resolve_identity` has decided this path's project inside this same
/// transaction, and no project loses a location without another gaining it in the same commit.
/// Leaving a stale `project_id` would strand the path on a project the evidence no longer
/// supports, and put one repository on the shelf twice.
///
/// The observation columns §1.3 lists — `branch`, `is_dirty`, `untracked_count`, `ahead`,
/// `behind`, `stash_count`, `interrupted_op`, `head_oid`, the three `*_observed_at`,
/// `fetch_head_at`, `trusted_at` — are **not** touched here, on insert or on update. They are
/// NULL until observed, and a rescan that has not re-observed them must not overwrite what a job
/// wrote: absence of a fact is "not computed", never zero and never stale-as-fresh.
pub fn upsert_location(
    tx: &Transaction<'_>,
    project_id: i64,
    loc: &LocationInput,
    now: i64,
) -> Result<i64, IdentityError> {
    let (path_bytes, path_key, path_display) = loc.path.as_params();

    // Ruling 3's contract, enforced instead of stated: the comparison key stored beside the raw
    // bytes is produced by the same canonicaliser that produced `path_key`, one line above, and
    // folded for the location's own platform rather than the host's (R2 — `platform_of` is where
    // that rule lives, so this is not a second statement of it).
    let common_dir_key = loc.common_dir_bytes.as_ref().map(|bytes| {
        StoredPath::from_bytes(bytes.clone(), platform_of(loc.kind.as_str()))
            .key()
            .to_vec()
    });

    // `RETURNING` rather than `last_insert_rowid()`: on the DO UPDATE arm nothing is inserted, so
    // the connection's last rowid is a stale value from an unrelated statement.
    let id = tx.query_row(
        "INSERT INTO location
            (project_id, kind, distro, path_bytes, path_key, path_display,
             store_key, volume_key, presence, repo_kind,
             common_dir_bytes, common_dir_key, scan_generation, last_seen_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
         ON CONFLICT (kind, distro, path_key) DO UPDATE SET
            project_id       = excluded.project_id,
            path_bytes       = excluded.path_bytes,
            path_display     = excluded.path_display,
            store_key        = excluded.store_key,
            volume_key       = excluded.volume_key,
            presence         = excluded.presence,
            repo_kind        = excluded.repo_kind,
            common_dir_bytes = excluded.common_dir_bytes,
            common_dir_key   = excluded.common_dir_key,
            scan_generation  = excluded.scan_generation,
            last_seen_at     = excluded.last_seen_at
         RETURNING id",
        params![
            project_id,
            loc.kind.as_str(),
            loc.distro.as_deref().unwrap_or(""),
            path_bytes,
            path_key,
            path_display,
            loc.store_key,
            loc.volume_key,
            loc.presence.as_str(),
            loc.repo_kind.as_str(),
            loc.common_dir_bytes,
            common_dir_key,
            loc.generation,
            loc.last_seen_at.unwrap_or(now),
        ],
        |r| r.get::<_, i64>(0),
    )?;
    Ok(id)
}

/// Assign this repository its project (§1.1). Writes `project` only; the `location` row for the
/// path goes through [`upsert_location`] under the returned id, in this same transaction.
///
/// **§22.4's amendment: the creating arms route through create-or-hydrate.** A not-cloned project
/// has a NULL `lineage_key` and is therefore in no candidate set `decide` can see, so without
/// this the ordinary *connect → sync → clone → rescan* sequence mints a second tile.
pub fn resolve_identity(
    tx: &Transaction<'_>,
    probe: &IdentityProbe,
    basename: &str,
    aliases: &HostAliases,
    now: i64,
) -> Result<IdentityOutcome, IdentityError> {
    let evidence = evidence_from(probe);

    let owner = match evidence.common_dir_key.as_deref() {
        Some(key) => worktree_owner(tx, key)?,
        None => None,
    };
    let candidates = match evidence.lineage_key.as_deref() {
        Some(l) => load_candidates(tx, l)?,
        None => Vec::new(),
    };

    match decide(&evidence, owner, &candidates) {
        IdentityDecision::AttachDefinitive { project_id } => {
            attach(tx, project_id, AssociationKind::Definitive, now)
        }
        IdentityDecision::AttachStrong { project_id } => {
            attach(tx, project_id, AssociationKind::Strong, now)
        }
        IdentityDecision::AttachInferred { project_id } => {
            attach(tx, project_id, AssociationKind::Inferred, now)
        }
        IdentityDecision::NewFork { related } => {
            let landed = create_or_hydrate(tx, &evidence, probe, basename, true, aliases, now)?;
            for other in related {
                tx.execute(
                    "UPDATE project SET is_fork = 1, updated_at = ?2 WHERE id = ?1",
                    params![other, now],
                )?;
            }
            // Indexing a fork re-evaluates the remoteless projects indexed before it: one of
            // them may have had a single candidate a moment ago and two now.
            if let Some(l) = evidence.lineage_key.as_deref() {
                flag_remoteless_ambiguity(tx, l, now)?;
            }
            Ok(IdentityOutcome {
                project_id: landed.project_id,
                created: landed.created,
                association: None,
                ambiguous: landed.ambiguous,
                is_fork: true,
            })
        }
        IdentityDecision::NewAmbiguous { .. } => {
            // **This arm calls `create` directly, and the reason is reachability, not taste.**
            // `NewAmbiguous` is produced only where our own `remote_key` is NULL
            // (`decide.rs:92`, `:143-147`), and a NULL key folds to nothing and matches nothing —
            // so there is no hydration target it could ever have. Do not "unify" the three
            // creating arms.
            let id = create(tx, &evidence, probe.is_shallow, basename, false, true, now)?;
            Ok(IdentityOutcome {
                project_id: id,
                created: true,
                association: None,
                ambiguous: true,
                is_fork: false,
            })
        }
        IdentityDecision::New => {
            let landed = create_or_hydrate(tx, &evidence, probe, basename, false, aliases, now)?;
            Ok(IdentityOutcome {
                project_id: landed.project_id,
                created: landed.created,
                association: None,
                ambiguous: landed.ambiguous,
                is_fork: landed.is_fork,
            })
        }
    }
}

/// What create-or-hydrate landed on.
struct Landed {
    project_id: i64,
    created: bool,
    ambiguous: bool,
    is_fork: bool,
}

/// §22.4: hydrate the **single** not-cloned project on this repository's folded key, or create.
///
/// Zero or two-or-more targets both create, unchanged — and two-or-more additionally flags the
/// group for §22.5. **Never pick one.**
fn create_or_hydrate(
    tx: &Transaction<'_>,
    evidence: &IdentityEvidence,
    probe: &IdentityProbe,
    basename: &str,
    is_fork: bool,
    aliases: &HostAliases,
    now: i64,
) -> Result<Landed, IdentityError> {
    let folded = evidence
        .remote_key
        .as_deref()
        .and_then(|key| fold_key(key, aliases));
    let target = match folded.as_deref() {
        Some(key) => find_hydration_target(tx, key, aliases)?,
        // No remote key folds to nothing and matches nothing.
        None => HydrationTarget::None,
    };

    match target {
        HydrationTarget::One(project_id) => {
            hydrate(tx, project_id, evidence, probe.is_shallow, is_fork, now)?;
            // The same call and the same reason as the `NewFork` arm: a project that had one
            // candidate a moment ago may have two now that this lineage is known.
            if let Some(l) = evidence.lineage_key.as_deref() {
                flag_remoteless_ambiguity(tx, l, now)?;
            }
            let (ambiguous, is_fork) = tx
                .query_row(
                    "SELECT ambiguous_lineage, is_fork FROM project WHERE id = ?1",
                    params![project_id],
                    |r| Ok((r.get::<_, i64>(0)? == 1, r.get::<_, i64>(1)? == 1)),
                )
                .optional()?
                .ok_or(IdentityError::UnknownProject(project_id))?;
            Ok(Landed {
                project_id,
                created: false,
                ambiguous,
                is_fork,
            })
        }
        HydrationTarget::None => {
            let id = create(
                tx,
                evidence,
                probe.is_shallow,
                basename,
                is_fork,
                false,
                now,
            )?;
            Ok(Landed {
                project_id: id,
                created: true,
                ambiguous: false,
                is_fork,
            })
        }
        HydrationTarget::Many(targets) => {
            let id = create(tx, evidence, probe.is_shallow, basename, is_fork, true, now)?;
            for other in targets {
                tx.execute(
                    "UPDATE project SET ambiguous_lineage = 1, updated_at = ?2 WHERE id = ?1",
                    params![other, now],
                )?;
            }
            Ok(Landed {
                project_id: id,
                created: true,
                ambiguous: true,
                is_fork,
            })
        }
    }
}

/// The footer names one evidence for all of a project's copies, so an attach can only ever
/// weaken it (§8.5.2, and [`AssociationKind::combine`]).
fn attach(
    tx: &Transaction<'_>,
    project_id: i64,
    kind: AssociationKind,
    now: i64,
) -> Result<IdentityOutcome, IdentityError> {
    let stored: Option<String> = tx
        .query_row(
            "SELECT association_kind FROM project WHERE id = ?1",
            params![project_id],
            |r| r.get(0),
        )
        .optional()?
        .flatten();
    let combined = match stored.as_deref().and_then(AssociationKind::parse) {
        Some(previous) => previous.combine(kind),
        None => kind,
    };
    tx.execute(
        "UPDATE project SET association_kind = ?2, updated_at = ?3 WHERE id = ?1",
        params![project_id, combined.as_str(), now],
    )?;

    let (ambiguous, is_fork) = tx
        .query_row(
            "SELECT ambiguous_lineage, is_fork FROM project WHERE id = ?1",
            params![project_id],
            |r| Ok((r.get::<_, i64>(0)? == 1, r.get::<_, i64>(1)? == 1)),
        )
        .optional()?
        .ok_or(IdentityError::UnknownProject(project_id))?;

    Ok(IdentityOutcome {
        project_id,
        created: false,
        association: Some(combined),
        ambiguous,
        is_fork,
    })
}

/// The identity columns and the NOT NULL minimum. Everything else — `is_bare`, the description
/// chain, sizes, condition — is written by the jobs that produce it, and every column left out
/// here takes the DDL's own default rather than a second statement of it.
///
/// **`last_touched_at` is not written.** §5.1 owns it and `0001` says NULL until a scan job
/// produces one; a project identity has just created has never been scanned, so `now` there
/// would be a touch time the app does not have.
fn create(
    tx: &Transaction<'_>,
    evidence: &IdentityEvidence,
    is_shallow: bool,
    basename: &str,
    is_fork: bool,
    ambiguous: bool,
    now: i64,
) -> Result<i64, IdentityError> {
    tx.execute(
        "INSERT INTO project
            (lineage_key, remote_key, ambiguous_lineage, name, seed_basename, is_fork,
             is_shallow, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?4, ?5, ?6, ?7, ?7)",
        params![
            evidence.lineage_key,
            evidence.remote_key,
            i64::from(ambiguous),
            basename,
            i64::from(is_fork),
            i64::from(is_shallow),
            now,
        ],
    )?;
    Ok(tx.last_insert_rowid())
}

/// A remoteless project that had one candidate when it was indexed may have two once a fork
/// appears. Setting the flag adds information and detaches nothing, which is the only direction
/// phase 1 may move: §8.5.2 rules that nothing undoes an association here.
///
/// The flag is never cleared — [`ambiguous_group`] re-evaluates the candidate set on every read.
pub fn flag_remoteless_ambiguity(
    tx: &Transaction<'_>,
    lineage_key: &str,
    now: i64,
) -> Result<usize, IdentityError> {
    let remoted: i64 = tx.query_row(
        "SELECT COUNT(*) FROM project
          WHERE lineage_key = ?1 AND remote_key IS NOT NULL AND merged_into IS NULL",
        params![lineage_key],
        |r| r.get(0),
    )?;
    if remoted < 2 {
        return Ok(0);
    }
    let changed = tx.execute(
        "UPDATE project SET ambiguous_lineage = 1, updated_at = ?2
          WHERE lineage_key = ?1 AND remote_key IS NULL AND merged_into IS NULL
            AND ambiguous_lineage = 0",
        params![lineage_key, now],
    )?;
    Ok(changed)
}

/// One row of §11.1's eighth scan-summary group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AmbiguousRow {
    pub project_id: i64,
    pub location_id: Option<i64>,
    pub path_display: Option<String>,
    /// At most two, ordered `(created_at, id)`.
    pub candidate_names: Vec<String>,
    pub candidate_total: usize,
}

/// §11.1's ambiguous-lineage group. **A live query.** The candidate set is not stored and must
/// not be: it changes as the history job progresses, so a set frozen at flag time is a claim
/// about a scan that has since moved.
///
/// The display path comes back through `index::path::display_paths_for_ui`, the one door §1.10
/// allows. The plan writes the read inline here instead, which this project's own gate refuses.
///
/// **Two bases, and the thresholds differ on purpose (§22.5).** The lineage arm is phase 1's and
/// is unchanged, down to its `< 2`: there the subject is a *remoteless third party* choosing
/// between candidates, so one candidate is not a choice — it is `AttachInferred`'s case, and
/// `the_ambiguous_group_names_its_candidates_and_drops_a_project_that_lost_them` asserts exactly
/// that. The remote arm takes `>= 1`, because there the subject is **one of the two competing
/// identities** and one other project is the whole ambiguity.
///
/// **Phase-1 output is unchanged by construction**: every row `flag_remoteless_ambiguity` can
/// flag has `remote_key IS NULL`, and NULL matches nothing, so the remote arm contributes an
/// empty set to every pre-existing row. It also no longer `continue`s on a NULL `lineage_key` —
/// a not-cloned project's lineage is NULL by construction (§22.4), so skipping on it made §22.5's
/// case unrepresentable.
pub fn ambiguous_group(
    tx: &Transaction<'_>,
    aliases: &HostAliases,
) -> Result<Vec<AmbiguousRow>, IdentityError> {
    let mut flagged = tx.prepare(
        "SELECT id, lineage_key, remote_key FROM project
          WHERE ambiguous_lineage = 1 AND merged_into IS NULL
          ORDER BY id",
    )?;
    let subjects = flagged
        .query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, Option<String>>(2)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;

    let mut out = Vec::new();
    for (project_id, lineage_key, remote_key) in subjects {
        let by_lineage = match lineage_key {
            Some(ref key) => lineage_candidates(tx, key, project_id)?,
            None => Vec::new(),
        };
        let by_remote = match remote_key.as_deref().and_then(|k| fold_key(k, aliases)) {
            Some(ref folded) => remote_candidates(tx, folded, project_id, aliases)?,
            None => Vec::new(),
        };
        if by_lineage.len() < 2 && by_remote.is_empty() {
            // No longer ambiguous on either basis. The row stops appearing; nothing is written.
            continue;
        }
        let mut names = by_lineage;
        for (id, name) in by_remote {
            if !names.iter().any(|(seen, _)| *seen == id) {
                names.push((id, name));
            }
        }
        let names: Vec<String> = names.into_iter().map(|(_, name)| name).collect();

        let location_id: Option<i64> = tx
            .query_row(
                "SELECT id FROM location WHERE project_id = ?1 ORDER BY id LIMIT 1",
                params![project_id],
                |r| r.get(0),
            )
            .optional()?;
        let path_display = match location_id {
            Some(id) => display_paths_for_ui(tx, DisplayPathTable::Location, &[id])
                .map_err(IdentityError::Index)?
                .into_iter()
                .next()
                .map(|(_, p)| p),
            None => None,
        };

        out.push(AmbiguousRow {
            project_id,
            location_id,
            path_display,
            candidate_total: names.len(),
            candidate_names: names.into_iter().take(2).collect(),
        });
    }
    Ok(out)
}

/// Phase 1's arm, byte for byte: the projects sharing this lineage that carry a remote.
fn lineage_candidates(
    tx: &Transaction<'_>,
    lineage_key: &str,
    subject: i64,
) -> Result<Vec<(i64, String)>, IdentityError> {
    let mut st = tx.prepare(
        "SELECT id, name FROM project
          WHERE lineage_key = ?1 AND remote_key IS NOT NULL AND merged_into IS NULL
            AND id <> ?2
          ORDER BY created_at, id",
    )?;
    let rows = st.query_map(params![lineage_key, subject], |r| {
        Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
    })?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

/// §22.5's arm: the projects sharing this subject's **folded** `remote_key`.
///
/// Narrowed by `idx_project_remote` on each stored spelling — one equality per declared host,
/// never a `LIKE` — and folded in Rust on both sides, for the same reason §22.2 gives: a stored
/// key carries whichever host spelling the clone used. This is a **live query** run per flagged
/// subject at render time, so a table scan here would be `O(flagged × library)`.
fn remote_candidates(
    tx: &Transaction<'_>,
    folded: &str,
    subject: i64,
    aliases: &HostAliases,
) -> Result<Vec<(i64, String)>, IdentityError> {
    let mut st = tx.prepare(
        "SELECT id, name, remote_key, created_at FROM project
          WHERE remote_key = ?1 AND merged_into IS NULL AND id <> ?2",
    )?;
    let mut found: Vec<(i64, i64, String)> = Vec::new();
    for spelling in stored_spellings(folded, aliases) {
        let rows = st.query_map(params![spelling, subject], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
            ))
        })?;
        for row in rows {
            let (id, name, stored, created_at) = row?;
            if fold_key(&stored, aliases).as_deref() != Some(folded) {
                continue;
            }
            if !found.iter().any(|(seen, _, _)| *seen == id) {
                found.push((id, created_at, name));
            }
        }
    }
    found.sort_by_key(|(id, created_at, _)| (*created_at, *id));
    Ok(found.into_iter().map(|(id, _, name)| (id, name)).collect())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::super::decide::IdentityProbe;
    use super::super::testutil::{forge_aliases, open_test_index};
    use super::IdentityOutcome;

    /// Every test below calls the production function with the one alias set this crate's
    /// fixtures use, so §22.4's amendment did not turn into fifteen edited call sites.
    fn resolve_identity(
        tx: &rusqlite::Transaction<'_>,
        probe: &IdentityProbe,
        basename: &str,
        at: i64,
    ) -> Result<IdentityOutcome, IdentityError> {
        super::resolve_identity(tx, probe, basename, &forge_aliases(), at)
    }

    fn probe(roots: &[&str], remotes: &[(&str, &str)], common: Option<&[u8]>) -> IdentityProbe {
        IdentityProbe {
            common_dir_key: common.map(<[u8]>::to_vec),
            is_shallow: false,
            root_oids: roots.iter().map(|s| (*s).to_owned()).collect(),
            remote_urls: remotes
                .iter()
                .map(|(n, u)| ((*n).to_owned(), (*u).to_owned()))
                .collect(),
        }
    }

    fn flag(conn: &rusqlite::Connection, id: i64, col: &str) -> i64 {
        conn.query_row(
            &format!("SELECT {col} FROM project WHERE id=?1"),
            rusqlite::params![id],
            |r| r.get(0),
        )
        .unwrap()
    }

    fn kind(conn: &rusqlite::Connection, id: i64) -> Option<String> {
        conn.query_row(
            "SELECT association_kind FROM project WHERE id=?1",
            rusqlite::params![id],
            |r| r.get(0),
        )
        .unwrap()
    }

    #[test]
    fn two_working_copies_of_one_repository_collapse_to_one_project() {
        let mut conn = open_test_index();
        let tx = conn.transaction().unwrap();
        let p = probe(
            &["r1"],
            &[("origin", "https://forge.example/acme/widget.git")],
            None,
        );
        let first = resolve_identity(&tx, &p, "widget", 100).unwrap();
        let second = resolve_identity(&tx, &p, "widget", 101).unwrap();
        tx.commit().unwrap();

        assert!(first.created);
        assert!(!second.created);
        assert_eq!(first.project_id, second.project_id);
        assert_eq!(kind(&conn, first.project_id).as_deref(), Some("strong"));
    }

    #[test]
    fn a_fork_and_its_upstream_stay_two_projects_sharing_a_lineage() {
        let mut conn = open_test_index();
        let tx = conn.transaction().unwrap();
        let up = resolve_identity(
            &tx,
            &probe(
                &["r1"],
                &[("origin", "https://forge.example/acme/widget.git")],
                None,
            ),
            "widget",
            100,
        )
        .unwrap();
        let fork = resolve_identity(
            &tx,
            &probe(
                &["r1"],
                &[("origin", "https://forge.example/mine/widget.git")],
                None,
            ),
            "widget",
            101,
        )
        .unwrap();
        tx.commit().unwrap();

        assert_ne!(up.project_id, fork.project_id);
        assert!(fork.is_fork);
        // Both sides are flagged: which of the two is "the fork" is not observable, and
        // flagging only the second would make the answer depend on walk order.
        assert_eq!(flag(&conn, up.project_id, "is_fork"), 1);
        assert_eq!(flag(&conn, fork.project_id, "is_fork"), 1);
        let shared: i64 = conn
            .query_row("SELECT COUNT(DISTINCT lineage_key) FROM project", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(shared, 1);
    }

    #[test]
    fn a_remoteless_clone_with_two_candidates_becomes_its_own_flagged_project() {
        let mut conn = open_test_index();
        let tx = conn.transaction().unwrap();
        resolve_identity(
            &tx,
            &probe(
                &["r1"],
                &[("origin", "https://forge.example/acme/widget.git")],
                None,
            ),
            "widget",
            100,
        )
        .unwrap();
        resolve_identity(
            &tx,
            &probe(
                &["r1"],
                &[("origin", "https://forge.example/mine/widget.git")],
                None,
            ),
            "widget",
            101,
        )
        .unwrap();
        let orphan = resolve_identity(&tx, &probe(&["r1"], &[], None), "widget", 102).unwrap();
        tx.commit().unwrap();

        assert!(orphan.created);
        assert!(orphan.ambiguous);
        assert_eq!(flag(&conn, orphan.project_id, "ambiguous_lineage"), 1);
        // Flagged is not failed: the project is indexed and fully usable (§1.2).
        assert_eq!(flag(&conn, orphan.project_id, "is_hidden"), 0);
    }

    #[test]
    fn a_linked_worktree_is_definitive_and_outranks_a_fork_reading() {
        let mut conn = open_test_index();
        let tx = conn.transaction().unwrap();
        let main = resolve_identity(
            &tx,
            &probe(
                &["r1"],
                &[("origin", "https://forge.example/acme/widget.git")],
                Some(b"/w/widget/.git"),
            ),
            "widget",
            100,
        )
        .unwrap();
        super::super::testutil::insert_location(
            &tx,
            main.project_id,
            "/w/widget",
            Some(b"/w/widget/.git"),
        );
        // The worktree carries a different remote, which would read as a fork on lineage alone.
        let wt = resolve_identity(
            &tx,
            &probe(
                &["r1"],
                &[("origin", "https://forge.example/mine/widget.git")],
                Some(b"/w/widget/.git"),
            ),
            "widget-wt",
            101,
        )
        .unwrap();
        tx.commit().unwrap();

        assert_eq!(wt.project_id, main.project_id);
        assert!(!wt.created);
        assert_eq!(kind(&conn, main.project_id).as_deref(), Some("definitive"));
    }

    #[test]
    fn the_footer_kind_falls_to_the_weakest_association_that_produced_a_copy() {
        let mut conn = open_test_index();
        let tx = conn.transaction().unwrap();
        let p = resolve_identity(
            &tx,
            &probe(
                &["r1"],
                &[("origin", "https://forge.example/acme/widget.git")],
                Some(b"/w/a/.git"),
            ),
            "widget",
            100,
        )
        .unwrap();
        super::super::testutil::insert_location(&tx, p.project_id, "/w/a", Some(b"/w/a/.git"));
        resolve_identity(
            &tx,
            &probe(
                &["r1"],
                &[("origin", "https://forge.example/acme/widget.git")],
                Some(b"/w/a/.git"),
            ),
            "widget",
            101,
        )
        .unwrap();
        assert_eq!(
            tx_kind(&tx, p.project_id).as_deref(),
            Some("definitive"),
            "a worktree alone reads definitive"
        );
        // Now a remoteless copy attaches by inference. The footer must not keep claiming the
        // stronger evidence.
        resolve_identity(&tx, &probe(&["r1"], &[], None), "widget", 102).unwrap();
        tx.commit().unwrap();
        assert_eq!(kind(&conn, p.project_id).as_deref(), Some("inferred"));
    }

    /// A project identity has just created has never been scanned, so §5.1's touch time is not
    /// a fact the app has. Stamping `now` there would make every new project claim one.
    #[test]
    fn a_created_project_claims_no_last_touched_at() {
        let mut conn = open_test_index();
        let tx = conn.transaction().unwrap();
        let p = resolve_identity(&tx, &probe(&["r1"], &[], None), "widget", 100).unwrap();
        tx.commit().unwrap();

        let touched: Option<i64> = conn
            .query_row(
                "SELECT last_touched_at FROM project WHERE id=?1",
                rusqlite::params![p.project_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(touched, None);
    }

    fn tx_kind(tx: &rusqlite::Transaction<'_>, id: i64) -> Option<String> {
        tx.query_row(
            "SELECT association_kind FROM project WHERE id=?1",
            rusqlite::params![id],
            |r| r.get(0),
        )
        .unwrap()
    }

    use super::super::IdentityError;
    use super::{upsert_location, LocationInput};
    use crate::derive::LocationKind;
    use crate::index::path::{PathPlatform, StoredPath};
    use crate::protocol::Presence;
    use crate::scan::discover::RepoKind;

    /// A location whose every field a test can override by editing the value it gets back.
    fn loc(kind: LocationKind, distro: Option<&str>, path: &str) -> LocationInput {
        let platform = match kind {
            LocationKind::Win => PathPlatform::Windows,
            LocationKind::Linux | LocationKind::Wsl => PathPlatform::Unix,
        };
        LocationInput {
            kind,
            distro: distro.map(ToOwned::to_owned),
            path: StoredPath::from_bytes(path.as_bytes().to_vec(), platform),
            store_key: "store-1".to_owned(),
            volume_key: Some("vol-1".to_owned()),
            presence: Presence::Present,
            repo_kind: RepoKind::WorkTree,
            common_dir_bytes: None,
            generation: 1,
            last_seen_at: None,
        }
    }

    fn location_count(conn: &rusqlite::Connection) -> i64 {
        conn.query_row("SELECT COUNT(*) FROM location", [], |r| r.get(0))
            .unwrap()
    }

    fn col<T: rusqlite::types::FromSql>(conn: &rusqlite::Connection, id: i64, c: &str) -> T {
        conn.query_row(
            &format!("SELECT {c} FROM location WHERE id=?1"),
            rusqlite::params![id],
            |r| r.get(0),
        )
        .unwrap()
    }

    /// Every test needs a project to hang a location on, and `resolve_identity` is how one is
    /// made — the two are never used apart in production either.
    fn project_for(tx: &rusqlite::Transaction<'_>, roots: &[&str], at: i64) -> i64 {
        resolve_identity(tx, &probe(roots, &[], None), "widget", at)
            .unwrap()
            .project_id
    }

    #[test]
    fn the_same_path_upserts_to_one_row_and_returns_one_id() {
        // §16 criterion 1: one repository must not appear twice on the shelf. A scan runs
        // repeatedly over the same paths, so "insert" is the wrong verb for this row.
        let mut conn = open_test_index();
        let tx = conn.transaction().unwrap();
        let p = project_for(&tx, &["r1"], 100);
        let first =
            upsert_location(&tx, p, &loc(LocationKind::Linux, None, "/w/widget"), 100).unwrap();
        let second =
            upsert_location(&tx, p, &loc(LocationKind::Linux, None, "/w/widget"), 200).unwrap();
        tx.commit().unwrap();

        assert_eq!(first, second);
        assert_eq!(location_count(&conn), 1);
    }

    #[test]
    fn a_missing_distro_is_stored_as_empty_string_because_sqlite_treats_nulls_as_distinct() {
        // §1.3 records this as a bug that shipped: `distro` was NULL for non-WSL locations, and
        // SQLite counts two NULLs as distinct inside a UNIQUE index, so `(kind, distro, path_key)`
        // silently admitted a second row for one path. `None` must reach the column as ''.
        let mut conn = open_test_index();
        let tx = conn.transaction().unwrap();
        let p = project_for(&tx, &["r1"], 100);
        let id =
            upsert_location(&tx, p, &loc(LocationKind::Linux, None, "/w/widget"), 100).unwrap();
        tx.commit().unwrap();

        let stored: Option<String> = col(&conn, id, "distro");
        assert_eq!(
            stored.as_deref(),
            Some(""),
            "'' — not NULL, which the index cannot see"
        );
    }

    #[test]
    fn one_path_in_two_distros_is_two_locations() {
        // The distro is part of a WSL location's identity: /home/me/widget in two distros is two
        // repositories that happen to spell their paths the same way.
        let mut conn = open_test_index();
        let tx = conn.transaction().unwrap();
        let p = project_for(&tx, &["r1"], 100);
        let a = upsert_location(
            &tx,
            p,
            &loc(LocationKind::Wsl, Some("alpha"), "/home/me/widget"),
            100,
        )
        .unwrap();
        let b = upsert_location(
            &tx,
            p,
            &loc(LocationKind::Wsl, Some("beta"), "/home/me/widget"),
            100,
        )
        .unwrap();
        tx.commit().unwrap();

        assert_ne!(a, b);
        assert_eq!(location_count(&conn), 2);
    }

    #[test]
    fn a_distro_on_a_non_wsl_location_is_rejected_rather_than_silently_dropped() {
        // `CHECK (kind = 'wsl' OR distro = '')` owns this rule. Normalising it away here would
        // state one rule in two places and hide the caller bug that produced it.
        let mut conn = open_test_index();
        let tx = conn.transaction().unwrap();
        let p = project_for(&tx, &["r1"], 100);
        let bad = upsert_location(
            &tx,
            p,
            &loc(LocationKind::Linux, Some("alpha"), "/w/widget"),
            100,
        );
        assert!(matches!(bad, Err(IdentityError::Sqlite(_))));
    }

    #[test]
    fn a_volume_key_of_none_is_stored_as_null_and_never_as_empty_string() {
        // A bind mount, an overlayfs or a tmpfs has no stable volume identifier. '' would be an
        // invented one that compares equal to every other invented one.
        let mut conn = open_test_index();
        let tx = conn.transaction().unwrap();
        let p = project_for(&tx, &["r1"], 100);
        let mut input = loc(LocationKind::Linux, None, "/w/widget");
        input.volume_key = None;
        let id = upsert_location(&tx, p, &input, 100).unwrap();
        tx.commit().unwrap();

        let stored: Option<String> = col(&conn, id, "volume_key");
        assert_eq!(stored, None);
    }

    #[test]
    fn a_rescan_updates_presence_generation_and_repo_kind_without_inserting() {
        // The drive came back and the checkout became a linked worktree. Same row, new facts.
        let mut conn = open_test_index();
        let tx = conn.transaction().unwrap();
        let p = project_for(&tx, &["r1"], 100);
        let mut gone = loc(LocationKind::Linux, None, "/w/widget");
        gone.presence = Presence::Offline;
        let id = upsert_location(&tx, p, &gone, 100).unwrap();

        let mut back = loc(LocationKind::Linux, None, "/w/widget");
        back.repo_kind = RepoKind::LinkedWorktree;
        back.generation = 2;
        back.last_seen_at = Some(250);
        assert_eq!(upsert_location(&tx, p, &back, 300).unwrap(), id);
        tx.commit().unwrap();

        assert_eq!(location_count(&conn), 1);
        assert_eq!(col::<String>(&conn, id, "presence"), "present");
        assert_eq!(col::<String>(&conn, id, "repo_kind"), "linked_worktree");
        assert_eq!(col::<i64>(&conn, id, "scan_generation"), 2);
        assert_eq!(col::<i64>(&conn, id, "last_seen_at"), 250);
    }

    #[test]
    fn an_observation_a_rescan_did_not_make_is_not_overwritten() {
        // §6: absence of a fact is "not computed", never zero and never stale-as-fresh. A
        // rescan writes presence and generation; it must not blank the branch and dirty flag a
        // job observed, and it must not claim them either.
        let mut conn = open_test_index();
        let tx = conn.transaction().unwrap();
        let p = project_for(&tx, &["r1"], 100);
        let id =
            upsert_location(&tx, p, &loc(LocationKind::Linux, None, "/w/widget"), 100).unwrap();
        tx.execute(
            "UPDATE location SET branch='main', is_dirty=1, worktree_observed_at=150
              WHERE id=?1",
            rusqlite::params![id],
        )
        .unwrap();

        let mut again = loc(LocationKind::Linux, None, "/w/widget");
        again.generation = 2;
        upsert_location(&tx, p, &again, 300).unwrap();
        tx.commit().unwrap();

        assert_eq!(col::<String>(&conn, id, "branch"), "main");
        assert_eq!(col::<i64>(&conn, id, "is_dirty"), 1);
        assert_eq!(col::<i64>(&conn, id, "worktree_observed_at"), 150);
    }

    #[test]
    fn the_common_dir_key_it_derives_is_the_one_worktree_owner_reads_back() {
        // The definitive-evidence loop, closed against the real writer instead of the fixture:
        // what this function stores is what §1.1's strongest evidence compares against. If the
        // two ever fold differently, a linked worktree silently reads as a fork.
        let mut conn = open_test_index();
        let tx = conn.transaction().unwrap();
        let p = project_for(&tx, &["r1"], 100);
        let mut input = loc(LocationKind::Linux, None, "/w/widget");
        input.common_dir_bytes = Some(b"/w/widget/.git".to_vec());
        upsert_location(&tx, p, &input, 100).unwrap();

        let key = StoredPath::from_bytes(b"/w/widget/.git".to_vec(), PathPlatform::Unix)
            .key()
            .to_vec();
        assert_eq!(super::worktree_owner(&tx, &key).unwrap(), Some(p));
        tx.commit().unwrap();
    }

    #[test]
    fn a_path_re_resolved_onto_another_project_moves_rather_than_duplicating() {
        // Evidence accumulates: a shallow clone with no lineage gets its own project, and once
        // deepened it resolves onto the project it belongs to. The path must follow the decision,
        // or one repository is on the shelf twice — once with the location and once without.
        let mut conn = open_test_index();
        let tx = conn.transaction().unwrap();
        let a = project_for(&tx, &["r1"], 100);
        let b = project_for(&tx, &["r2"], 101);
        let first =
            upsert_location(&tx, a, &loc(LocationKind::Linux, None, "/w/widget"), 100).unwrap();
        let second =
            upsert_location(&tx, b, &loc(LocationKind::Linux, None, "/w/widget"), 200).unwrap();
        tx.commit().unwrap();

        assert_eq!(first, second);
        assert_eq!(location_count(&conn), 1);
        assert_eq!(col::<i64>(&conn, first, "project_id"), b);
    }

    #[test]
    fn a_later_fork_flags_the_remoteless_project_that_arrived_before_it() {
        // §1.1 stores the association kind so an inference made early in a scan is not a
        // timeless predicate. In the direction phase 1 can act on — adding information — the
        // remoteless project that had one candidate, then two, becomes flagged.
        let mut conn = open_test_index();
        let tx = conn.transaction().unwrap();
        let orphan = resolve_identity(&tx, &probe(&["r1"], &[], None), "widget", 100).unwrap();
        assert!(!orphan.ambiguous);

        resolve_identity(
            &tx,
            &probe(
                &["r1"],
                &[("origin", "https://forge.example/acme/widget.git")],
                None,
            ),
            "widget",
            101,
        )
        .unwrap();
        assert_eq!(
            super::flag_remoteless_ambiguity(&tx, &lineage(&tx, orphan.project_id), 101).unwrap(),
            0,
            "one candidate is not ambiguous"
        );

        resolve_identity(
            &tx,
            &probe(
                &["r1"],
                &[("origin", "https://forge.example/mine/widget.git")],
                None,
            ),
            "widget",
            102,
        )
        .unwrap();
        tx.commit().unwrap();
        assert_eq!(flag(&conn, orphan.project_id, "ambiguous_lineage"), 1);
    }

    #[test]
    fn the_ambiguous_group_names_its_candidates_and_drops_a_project_that_lost_them() {
        let mut conn = open_test_index();
        let tx = conn.transaction().unwrap();
        resolve_identity(
            &tx,
            &probe(
                &["r1"],
                &[("origin", "https://forge.example/acme/widget.git")],
                None,
            ),
            "acme",
            100,
        )
        .unwrap();
        let mine = resolve_identity(
            &tx,
            &probe(
                &["r1"],
                &[("origin", "https://forge.example/mine/widget.git")],
                None,
            ),
            "mine",
            101,
        )
        .unwrap();
        let orphan = resolve_identity(&tx, &probe(&["r1"], &[], None), "local", 102).unwrap();
        super::super::testutil::insert_location(&tx, orphan.project_id, "/w/local", None);

        let group = super::ambiguous_group(&tx, &super::super::testutil::forge_aliases()).unwrap();
        assert_eq!(group.len(), 1);
        let row = group.first().unwrap();
        assert_eq!(row.project_id, orphan.project_id);
        assert_eq!(row.path_display.as_deref(), Some("/w/local"));
        assert_eq!(
            row.candidate_names,
            vec!["acme".to_owned(), "mine".to_owned()]
        );
        assert_eq!(row.candidate_total, 2);

        // One candidate is absorbed elsewhere. The flag is never cleared; the live query stops
        // returning the row because only one candidate is left.
        tx.execute(
            "UPDATE project SET merged_into = ?1 WHERE id = ?2",
            rusqlite::params![orphan.project_id, mine.project_id],
        )
        .unwrap();
        assert!(
            super::ambiguous_group(&tx, &super::super::testutil::forge_aliases())
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            tx.query_row(
                "SELECT ambiguous_lineage FROM project WHERE id=?1",
                rusqlite::params![orphan.project_id],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            1,
            "no repair pass — the flag stands, the query decides"
        );
        tx.commit().unwrap();
    }

    fn lineage(tx: &rusqlite::Transaction<'_>, id: i64) -> String {
        tx.query_row(
            "SELECT lineage_key FROM project WHERE id=?1",
            rusqlite::params![id],
            |r| r.get(0),
        )
        .unwrap()
    }
}
