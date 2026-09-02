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
use crate::derive::LocationKind;
use crate::git::{GitBackend, GitError, JobClass, JobContext, RepoHandle, StoreKey};
use crate::identity::probe::probe_identity;
use crate::identity::store::{resolve_identity, upsert_location, LocationInput};
use crate::identity::IdentityError;
use crate::index::path::StoredPath;
use crate::index::{Index, IndexError};
use crate::mount::StoreClass;
use crate::paths::path_bytes;
use crate::protocol::{LocationId, Presence, ProjectId};
use crate::scan::discover::{RepoKind, GIT_PROBE_TIMEOUT};
use crate::scan::run::{platform_of, Discovered};

/// The two ids the scheduler needs, and the only thing the hand-off returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Indexed {
    pub project: ProjectId,
    pub location: LocationId,
}

/// Why one repository could not be indexed.
///
/// Each variant becomes a `scan_problem` at the call site. A repository the hand-off cannot read
/// is **reported**, never silently skipped: a scan that quietly drops what it could not identify
/// claims a completeness the walk did not have.
#[derive(Debug, thiserror::Error)]
pub enum HandoffError {
    #[error("git: {0}")]
    Git(#[from] GitError),
    #[error("identity: {0}")]
    Identity(#[from] IndexError),
    /// A `location.kind` this build does not know — a row from a newer schema. Guessing would
    /// file a Windows path under Linux path rules (R2).
    #[error("unrecognised location kind {0:?}")]
    UnknownKind(String),
    #[error("the index lock is poisoned")]
    Poisoned,
}

/// Everything the hand-off needs that is not the index or the discovery itself.
pub struct HandoffCtx<'a> {
    /// The seam, never a spawned git (§15.2).
    pub git: &'a dyn GitBackend,
    /// The scan run's token, so a cancelled run stops probing.
    pub cancel: &'a CancelToken,
    /// `MountFacts.class`, resolved once by the walk (**R4**). Not persisted — the class is a
    /// property of the mount right now, not of the location (R27) — but the git slot pool caps
    /// on it, so a probe without it would run against a network share at a local store's
    /// concurrency.
    pub store_class: StoreClass,
    /// `location.scan_generation`: the run that saw this path (§4.6).
    pub generation: i64,
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
    let kind = LocationKind::parse(&discovered.kind)
        .ok_or_else(|| HandoffError::UnknownKind(discovered.kind.clone()))?;
    let platform = platform_of(&discovered.kind);
    let repo = repo_handle(discovered, ctx.store_class);

    // Everything git, with **nothing locked**. A test asserts the mutex is free throughout.
    let job = JobContext::new(JobClass::Background, ctx.cancel, Some(GIT_PROBE_TIMEOUT));
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
    let basename = basename_of(discovered);
    let now = ctx.now;

    let mut guard = index.lock().map_err(|_| HandoffError::Poisoned)?;
    let indexed = guard.with_tx(|tx| {
        // R1: the decision and the row it decides are **one** transaction. A committed decision
        // without its row is a repository that was discovered and then vanished.
        let outcome = resolve_identity(tx, &probe, &basename, now).map_err(as_index_error)?;
        let location =
            upsert_location(tx, outcome.project_id, &input, now).map_err(as_index_error)?;
        Ok(Indexed {
            project: ProjectId(outcome.project_id),
            location: LocationId(location),
        })
    });
    drop(guard);
    Ok(indexed?)
}

/// The handle the probe runs against, built from what the walk already resolved.
///
/// `RepoHandle::resolve` is deliberately not used: it re-reads `.git` and `commondir` from disk,
/// and `classify` has already done that once for this candidate. `trusted` is false because
/// `location.trusted_at` is a column of a row that does not exist yet.
fn repo_handle(discovered: &Discovered, store_class: StoreClass) -> RepoHandle {
    let store = StoreKey::new(discovered.store_key.clone());
    let candidate = &discovered.candidate;
    if candidate.kind == RepoKind::Bare {
        return RepoHandle::bare(&candidate.path, store, store_class);
    }
    RepoHandle {
        work_dir: candidate.path.clone(),
        git_dir: candidate.git_dir.clone(),
        common_dir: candidate.common_dir.clone(),
        store,
        store_class,
        trusted: false,
    }
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
