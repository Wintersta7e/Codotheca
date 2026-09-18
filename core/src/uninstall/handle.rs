//! §24.7's two commands, assembled.
//!
//! **This is the layer p2-24b's route arms named and did not build.** `compute_verdict` and
//! `uninstall_location` were complete; what nobody wrote was the code that fills a
//! `VerdictInputs` — the row, the roots, the live session, the uniqueness analysis and §24.7C's
//! in-session fetch — so both commands answered `Route::NoOwner` and the whole feature was
//! unreachable at the core's own dispatcher.
//!
//! **Neither handler holds the index mutex across the network.** §24.7C requires a fetch
//! immediately before the verdict, and `SqliteScanStore` takes the same `std::sync::Mutex`, which
//! is not reentrant — so these take the guard, read, drop it, go to the network, and take it
//! again. It is `readme::handle_readme_assets_off_lock`'s shape, for the same reason.
//!
//! **The verdict is computed once per call, by `compute_verdict`, from inputs assembled once.**
//! Two computations at two freshnesses is the drift this project has already paid for four times,
//! and on a safety verdict it is the difference between refusing a removal and performing one.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use serde_json::Value;

use crate::git::{GitBackend, JobClass, JobContext, RepoHandle, StoreKey};
use crate::gitw::backend::MutatingGit;
use crate::gitw::intent::{Intent, RemoteName};
use crate::index::Index;
use crate::mount::StoreClass;
use crate::proto::dispatch::{parse_args, CommandFailure};
use crate::protocol::{
    LocationDetail, LocationId, LocationsUninstallArgs, LocationsUninstallPreflightArgs,
};
use crate::uninstall::gates::{self, RemoteOutcome};
use crate::uninstall::preflight::{compute_verdict, LocationSnapshot, VerdictInputs};
use crate::uninstall::{uninstall_location, unique};

/// §24.7C's fetch runs while a user waits, so it carries a deadline rather than blocking the
/// core on an unresponsive remote. The analyser's git calls carry the same one.
const PREFLIGHT_DEADLINE: Duration = Duration::from_secs(20);

/// The remote a fetch verifies against. §24.1a's `RemoteName` refuses anything that is not one
/// safe segment, so this cannot become a path.
const ORIGIN: &str = "origin";

fn internal(e: impl std::fmt::Display) -> CommandFailure {
    CommandFailure::internal(e.to_string())
}

/// One row, named rather than positional. A seven-tuple is `type_complexity` to clippy and seven
/// positions to keep in order to a reader.
#[derive(Debug, Clone)]
struct RowRead {
    path_bytes: Vec<u8>,
    refstate_observed_at: Option<i64>,
    worktree_observed_at: Option<i64>,
    removed_at: Option<i64>,
    is_shallow: bool,
    trusted: bool,
    store_key: String,
}

/// Everything read under the guard, so the guard can be dropped before the network.
#[derive(Debug, Clone)]
struct RowFacts {
    snapshot: LocationSnapshot,
    roots: Vec<PathBuf>,
    live_session: bool,
    store: StoreKey,
    trusted: bool,
}

/// The one read. **`is_shallow` comes from `project`** — §24.7B's gate is about the graph, and
/// the graph belongs to the repository rather than to one checkout of it.
// **A read transaction, not a bare connection**: `gate_live_session` takes a `&Transaction`,
// and every rusqlite transaction in this core opens through `TxGuard`.
fn read_row(index: &mut Index, id: LocationId) -> Result<RowFacts, CommandFailure> {
    index
        .with_tx(|tx| {
            // Mapped straight into the snapshot rather than through a seven-tuple, which clippy
            // reads as `type_complexity` and a reader reads as seven positions to keep in order.
            let row = tx.query_row(
                "SELECT l.path_bytes, l.refstate_observed_at, l.worktree_observed_at, l.removed_at,
                    p.is_shallow, l.trusted_at, l.store_key
               FROM location l JOIN project p ON p.id = l.project_id
              WHERE l.id = ?1",
                [id.0],
                |r| {
                    Ok(RowRead {
                        path_bytes: r.get(0)?,
                        refstate_observed_at: r.get(1)?,
                        worktree_observed_at: r.get(2)?,
                        removed_at: r.get(3)?,
                        is_shallow: r.get::<_, i64>(4)? != 0,
                        trusted: r.get::<_, Option<i64>>(5)?.is_some(),
                        store_key: r.get(6)?,
                    })
                },
            )?;

            // Every configured root, enabled or not: §24.7D refuses a path that **is** a scan root or
            // sits outside every one of them, and a disabled root is still a root the user named.
            let mut stmt = tx.prepare("SELECT path_bytes FROM scan_root")?;
            let roots = stmt
                .query_map([], |r| r.get::<_, Vec<u8>>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?
                .iter()
                .map(|bytes| crate::paths::path_from_bytes(bytes))
                .collect();

            let live_session = gates::gate_live_session(tx, id)?.is_some();

            Ok(RowFacts {
                snapshot: LocationSnapshot {
                    id,
                    path: crate::paths::path_from_bytes(&row.path_bytes),
                    refstate_observed_at: row.refstate_observed_at,
                    worktree_observed_at: row.worktree_observed_at,
                    is_shallow: row.is_shallow,
                    removed_at: row.removed_at,
                },
                roots,
                live_session,
                store: StoreKey::new(row.store_key),
                trusted: row.trusted,
            })
        })
        .map_err(|e| match e {
            crate::index::IndexError::Sqlite(rusqlite::Error::QueryReturnedNoRows) => {
                CommandFailure::protocol(format!("no location {}", id.0))
            }
            other => internal(other),
        })
}

/// §24.7C: **verified, not believed.**
///
/// A fetch that reached the remote is the only thing that yields `Reached`. Everything else is a
/// class of not-knowing, and every one of them behaves as unsafe — a 401, 403 or 404 is *unknown*
/// and never *gone*, which is the specific mistake that would turn this into a shredder.
fn verify_remote_live(
    repo: &RepoHandle,
    git: &dyn GitBackend,
    write_git: &dyn MutatingGit,
    ctx: &JobContext<'_>,
) -> RemoteOutcome {
    // A remote on this machine is a mirror, not a backup, and it is recognised before the fetch:
    // fetching from it would succeed and prove nothing.
    match git.remote_urls(repo, ctx) {
        Ok(urls) => {
            // `(name, url)` pairs: the URL is what a mirror check reads, never the remote name.
            if urls.iter().any(|(_, url)| gates::is_local_mirror(url)) {
                return RemoteOutcome::LocalMirror;
            }
            if urls.is_empty() {
                // No remote at all: nothing upstream can hold this work.
                return RemoteOutcome::Unreachable;
            }
        }
        // A config that could not be read is a remote that was not established.
        Err(_) => return RemoteOutcome::Unreachable,
    }

    let Ok(remote) = RemoteName::parse(ORIGIN) else {
        return RemoteOutcome::Unreachable;
    };
    let intent = Intent::Fetch {
        work_dir: repo.work_dir.clone(),
        remote,
    };
    // The child's stderr is read and dropped: §24.7C needs *whether* the remote answered, and the
    // core's own stdout carries protocol frames and nothing else.
    let mut sink = |_: &str| {};
    match write_git.run(&intent, ctx.cancel, &mut sink) {
        Ok(()) => RemoteOutcome::Reached,
        // A refusal and a failure to arrive are different facts and §24.7C keeps them apart, but
        // both behave as unsafe. `GitError` does not carry an HTTP status, so the distinction is
        // drawn where it is available: a permission error is a refusal, everything else is a
        // remote that was never reached.
        Err(crate::git::GitError::PermissionDenied { .. }) => RemoteOutcome::Refused,
        Err(_) => RemoteOutcome::Unreachable,
    }
}

/// The uniqueness analysis and the remote check, both off the index guard.
fn assemble(
    facts: &RowFacts,
    git: &dyn GitBackend,
    write_git: &dyn MutatingGit,
    now: i64,
) -> VerdictInputs {
    let cancel = crate::cancel::CancelToken::new();
    let ctx = JobContext::new(JobClass::Interactive, &cancel, Some(PREFLIGHT_DEADLINE));

    // §24.7E begins here: the directory is **re-resolved from disk**, never reconstructed from
    // the row. A handle that could not be resolved is a copy nothing can reason about, and the
    // analyser's `NeverObserved` is the honest answer rather than an empty blocker list.
    let Ok(mut repo) =
        RepoHandle::resolve(&facts.snapshot.path, facts.store.clone(), StoreClass::Local)
    else {
        return VerdictInputs {
            snapshot: facts.snapshot.clone(),
            roots: facts.roots.clone(),
            remote: RemoteOutcome::Unreachable,
            unique: vec![crate::protocol::UninstallBlocker::NeverObserved],
            live_session: facts.live_session,
            now,
        };
    };
    repo.trusted = facts.trusted;

    let mut blockers = unique::analyse_refs(&repo, git, &ctx);
    blockers.extend(unique::analyse_worktree(&repo, git, &ctx));
    blockers.extend(unique::analyse_nested(&repo, git, &ctx, 0));

    VerdictInputs {
        snapshot: facts.snapshot.clone(),
        roots: facts.roots.clone(),
        remote: verify_remote_live(&repo, git, write_git, &ctx),
        unique: blockers,
        live_session: facts.live_session,
        now,
    }
}

/// §24.7's pre-flight. **Read-only, unprivileged, and not callable on hover** — it fetches.
///
/// # Errors
/// `PROTOCOL` for an argument shape the schema does not admit or an id that names no location;
/// `INTERNAL` for an index fault. A *blocked* verdict is a success, not an error.
pub fn handle_preflight_off_lock(
    index: &Arc<Mutex<Index>>,
    git: &dyn GitBackend,
    write_git: &dyn MutatingGit,
    args: Value,
    now: i64,
) -> Result<Value, CommandFailure> {
    let a: LocationsUninstallPreflightArgs = parse_args(args)?;
    let facts = {
        let mut guard = index.lock().unwrap_or_else(PoisonError::into_inner);
        read_row(&mut guard, a.location_id)?
    };
    let inputs = assemble(&facts, git, write_git, now);
    let (verdict, _seal) = compute_verdict(&inputs)?;
    serde_json::to_value(verdict).map_err(internal)
}

/// §24.8's removal. **The verdict it acts on is the one it computes, inside this call.**
///
/// A verdict rendered thirty seconds ago is a cache, and *never claim currency you do not have*
/// applies to a safety verdict more than to anything else in this product. Nothing about the
/// verdict crosses a call boundary to get here: the argument is a `LocationId`.
///
/// # Errors
/// `PROTOCOL` when the freshly computed verdict is not `safe`, when the directory's identity no
/// longer matches, or when the removal itself refuses; `INTERNAL` for an index fault.
pub fn handle_uninstall_off_lock(
    index: &Arc<Mutex<Index>>,
    git: &dyn GitBackend,
    write_git: &dyn MutatingGit,
    args: Value,
    now: i64,
) -> Result<Value, CommandFailure> {
    let a: LocationsUninstallArgs = parse_args(args)?;
    let facts = {
        let mut guard = index.lock().unwrap_or_else(PoisonError::into_inner);
        read_row(&mut guard, a.location_id)?
    };
    let inputs = assemble(&facts, git, write_git, now);

    // §24.7E: the **root-commit SHA**, re-derived from the directory at removal time and never
    // remembered. `head_oid` is a tip — a position, not an identity — and a guard over it would
    // refuse a copy the user had merely committed to and admit one rewound onto the same tip.
    let cancel = crate::cancel::CancelToken::new();
    let ctx = JobContext::new(JobClass::Interactive, &cancel, Some(PREFLIGHT_DEADLINE));
    let identity_now =
        RepoHandle::resolve(&facts.snapshot.path, facts.store.clone(), StoreClass::Local)
            .ok()
            .and_then(|repo| git.root_commits(&repo, &ctx).ok())
            .and_then(|roots| roots.into_iter().next());

    // **The seal comes from the same computation the warrant is built on**, and
    // `uninstall_location` recomputes once more as the authority. Three computations at three
    // freshnesses would be the drift this module exists to refuse, so this is the only one here.
    let (_verdict, seal) = compute_verdict(&inputs)?;
    let expected = identity_now.clone().ok_or_else(|| {
        // §24.7E: no re-derived identity is nothing to match the row against, and an unmatched
        // identity is a refusal rather than a removal performed on trust.
        CommandFailure::protocol(
            "uninstall refused: the directory's root commit could not be re-derived".to_owned(),
        )
    })?;
    let warrant = crate::removal::Warrant::for_uninstall(
        inputs.snapshot.id,
        inputs.snapshot.path.clone(),
        expected,
        seal,
    );

    // **The refusal is carried out as a value, not flattened into an `IndexError`.** §2.2's code
    // is the whole answer here — a refused removal is `PROTOCOL`, and a retry is pointless —
    // whereas an `IndexError` reads as a fault in the index and carries `INTERNAL`.
    //
    // Committing an unchanged transaction on that path is safe, and it is safe for a stated
    // reason rather than by luck: `uninstall_location` writes **once**, in a single `UPDATE`
    // after the bytes are already gone (`core/src/uninstall/command.rs`), so every refusal
    // returns before anything is written and there is no partial state to roll back. A second
    // write added there would break that, which is what this note is for.
    let mut guard = index.lock().unwrap_or_else(PoisonError::into_inner);
    let removed = guard
        .with_tx(|tx| {
            Ok(uninstall_location(
                tx,
                &inputs,
                &warrant,
                identity_now.as_ref(),
            ))
        })
        .map_err(internal)??;
    // The same projection RELOCATE returns — the one the page already reads, never a second one.
    let detail: LocationDetail =
        crate::detail::get::location_detail(guard.conn(), removed.location)?;
    serde_json::to_value(detail).map_err(internal)
}
