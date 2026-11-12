//! The one gate between a WSL-spelled `gitdir:` pointer and explicit selection.
//!
//! A translated handle ([`GitDirSelect::Translated`]) is read with `--git-dir` and `--work-tree`
//! once its ownership is established, and explicit selection skips the ownership check git runs
//! on discovery — the only check between a foreign-owned repository's config (a
//! `filter.<x>.clean`, an `core.fsmonitor`) and a read. So ownership is established here, by
//! probing, in the same resolution that then reads: never from a row, a scan candidate or an
//! earlier scan, because a pointer or an owner can change between the two.
//!
//! **Residual gaps.** The probes do not check the worktree directory's or the `.git` file's
//! owner, which git's discovery path also checks. And the probes and the read are separate git
//! processes, so an owner change between them is not seen; git itself checks in-process.

use std::path::Path;

use crate::cancel::CancelToken;

use super::backend::{GitBackend, JobContext};
use super::error::{GitError, GitResult};
use super::exec::{GitExec, RunLimits};
use super::facts::repo_facts;
use super::repo::{GitDirSelect, RepoHandle};

/// `repo`, selected explicitly if this user owns it.
///
/// Spawns nothing unless `repo` is an untrusted translated handle; a trusted one is already
/// selected explicitly, and any other is selected by discovery, where git checks ownership
/// itself.
///
/// # Errors
///
/// `GitError::Unreadable` when the translated git dir is not there — trusting it would not make
/// it readable; `GitError::Cancelled` when `ctx` is cancelled; `GitError::Untrusted` for any other
/// probe failure (fail closed).
pub fn establish_ownership(
    git: &dyn GitBackend,
    repo: RepoHandle,
    ctx: &JobContext<'_>,
) -> GitResult<RepoHandle> {
    establish(repo, |inside| git.repo_facts(inside, ctx).map(drop))
}

/// [`establish_ownership`] for a caller holding the executor rather than the backend — a launch
/// session's `check-ignore`, which runs outside the job slots.
///
/// # Errors
///
/// As [`establish_ownership`].
pub fn establish_ownership_exec(
    exec: &GitExec,
    repo: RepoHandle,
    limits: RunLimits,
    cancel: &CancelToken,
) -> GitResult<RepoHandle> {
    establish(repo, |inside| {
        repo_facts(exec, inside, limits, cancel).map(drop)
    })
}

/// One discovery-mode `rev-parse` from inside the git dir and one from inside the common dir
/// (whose config a read obeys), each under `base_args`' neutralisation and nothing else, against
/// exactly the directories `repo` then carries.
fn establish(
    repo: RepoHandle,
    mut probe: impl FnMut(&RepoHandle) -> GitResult<()>,
) -> GitResult<RepoHandle> {
    if repo.git_dir_select != GitDirSelect::Translated || repo.trusted {
        return Ok(repo);
    }
    if let Err(e) = std::fs::metadata(&repo.git_dir) {
        if e.kind() == std::io::ErrorKind::NotFound {
            return Err(GitError::Unreadable {
                detail: format!("the `gitdir:` target is not there: {e}"),
            });
        }
    }
    for dir in [&repo.git_dir, &repo.common_dir] {
        let inside = RepoHandle::bare(dir, repo.store.clone(), repo.store_class);
        match probe(&inside) {
            Ok(()) => {}
            Err(GitError::Cancelled) => return Err(GitError::Cancelled),
            Err(_) => return Err(untrusted(dir)),
        }
    }
    Ok(repo.ownership_established())
}

fn untrusted(dir: &Path) -> GitError {
    GitError::Untrusted {
        path: dir.to_string_lossy().into_owned(),
    }
}
