//! §24.7's two commands, assembled.
//!
//! **This is the layer p2-24b's route arms named and did not build.** `compute_verdict` and
//! `uninstall_location` were complete; what nobody wrote was the code that fills a
//! `VerdictInputs` — the row, the roots, the live session, the uniqueness analysis and the remote
//! check — so both commands answered `Route::NoOwner` and the whole feature was unreachable at
//! the core's own dispatcher.
//!
//! **Neither handler holds the index mutex across the network.** §47.4's verifying read runs
//! immediately before the verdict, and `SqliteScanStore` takes the same `std::sync::Mutex`, which
//! is not reentrant — so these take the guard, read, drop it, go to the network, and take it
//! again. It is `readme::handle_readme_assets_off_lock`'s shape, for the same reason.
//!
//! **The verdict is computed once per call, by `compute_verdict`, from inputs assembled once.**
//! Two computations at two freshnesses is the drift this project has already paid for four times,
//! and on a safety verdict it is the difference between refusing a removal and performing one.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

use serde_json::Value;

use crate::analyser::identity::{identify, live_identity, IdentityOutcome, LiveIdentity};
use crate::analyser::remote::{GitRemoteVerifier, RemoteReading, RemoteVerifier as _};
use crate::analyser::{read_location_row, LocationRow};
use crate::git::{GitBackend, JobClass, JobContext, RepoHandle};
use crate::gitw::backend::MutatingGit;
use crate::gitw::intent::GIT_INVOCATION_DEADLINE;
use crate::index::Index;
use crate::mount::StoreClass;
use crate::proto::dispatch::{parse_args, CommandFailure};
use crate::protocol::{
    LocationDetail, LocationId, LocationsUninstallArgs, LocationsUninstallPreflightArgs,
    UninstallBlocker,
};
use crate::removal::Trash;
use crate::uninstall::gates::{self, RemoteOutcome};
use crate::uninstall::preflight::{
    compute_verdict, stopped_verdict, LocationSnapshot, VerdictInputs,
};
use crate::uninstall::{uninstall_location, unique};

fn internal(e: impl std::fmt::Display) -> CommandFailure {
    CommandFailure::internal(e.to_string())
}

/// Everything read under the guard, so the guard can be dropped before the network.
#[derive(Debug, Clone)]
struct RowFacts {
    row: LocationRow,
    roots: Vec<PathBuf>,
    live_session: bool,
}

/// The one row read, the scan roots and the live session, in one read transaction.
// **A read transaction, not a bare connection**: `gate_live_session` takes a `&Transaction`,
// and every rusqlite transaction in this core opens through `TxGuard`.
fn read_row(index: &mut Index, id: LocationId) -> Result<RowFacts, CommandFailure> {
    index
        .with_tx(|tx| {
            let row = match read_location_row(tx, id) {
                Ok(row) => row,
                Err(failure) => return Ok(Err(failure)),
            };

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

            Ok(Ok(RowFacts {
                row,
                roots,
                live_session,
            }))
        })
        .map_err(internal)?
}

/// The directory, re-resolved from disk (§24.7E) — never reconstructed from the row.
fn resolve(row: &LocationRow) -> Option<RepoHandle> {
    let mut repo = RepoHandle::resolve(&row.path, row.store.clone(), StoreClass::Local).ok()?;
    repo.trusted = row.trusted;
    Some(repo)
}

/// A context carrying the per-invocation deadline.
const fn interactive(cancel: &crate::cancel::CancelToken) -> JobContext<'_> {
    JobContext::new(JobClass::Interactive, cancel, Some(GIT_INVOCATION_DEADLINE))
}

/// §47.4: **verified, not believed** — every configured remote, one at a time, through the
/// verifying read, which writes objects and no ref.
///
/// **Interim, until the analyser's own composition lands.** The readings are folded to phase 2's
/// `RemoteOutcome`: any network remote that did not answer — a not-admitted transport included —
/// is `Unreachable`; only same-machine remotes is `LocalMirror`; no remote at all is
/// `Unreachable`; otherwise `Reached`. Uniqueness is still computed before this read, against
/// tracking refs the read no longer refreshes, so the answer can only block more: the safe
/// direction.
fn verify_remotes(
    repo: &RepoHandle,
    git: &dyn GitBackend,
    write_git: &dyn MutatingGit,
    ctx: &JobContext<'_>,
) -> RemoteOutcome {
    let Ok(urls) = git.remote_urls(repo, ctx) else {
        // A config that could not be read is a remote that was not established.
        return RemoteOutcome::Unreachable;
    };
    // `(name, url)` pairs, one per configured URL: the names, once each, in config order. The
    // URL here is the configured one; §45.3(a) classifies the effective one the read resolves.
    let mut names: Vec<String> = Vec::new();
    for (name, _) in urls {
        if !names.contains(&name) {
            names.push(name);
        }
    }
    if names.is_empty() {
        // No remote at all: nothing upstream can hold this work.
        return RemoteOutcome::Unreachable;
    }
    let verifier = GitRemoteVerifier::new(write_git, git);
    let mut network = false;
    let mut all_answered = true;
    for name in &names {
        match verifier.read(repo, name, ctx) {
            RemoteReading::SameMachine => {}
            RemoteReading::Answered { .. } => network = true,
            RemoteReading::DidNotAnswer(_) => {
                network = true;
                all_answered = false;
            }
        }
    }
    if !all_answered {
        RemoteOutcome::Unreachable
    } else if network {
        RemoteOutcome::Reached
    } else {
        RemoteOutcome::LocalMirror
    }
}

/// What the analysis produced: the inputs to the verdict, or the blocker step 1 stopped at.
enum Assembled {
    Inputs(VerdictInputs),
    Stopped(UninstallBlocker),
}

/// §45.6 step 1, then the uniqueness analysis and the remote check, all off the index guard.
fn assemble(
    facts: &RowFacts,
    git: &dyn GitBackend,
    write_git: &dyn MutatingGit,
    now: i64,
) -> Assembled {
    let cancel = crate::cancel::CancelToken::new();
    let ctx = interactive(&cancel);
    let row = &facts.row;
    let mut snapshot = LocationSnapshot {
        id: row.id,
        path: row.path.clone(),
        refstate_observed_at: row.refstate_observed_at,
        worktree_observed_at: row.worktree_observed_at,
        is_shallow: false,
        removed_at: row.removed_at,
    };

    // A handle that could not be resolved is a copy nothing can reason about, and the
    // analyser's `NeverObserved` is the honest answer rather than an empty blocker list.
    let Some(repo) = resolve(row) else {
        return Assembled::Inputs(VerdictInputs {
            snapshot,
            roots: facts.roots.clone(),
            remote: RemoteOutcome::Unreachable,
            unique: vec![UninstallBlocker::NeverObserved],
            live_session: facts.live_session,
            now,
        });
    };

    // §45.6 step 1: the live lineage against the **row's**, read in this call. A mismatch or a
    // failed derivation stops the analysis here; a live shallow copy continues as
    // `shallow_clone` (D-2), read live rather than from `project.is_shallow` (§45.7).
    match identify(git, &repo, row.lineage_key.as_deref(), &ctx) {
        IdentityOutcome::Match => {}
        IdentityOutcome::Shallow => snapshot.is_shallow = true,
        IdentityOutcome::Mismatch => return Assembled::Stopped(UninstallBlocker::RefusedPath),
        IdentityOutcome::Unreadable => return Assembled::Stopped(UninstallBlocker::RefsUnreadable),
    }

    let mut blockers = unique::analyse_refs(&repo, git, &ctx);
    blockers.extend(unique::analyse_worktree(&repo, git, &ctx));
    blockers.extend(unique::analyse_nested(&repo, git, &ctx, 0));

    Assembled::Inputs(VerdictInputs {
        snapshot,
        roots: facts.roots.clone(),
        remote: verify_remotes(&repo, git, write_git, &ctx),
        unique: blockers,
        live_session: facts.live_session,
        now,
    })
}

/// §24.7's pre-flight. **Unprivileged, and not callable on hover** — its only write is objects,
/// through §47.4's verifying read, and it goes to the network (R186).
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
    let (verdict, _seal) = match assemble(&facts, git, write_git, now) {
        Assembled::Inputs(inputs) => compute_verdict(&inputs)?,
        Assembled::Stopped(blocker) => stopped_verdict(blocker, &facts.row.path, now),
    };
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
    trash: &dyn Trash,
    args: Value,
    now: i64,
) -> Result<Value, CommandFailure> {
    let a: LocationsUninstallArgs = parse_args(args)?;
    let facts = {
        let mut guard = index.lock().unwrap_or_else(PoisonError::into_inner);
        read_row(&mut guard, a.location_id)?
    };
    let inputs = match assemble(&facts, git, write_git, now) {
        Assembled::Inputs(inputs) => inputs,
        Assembled::Stopped(blocker) => {
            let (verdict, _seal) = stopped_verdict(blocker, &facts.row.path, now);
            return Err(CommandFailure::protocol(format!(
                "uninstall refused: {:?} — {:?}",
                verdict.disposition, verdict.blockers
            )));
        }
    };

    // **The seal comes from the same computation the warrant is built on**, and
    // `uninstall_location` recomputes once more as the authority. Three computations at three
    // freshnesses would be the drift this module exists to refuse, so this is the only one here.
    let (_verdict, seal) = compute_verdict(&inputs)?;

    // §24.7E against the **row** (§45.6 step 1): the warrant expects the row's
    // `project.lineage_key`, and the identity re-derived from the directory at removal time is
    // compared with it inside `remove_warranted`. Phase 2 built `expected` from the directory it
    // then checked, so a replaced directory matched itself (§37.8).
    let warrant = crate::removal::Warrant::for_uninstall(
        inputs.snapshot.id,
        inputs.snapshot.path.clone(),
        facts.row.lineage_key.clone(),
        seal,
    );
    let cancel = crate::cancel::CancelToken::new();
    let identity_now = resolve(&facts.row).map_or(LiveIdentity::Underivable, |repo| {
        live_identity(git, &repo, &interactive(&cancel))
    });

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
                trash,
                &identity_now,
            ))
        })
        .map_err(internal)??;
    // The same projection RELOCATE returns — the one the page already reads, never a second one.
    let detail: LocationDetail =
        crate::detail::get::location_detail(guard.conn(), removed.location)?;
    drop(guard);
    serde_json::to_value(detail).map_err(internal)
}
