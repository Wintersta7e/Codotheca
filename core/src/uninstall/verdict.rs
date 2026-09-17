//! §24.8's verdict: **never a boolean**.
//!
//! Every failing condition is named, so a surface states *why* a control is disabled instead of
//! only disabling it. `blockers` always carries **every** blocker found, of both classes, so the
//! fold never hides one.
//!
//! **`unknown` is distinct from `blocked` at the type level, not by convention.** Collapsing them
//! would render an absence as a fact — the *never render unknown as zero* invariant, in the type
//! system. `blocked` wins when both are present, because naming a concrete problem is more useful
//! than naming an absence, and it changes nothing about reachability: **neither disposition
//! reaches removal.**

use crate::protocol::{UninstallBlocker, UninstallDisposition};

/// Is this blocker a known-bad fact, or an absence of knowledge?
///
/// An exhaustive `match` with **no wildcard arm**: a fifteenth variant fails to compile rather
/// than falling silently into one class or the other.
const fn is_unknown(blocker: UninstallBlocker) -> bool {
    match blocker {
        // Four unknowns: the app could not establish the fact, which is not the same as
        // establishing that the fact is fine.
        UninstallBlocker::ShallowClone
        | UninstallBlocker::RemoteUnreachable
        | UninstallBlocker::StashUnreadable
        | UninstallBlocker::NeverObserved => true,
        // Ten known-bad: each one is something the app read and can name.
        UninstallBlocker::UnpushedCommits
        | UninstallBlocker::UncommittedChanges
        | UninstallBlocker::StashPresent
        | UninstallBlocker::UntrackedPrecious
        | UninstallBlocker::IgnoredPrecious
        | UninstallBlocker::SubmoduleUnsafe
        | UninstallBlocker::LinkedWorktree
        | UninstallBlocker::LiveSession
        | UninstallBlocker::RefusedPath
        | UninstallBlocker::RemoteIsLocalMirror => false,
    }
}

/// §24.8's fold, ruled by this plan because the spec states the type and not the arithmetic.
///
/// | Inputs | Disposition |
/// |---|---|
/// | no blockers | `safe` |
/// | at least one known-bad | `blocked` |
/// | only unknowns | `unknown` |
#[must_use]
pub fn fold_disposition(blockers: &[UninstallBlocker]) -> UninstallDisposition {
    if blockers.is_empty() {
        return UninstallDisposition::Safe;
    }
    if blockers.iter().any(|b| !is_unknown(*b)) {
        return UninstallDisposition::Blocked;
    }
    UninstallDisposition::Unknown
}

/// The digest of the inputs a verdict was computed from.
///
/// **In-core only, and it never crosses a call boundary** (§24.8). The renderer passes a
/// `LocationId` and receives an `UninstallVerdict` with no token in it — a token on the wire
/// would be a capability the renderer could hold and replay. This exists only so
/// `locations.uninstall` can compare its own freshly computed verdict against the one it computed
/// a moment earlier **in the same call**, which is what makes *recomputed inside* checkable
/// rather than asserted.
///
/// No `Serialize`, no `Deserialize`, and `core/tests/uninstall_verdict.rs` asserts the name
/// appears in no generated file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerdictSeal(String);

impl VerdictSeal {
    /// Seal a verdict's inputs. Two verdicts over the same inputs seal equal; any difference in
    /// the blocker set or its order of discovery does not — the set is sorted first, because the
    /// *order* blockers were found in is not an input to the decision.
    #[must_use]
    pub fn of(blockers: &[UninstallBlocker], disposition: UninstallDisposition) -> Self {
        let mut names: Vec<&'static str> = blockers.iter().map(|b| slug(*b)).collect();
        names.sort_unstable();
        names.dedup();
        Self(format!(
            "{}|{}",
            disposition_slug(disposition),
            names.join(",")
        ))
    }
}

const fn disposition_slug(disposition: UninstallDisposition) -> &'static str {
    match disposition {
        UninstallDisposition::Safe => "safe",
        UninstallDisposition::Blocked => "blocked",
        UninstallDisposition::Unknown => "unknown",
    }
}

const fn slug(blocker: UninstallBlocker) -> &'static str {
    match blocker {
        UninstallBlocker::UnpushedCommits => "unpushed_commits",
        UninstallBlocker::UncommittedChanges => "uncommitted_changes",
        UninstallBlocker::StashPresent => "stash_present",
        UninstallBlocker::UntrackedPrecious => "untracked_precious",
        UninstallBlocker::IgnoredPrecious => "ignored_precious",
        UninstallBlocker::SubmoduleUnsafe => "submodule_unsafe",
        UninstallBlocker::LinkedWorktree => "linked_worktree",
        UninstallBlocker::ShallowClone => "shallow_clone",
        UninstallBlocker::RemoteUnreachable => "remote_unreachable",
        UninstallBlocker::RemoteIsLocalMirror => "remote_is_local_mirror",
        UninstallBlocker::StashUnreadable => "stash_unreadable",
        UninstallBlocker::LiveSession => "live_session",
        UninstallBlocker::RefusedPath => "refused_path",
        UninstallBlocker::NeverObserved => "never_observed",
    }
}

/// Every blocker, for the partition test and for anything that must iterate them.
pub const ALL_BLOCKERS: [UninstallBlocker; 14] = [
    UninstallBlocker::UnpushedCommits,
    UninstallBlocker::UncommittedChanges,
    UninstallBlocker::StashPresent,
    UninstallBlocker::UntrackedPrecious,
    UninstallBlocker::IgnoredPrecious,
    UninstallBlocker::SubmoduleUnsafe,
    UninstallBlocker::LinkedWorktree,
    UninstallBlocker::ShallowClone,
    UninstallBlocker::RemoteUnreachable,
    UninstallBlocker::RemoteIsLocalMirror,
    UninstallBlocker::StashUnreadable,
    UninstallBlocker::LiveSession,
    UninstallBlocker::RefusedPath,
    UninstallBlocker::NeverObserved,
];

/// Whether a blocker is an unknown, for callers outside this module.
#[must_use]
pub const fn is_unknown_blocker(blocker: UninstallBlocker) -> bool {
    is_unknown(blocker)
}
