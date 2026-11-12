//! `Discovered` → one `project` row and one `location` row, in one transaction.
//!
//! The step `core/src/scan/run.rs`'s own doc comment described and named as the gap. Until this
//! module existed a walk found repositories and persisted nothing: `resolve_identity` and
//! `identity::store::upsert_location` had no production caller at all (R1).
//!
//! **R35(a) is settled here by its second option.** `ScanStore::upsert_location` is gone from the
//! trait rather than growing a `&Transaction` parameter. R1's *"plan 07's `ScanStore` gains a
//! delegating method"* is not satisfiable: the writer needs the transaction that decided the
//! identity, and a trait method taking one leaks the identity module's transaction into the
//! scanner's seam. `SqliteScanStore::upsert_location`'s refusal — correct, and load-bearing
//! until now — goes with it.
//!
//! **R39's lock discipline is the shape of [`hand_off_discovered`], not a note on it.** Every git
//! read happens before the lock is taken; the lock covers one transaction and nothing else.
//! `jobs::run_one` has the same shape for the same reason: a twenty-second read holding the one
//! `rusqlite::Connection` would block every command.

use std::sync::Mutex;

use crate::cancel::CancelToken;
use crate::debt::store::{DebtStore as _, SqliteDebtStore};
use crate::derive::LocationKind;
use crate::exclusion::gate::{ExclusionGate, GateSite};
use crate::exclusion::store::candidate_key;
use crate::git::{
    establish_ownership, GitBackend, GitDirSelect, GitError, JobClass, JobContext, RepoHandle,
    StoreKey,
};
use crate::identity::lineage::lineage_key;
use crate::identity::probe::probe_identity;
use crate::identity::store::{
    claim_removed_row, clear_removed_on_find, resolve_identity, stored_presence, upsert_location,
    LocationInput, RemovedRowClaim,
};
use crate::identity::IdentityError;
use crate::index::path::StoredPath;
use crate::index::pending::{has_own_pending, match_pending};
use crate::index::{Index, IndexError};
use crate::motivation::absorb::{SilentAbsorb, SilentClass};
use crate::mount::StoreClass;
use crate::paths::path_bytes;
use crate::protocol::{LocationId, Presence, ProjectId};
use crate::scan::discover::{RepoKind, GIT_PROBE_TIMEOUT};
use crate::scan::run::{platform_of, Discovered};

/// The two ids the scheduler needs, and the only thing the hand-off returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Indexed {
    /// The project identity decided for the repository.
    pub project: ProjectId,
    /// The `location` row written or updated for the discovered path.
    pub location: LocationId,
}

/// Why one repository could not be indexed.
///
/// Each variant becomes a `scan_problem` at the call site. A repository the hand-off cannot read
/// is **reported**, never silently skipped: a scan that quietly drops what it could not identify
/// claims a completeness the walk did not have.
#[derive(Debug, thiserror::Error)]
pub enum HandoffError {
    /// The identity probe's git read failed or was cancelled.
    #[error("git: {0}")]
    Git(#[from] GitError),
    /// Deciding the project or writing the rows failed in the index.
    #[error("identity: {0}")]
    Identity(#[from] IndexError),
    /// A `location.kind` this build does not know — a row from a newer schema. Guessing would
    /// file a Windows path under Linux path rules (R2).
    #[error("unrecognised location kind {0:?}")]
    UnknownKind(String),
    /// Another thread panicked while holding the index mutex, so nothing was written.
    #[error("the index lock is poisoned")]
    Poisoned,
    /// A trusted hand-off's `scan_problem` row was no longer there to answer — another call took
    /// it first — so the whole transaction rolled back and nothing was written.
    #[error("that problem has already been answered")]
    ProblemGone,
    /// The exclusion list covers the repository (§56.5 rows 2 and 4): nothing was read from it,
    /// or an entry added while it was being read stopped the write.
    #[error("the folder is excluded from scanning")]
    Excluded,
}

/// Everything the hand-off needs that is not the index or the discovery itself.
pub struct HandoffCtx<'a> {
    /// The seam, never a spawned git (§15.2).
    pub git: &'a dyn GitBackend,
    /// The scan run's token, so a cancelled run stops probing.
    pub cancel: &'a CancelToken,
    /// The core's exclusion gate, asked before the first git read and again inside the
    /// transaction before the first write.
    pub exclusions: &'a ExclusionGate,
    /// `MountFacts.class`, resolved once by the walk (**R4**). Not persisted — the class is a
    /// property of the mount right now, not of the location (R27) — but the git slot pool caps
    /// on it, so a probe without it would run against a network share at a local store's
    /// concurrency.
    pub store_class: StoreClass,
    /// `location.scan_generation`: the run that saw this path (§4.6).
    pub generation: i64,
    /// The scan's clock reading, in unix seconds; every row this writes is stamped with it.
    pub now: i64,
}

impl std::fmt::Debug for HandoffCtx<'_> {
    /// By hand: `GitBackend` requires `Debug` but its contents are not a log line, and
    /// `CancelToken` is shared state whose value at format time means nothing.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HandoffCtx")
            .field("store_class", &self.store_class)
            .field("generation", &self.generation)
            .finish_non_exhaustive()
    }
}

/// Index one discovered repository: probe it, decide its project, write its row.
///
/// Idempotent through `upsert_location`'s `(kind, distro, path_key)` uniqueness: scanning the
/// same tree twice produces one `location` row and re-decides the same project.
///
/// # Errors
/// [`HandoffError`], which the caller records as a `scan_problem`.
pub fn hand_off_discovered(
    index: &Mutex<Index>,
    ctx: &HandoffCtx<'_>,
    discovered: &Discovered,
) -> Result<Indexed, HandoffError> {
    hand_off(index, ctx, discovered, None)
}

/// §11.1's `TRUST THIS REPOSITORY`: the same hand-off for a folder the user has just trusted.
///
/// Every git read carries `RepoHandle::trusted`, so `base_args` adds `-c safe.directory=<path>`
/// and nothing touches the user's git config (§17). The transaction that writes the row also
/// stamps its `location.trusted_at` — still the one trust store — and deletes the `scan_problem`
/// row `problem` names; a row already gone rolls the whole transaction back.
///
/// # Errors
/// [`HandoffError`]; [`HandoffError::ProblemGone`] when `problem` was answered first.
pub fn hand_off_trusted(
    index: &Mutex<Index>,
    ctx: &HandoffCtx<'_>,
    discovered: &Discovered,
    problem: i64,
) -> Result<Indexed, HandoffError> {
    hand_off(index, ctx, discovered, Some(problem))
}

fn hand_off(
    index: &Mutex<Index>,
    ctx: &HandoffCtx<'_>,
    discovered: &Discovered,
    trusting: Option<i64>,
) -> Result<Indexed, HandoffError> {
    let kind = LocationKind::parse(&discovered.kind)
        .ok_or_else(|| HandoffError::UnknownKind(discovered.kind.clone()))?;
    let platform = platform_of(&discovered.kind);
    admitted_to_read(index, ctx, kind, discovered)?;
    // Everything git, with **nothing locked**. A test asserts the mutex is free throughout.
    // A trusted handle passes `establish_ownership` unprobed: the user has answered that question.
    let job = JobContext::new(JobClass::Background, ctx.cancel, Some(GIT_PROBE_TIMEOUT));
    let repo = establish_ownership(
        ctx.git,
        repo_handle(discovered, ctx.store_class, trusting.is_some()),
        &job,
    )?;
    let probe = probe_identity(ctx.git, &repo, platform, &job)?;

    // `common_dir_bytes` and the probe's `common_dir_key` are **one value**: `upsert_location`
    // re-keys these bytes with the same canonicaliser `probe_identity` used on `repo.common_dir`,
    // so the key stored beside the row is the key identity was just decided on. Storing the
    // walk's bytes and keying git's answer would be the same fact written twice, drifting.
    let input = LocationInput {
        kind,
        // §1.3: the column is NOT NULL and holds `''` off WSL; `None` is what `upsert_location`
        // maps to that.
        distro: if discovered.distro.is_empty() {
            None
        } else {
            Some(discovered.distro.clone())
        },
        path: StoredPath::from_bytes(discovered.path_bytes.clone(), platform),
        store_key: discovered.store_key.clone(),
        // R27: `None` is not `""`. A mount with no stable identifier can never be recognised
        // across a remount, and inventing one hides that.
        volume_key: discovered.volume_key.clone(),
        // The walk has just looked at it. Anything else would be a claim about a directory
        // nobody visited.
        presence: Presence::Present,
        repo_kind: discovered.candidate.kind,
        common_dir_bytes: Some(path_bytes(&repo.common_dir)),
        generation: ctx.generation,
        // `upsert_location` stamps `now` when this is absent, which is the same value and one
        // fewer place to disagree.
        last_seen_at: None,
    };
    // The found repository's lineage, from the function every `project.lineage_key` was written
    // with, so a removed copy's row is recognised as this repository's own by one rule.
    let found_lineage = lineage_key(&probe.root_oids, probe.is_shallow);
    let basename = basename_of(discovered);
    let now = ctx.now;
    let aliases = crate::provider::declared_host_aliases();

    let mut problem_gone = false;
    let mut excluded = false;
    let mut guard = index.lock().map_err(|_| HandoffError::Poisoned)?;
    let indexed = guard.with_tx(|tx| {
        if !admitted_to_write(tx, ctx, kind, discovered)? {
            excluded = true;
            return Err(IndexError::Corrupt {
                detail: "the folder is excluded from scanning".to_owned(),
            });
        }
        // R1: the decision and the row it decides are **one** transaction. A committed decision
        // without its row is a repository that was discovered and then vanished.
        // §22.4: a scan that would create a row hydrates the single not-cloned project on this
        // repository's folded key instead. The fold needs the adapters' declared alias sets and
        // not an account — a scan holds none.
        let outcome =
            resolve_identity(tx, &probe, &basename, &aliases, now).map_err(as_index_error)?;
        // §46.9: a removed copy's row at this path goes back to the repository found here when it
        // is that copy's own, and otherwise keeps its project and its references. Claimed before
        // the presence read, so a repository that was never here does not read as one coming back.
        let reclaimed = matches!(
            claim_removed_row(tx, outcome.project_id, found_lineage.as_deref(), &input)
                .map_err(as_index_error)?,
            RemovedRowClaim::Reclaimed { .. }
        );
        // [p3] §30.1's unfreeze, recorded where it is first known: this write is what turns a
        // copy that was away — offline, missing, unscanned — back into `present`. Everything
        // observed there before it left is withdrawn in the same transaction, so the first sweep
        // after the return is diffed against nothing from before it (§34.2). A reclaimed copy was
        // away too: its removal left `presence` as it was, and its sweeps still read as observed.
        let stored = stored_presence(tx, &input).map_err(as_index_error)?;
        let returning = reclaimed || stored.is_some_and(|before| before != Presence::Present);
        let location =
            upsert_location(tx, outcome.project_id, &input, now).map_err(as_index_error)?;
        if let Some(problem) = trusting {
            if !answer_untrusted(tx, location, problem, now)? {
                problem_gone = true;
                return Err(IndexError::Corrupt {
                    detail: "the trusted problem row is gone".to_owned(),
                });
            }
        }
        // §46.9's revival: a reclaimed row or a new one is the removed project's repository found
        // again. A copy that was only offline coming back is neither, and revives nothing.
        if reclaimed || stored.is_none() {
            clear_removed_on_find(tx, outcome.project_id, found_lineage.as_deref(), now)
                .map_err(as_index_error)?;
        }
        if returning {
            SqliteDebtStore
                .mark_root_unobserved(tx, LocationId(location), now)
                .map_err(debt_error)?;
        }
        // §48.8.4: create, hydrate, attach and move all compute the project's subject key here,
        // so a record a rebuild staged applies in the transaction that makes it matchable —
        // committed before any job for the copy is enqueued. What it restores earns nothing
        // (§43.5); the absorb is taken only when a record is the project's, so a routine scan's
        // hand-off reads no level.
        let project = ProjectId(outcome.project_id);
        let absorb = if has_own_pending(tx, project)? {
            Some(SilentAbsorb::begin(tx, SilentClass::Restore, now)?)
        } else {
            None
        };
        match_pending(tx, project, now)?;
        if let Some(absorb) = absorb {
            absorb.finish(tx)?;
        }
        Ok(Indexed {
            project: ProjectId(outcome.project_id),
            location: LocationId(location),
        })
    });
    drop(guard);
    if excluded {
        return Err(HandoffError::Excluded);
    }
    if problem_gone {
        return Err(HandoffError::ProblemGone);
    }
    Ok(indexed?)
}

/// §56.5 row 4: the gate sits in front of the ownership probe, so a covered copy costs no git.
/// The key's root comes from `scan_root`, read under the lock and released before any git.
fn admitted_to_read(
    index: &Mutex<Index>,
    ctx: &HandoffCtx<'_>,
    kind: LocationKind,
    discovered: &Discovered,
) -> Result<(), HandoffError> {
    let key = {
        let guard = index.lock().map_err(|_| HandoffError::Poisoned)?;
        candidate_key(
            guard.conn(),
            kind,
            &discovered.distro,
            &discovered.path_bytes,
        )?
    };
    ctx.exclusions
        .check_read(GateSite::OwnershipProbe, &key)
        .map_err(|_| HandoffError::Excluded)
}

/// §56.5 row 2: an entry added while the reads ran stops the write. The check reads the gate
/// inside `tx`, under the guard no add or remove is part-way through. `false` when covered.
fn admitted_to_write(
    tx: &rusqlite::Transaction<'_>,
    ctx: &HandoffCtx<'_>,
    kind: LocationKind,
    discovered: &Discovered,
) -> Result<bool, IndexError> {
    let key = candidate_key(tx, kind, &discovered.distro, &discovered.path_bytes)?;
    Ok(ctx
        .exclusions
        .check_write(GateSite::Handoff, tx, &key)
        .is_ok())
}

/// Trust's two writes, in the hand-off's transaction: `location.trusted_at` on the row just
/// written, and the `untrusted_repo` row it answers deleted. `false` when that row is gone.
fn answer_untrusted(
    tx: &rusqlite::Transaction<'_>,
    location: i64,
    problem: i64,
    now: i64,
) -> Result<bool, IndexError> {
    let deleted = tx.execute(
        "DELETE FROM scan_problem WHERE id = ?1 AND kind = 'untrusted_repo'",
        [problem],
    )?;
    if deleted != 1 {
        return Ok(false);
    }
    tx.execute(
        "UPDATE location SET trusted_at = ?2 WHERE id = ?1",
        rusqlite::params![location, now],
    )?;
    Ok(true)
}

/// The handle the probe runs against, built from what the walk already resolved.
///
/// `RepoHandle::resolve` is deliberately not used: it re-reads `.git` and `commondir` from disk,
/// and `classify` has already done that once for this candidate. `trusted` is false for a scan,
/// because `location.trusted_at` is a column of a row that does not exist yet, and true only for
/// [`hand_off_trusted`], whose transaction writes that column.
///
/// A WSL-spelled pointer is the exception that reads `.git` again, because the candidate does not
/// record how it was selected; its ownership is established by the caller, never assumed.
fn repo_handle(discovered: &Discovered, store_class: StoreClass, trusted: bool) -> RepoHandle {
    let store = StoreKey::new(discovered.store_key.clone());
    let candidate = &discovered.candidate;
    if candidate.kind == RepoKind::Bare {
        return RepoHandle::bare(&candidate.path, store, store_class).with_trust(trusted);
    }
    RepoHandle {
        work_dir: candidate.path.clone(),
        git_dir: candidate.git_dir.clone(),
        common_dir: candidate.common_dir.clone(),
        store,
        store_class,
        trusted: false,
        git_dir_select: GitDirSelect::Discover,
    }
    .follow_wsl_pointer()
    .with_trust(trusted)
}

/// `project.name` before anything has been derived: the directory's own name.
///
/// Lossy for a non-UTF-8 path, and §11.1's `NonUtf8Path` problem row is what tells the user the
/// name they are reading is the lossy form.
fn basename_of(discovered: &Discovered) -> String {
    discovered.candidate.path.file_name().map_or_else(
        || discovered.path_display.clone(),
        |name| name.to_string_lossy().into_owned(),
    )
}

/// `Index::with_tx` commits on `Ok` and rolls back on `Err`, so an identity failure has to *be*
/// an error inside the closure — returning it as a value would commit a half-written decision.
fn as_index_error(e: IdentityError) -> IndexError {
    match e {
        IdentityError::Sqlite(e) => IndexError::Sqlite(e),
        IdentityError::Index(e) => e,
        other => IndexError::Corrupt {
            detail: format!("identity: {other:?}"),
        },
    }
}

/// The debt store's refusal, carried through the same transaction's error type.
fn debt_error(e: crate::debt::DebtError) -> IndexError {
    match e {
        crate::debt::DebtError::Index(e) => e,
        crate::debt::DebtError::Codec(detail) => IndexError::Corrupt { detail },
    }
}
