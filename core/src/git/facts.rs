//! The four `rev-parse` answers §4.2 needs to classify a discovery.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use crate::cancel::CancelToken;

use super::error::{GitError, GitResult};
use super::exec::{GitExec, RunLimits};
use super::repo::RepoHandle;

/// What git says about a repository's shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoFacts {
    /// `project.is_bare`.
    pub is_bare: bool,
    /// `project.is_shallow`.
    pub is_shallow: bool,
    /// This checkout's git dir.
    pub git_dir: PathBuf,
    /// The shared git dir; `location.common_dir_bytes`.
    pub common_dir: PathBuf,
}

impl RepoFacts {
    /// `location.is_worktree`: a linked worktree does not own its refs (§4.2).
    #[must_use]
    pub fn is_linked_worktree(&self) -> bool {
        !same_directory(&self.git_dir, &self.common_dir)
    }
}

/// Whether two paths name one directory.
///
/// git answers `--absolute-git-dir` with forward slashes and `--git-common-dir` relatively, so
/// the two spellings of one directory differ textually — on Windows for every ordinary
/// checkout. Comparing the strings therefore reported every repository as a linked worktree,
/// which §4.2 reads as "the same project somewhere else". The comparison canonicalises; the
/// values this struct *stores* stay exactly as git reported them (§1.3).
fn same_directory(left: &Path, right: &Path) -> bool {
    match (std::fs::canonicalize(left), std::fs::canonicalize(right)) {
        (Ok(l), Ok(r)) => l == r,
        // A directory that cannot be canonicalised is gone or unreadable; fall back to the
        // literal comparison rather than claiming the two are the same.
        _ => left == right,
    }
}

/// One invocation; the four answers come back in the order the flags were given.
pub fn repo_facts(
    exec: &GitExec,
    repo: &RepoHandle,
    limits: RunLimits,
    cancel: &CancelToken,
) -> GitResult<RepoFacts> {
    let out = exec.run(
        repo,
        &[
            OsStr::new("rev-parse"),
            OsStr::new("--is-bare-repository"),
            OsStr::new("--is-shallow-repository"),
            OsStr::new("--absolute-git-dir"),
            OsStr::new("--git-common-dir"),
        ],
        limits,
        cancel,
    )?;
    let text = String::from_utf8_lossy(&out.stdout);
    let mut lines = text.lines();
    let unexpected = || GitError::Internal {
        detail: "rev-parse returned fewer lines than flags".to_owned(),
    };
    let is_bare = lines.next().ok_or_else(unexpected)?.trim() == "true";
    let is_shallow = lines.next().ok_or_else(unexpected)?.trim() == "true";
    let git_dir = PathBuf::from(lines.next().ok_or_else(unexpected)?.trim());
    let common_raw = lines.next().ok_or_else(unexpected)?.trim().to_owned();

    // `--git-common-dir` may answer relatively; resolve it against the work dir git ran in.
    let common = PathBuf::from(&common_raw);
    let common_dir = if common.is_absolute() {
        common
    } else {
        repo.work_dir.join(common)
    };
    Ok(RepoFacts {
        is_bare,
        is_shallow,
        git_dir,
        common_dir,
    })
}
