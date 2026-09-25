//! §24.7's verdict, assembled **once**.
//!
//! `compute_verdict` is the one function both commands call. Two verdict computations at two
//! freshnesses is the drift defect this project has already paid for four times, and on a safety
//! verdict it would be the difference between refusing a removal and performing one.

use std::path::PathBuf;

use crate::proto::dispatch::CommandFailure;
use crate::protocol::{LocationId, UninstallBlocker, UninstallVerdict};
use crate::removal::{SystemTrash, Trash as _, TrashAvailability};
use crate::uninstall::gates;
use crate::uninstall::verdict::{fold_disposition, VerdictSeal};

/// One location's facts, read from the row before any network call.
#[derive(Debug, Clone)]
pub struct LocationSnapshot {
    /// The `location` row these facts were read from.
    pub id: LocationId,
    /// The working copy's directory: what would be removed.
    pub path: PathBuf,
    /// When ref state was last observed; `None` blocks the verdict as never observed (§24.7G).
    pub refstate_observed_at: Option<i64>,
    /// When the worktree was last observed; `None` blocks the verdict the same way.
    pub worktree_observed_at: Option<i64>,
    /// Whether the project's history is shallow, which blocks the verdict (§24.7B).
    pub is_shallow: bool,
    /// When this copy was uninstalled, or `None` while its bytes are on disk.
    pub removed_at: Option<i64>,
}

/// Everything the verdict is computed from that is not the row.
#[derive(Debug, Clone)]
pub struct VerdictInputs {
    /// The row's own facts.
    pub snapshot: LocationSnapshot,
    /// The configured scan roots, for §24.7D's containment check.
    pub roots: Vec<PathBuf>,
    /// What the live remote check established (§24.7C).
    pub remote: gates::RemoteOutcome,
    /// Blockers the uniqueness analyser found (§24.7A).
    pub unique: Vec<UninstallBlocker>,
    /// A live launch session on this location (§24.7D).
    pub live_session: bool,
    /// The clock reading, in unix seconds: the verdict's `computedAt` and a removal's
    /// `removed_at`.
    pub now: i64,
}

/// §24.7's verdict and its in-core seal.
///
/// **Every gate contributes, and every blocker found is reported** — the fold decides a
/// disposition, it never edits the list. A surface that only learned *blocked* could not say why.
///
/// # Errors
/// Fails only when a fact could not be read at all; a *blocked* verdict is a success, not an error.
pub fn compute_verdict(
    inputs: &VerdictInputs,
) -> Result<(UninstallVerdict, VerdictSeal), CommandFailure> {
    let mut blockers: Vec<UninstallBlocker> = Vec::new();

    // §24.7G first: a location nobody has looked at cannot be reasoned about at all, and every
    // gate below would be reasoning from absent observations.
    blockers.extend(gates::gate_first_day(
        inputs.snapshot.refstate_observed_at,
        inputs.snapshot.worktree_observed_at,
    ));
    blockers.extend(gates::gate_shallow(inputs.snapshot.is_shallow));
    blockers.extend(gates::gate_path(&inputs.snapshot.path, &inputs.roots));
    if inputs.live_session {
        blockers.push(UninstallBlocker::LiveSession);
    }
    blockers.extend(inputs.unique.iter().copied());

    let verification = gates::verify_remote(inputs.remote, inputs.now);
    blockers.extend(verification.blockers.iter().copied());

    // One blocker of a kind is enough; the list is a vocabulary, not a tally.
    blockers.sort_unstable_by_key(|b| format!("{b:?}"));
    blockers.dedup();

    let disposition = fold_disposition(&blockers);
    let seal = VerdictSeal::of(&blockers, disposition);

    // §24.7F: the copy says what will happen **before the click**.
    let trash_available = matches!(
        SystemTrash.availability(&inputs.snapshot.path),
        TrashAvailability::Available
    );

    Ok((
        UninstallVerdict {
            disposition,
            blockers,
            remote_verified_at: verification.verified_at,
            trash_available,
            computed_at: inputs.now,
        },
        seal,
    ))
}
