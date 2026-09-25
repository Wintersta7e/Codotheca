//! §45.6 steps 4 and 5: the worktree and nested repositories.
//!
//! **The phase-2 reads, moved unchanged**, so the analyser's order runs end to end before §45.2
//! rows 5–9 replace them. Live, never from cache: `location.is_dirty` and `untracked_count` are
//! badge values keyed on the freshness basis, right for a badge and wrong for clearing a working
//! copy. A fact that cannot be established is an `unknown`-class blocker, never an absent one.

use crate::git::{GitBackend, JobContext, RepoHandle, StatusOptions, UntrackedMode};
use crate::protocol::UninstallBlocker;

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
