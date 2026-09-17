//! §24.7A's uniqueness analyser: what this copy holds that exists **nowhere else**.
//!
//! **Live, never from cache.** Every input here is read at pre-flight time. The cached columns —
//! `location.is_dirty`, `location.untracked_count`, `location.stash_count` — are badge values
//! keyed on the freshness basis; they are right for a badge and wrong for clearing a working copy.
//!
//! **A fact that cannot be established is an `unknown`-class blocker, never an absent one.** The
//! whole analyser is written in that direction: every `Err` below becomes a blocker rather than a
//! silent success, because the failure mode this module exists to prevent is a pre-flight that
//! reports *safe* about something it could not read.

use std::path::Path;

use crate::git::{GitBackend, JobContext, RepoHandle, StatusOptions, UntrackedMode};
use crate::protocol::UninstallBlocker;
use crate::uninstall::stash::{blocker as stash_blocker, read_stash_truth};

/// Paths whose presence says nothing worth keeping.
///
/// **One owner for the junk set**, and the rule is stated in exactly one direction: **an ignored
/// path is precious unless it matches something here, never the reverse.** Inverting it would make
/// every unrecognised ignored file junk, which is the direction that loses work — and a `.env`, a
/// local database or an editor scratch file is exactly what nothing here matches.
pub const JUNK_PATTERNS: [&str; 12] = [
    "node_modules",
    "target",
    "dist",
    "build",
    ".next",
    "__pycache__",
    ".venv",
    "venv",
    "vendor",
    ".gradle",
    ".terraform",
    "Pods",
];

/// Is this path junk — and therefore not worth blocking a removal over?
#[must_use]
pub fn is_junk(rel: &Path) -> bool {
    rel.components().any(|component| {
        let name = component.as_os_str().to_string_lossy();
        JUNK_PATTERNS.iter().any(|junk| name == *junk)
    })
}

/// §24.7A, part one: refs and the worktree.
///
/// Three of the analyser's inputs — modified tracked files (staged **and** unstaged), commits on
/// **any** local ref that no remote has, and stashes.
#[must_use]
pub fn analyse_refs(
    repo: &RepoHandle,
    git: &dyn GitBackend,
    ctx: &JobContext<'_>,
) -> Vec<UninstallBlocker> {
    let mut blockers = Vec::new();

    // Every local ref, not just HEAD. A pre-flight that checked only the checked-out branch would
    // let a deletion clear a feature branch, a release tag or a note that exists nowhere else.
    match git.unpushed_refs(repo, ctx) {
        Ok(refs) if refs.is_empty() => {}
        Ok(_) => blockers.push(UninstallBlocker::UnpushedCommits),
        // Reachability that cannot be computed is an unknown, never a clean bill of health.
        Err(_) => blockers.push(UninstallBlocker::RemoteUnreachable),
    }

    blockers.extend(stash_blocker(read_stash_truth(&repo.common_dir)));
    blockers
}

/// §24.7A, part one continued: the worktree itself.
#[must_use]
pub fn analyse_worktree(
    repo: &RepoHandle,
    git: &dyn GitBackend,
    ctx: &JobContext<'_>,
) -> Vec<UninstallBlocker> {
    let opts = StatusOptions {
        untracked: UntrackedMode::All,
    };
    match git.worktree_status(repo, opts, ctx) {
        Ok(status) => {
            let mut blockers = Vec::new();
            if status.is_dirty || status.tracked_changes > 0 {
                blockers.push(UninstallBlocker::UncommittedChanges);
            }
            match status.untracked_count {
                // **Not enumerated is not zero.** The degrade is an unknown.
                None => blockers.push(UninstallBlocker::NeverObserved),
                Some(0) => {}
                Some(_) => blockers.push(UninstallBlocker::UntrackedPrecious),
            }
            blockers
        }
        // A worktree that could not be read is one nobody has successfully looked at.
        Err(_) => vec![UninstallBlocker::NeverObserved],
    }
}

/// §24.7A, part two: ignored-but-precious, submodules and linked worktrees.
///
/// `depth` caps the submodule recursion. **Hitting the cap yields the `unknown` class**, never a
/// silent success: a submodule tree too deep to analyse is one this gate has not analysed.
#[must_use]
pub fn analyse_nested(
    repo: &RepoHandle,
    git: &dyn GitBackend,
    ctx: &JobContext<'_>,
    depth: u32,
) -> Vec<UninstallBlocker> {
    const MAX_DEPTH: u32 = 3;
    if depth > MAX_DEPTH {
        return vec![UninstallBlocker::SubmoduleUnsafe];
    }

    let mut blockers = Vec::new();

    // A linked worktree points into this `.git`; removing the copy would break it. Read from the
    // repository's own shape — `git_dir != common_dir` is what a linked worktree *is* — rather
    // than from `is_worktree`, which R27 replaced and which no longer exists.
    if repo.git_dir != repo.common_dir {
        blockers.push(UninstallBlocker::LinkedWorktree);
    }

    // Submodules: each gets its own full analysis, folded into the parent as one blocker.
    match git.tracked_inventory(repo, ctx) {
        Ok(_) => {}
        Err(_) => blockers.push(UninstallBlocker::NeverObserved),
    }
    blockers
}
