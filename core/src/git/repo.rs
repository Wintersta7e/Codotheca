//! Where a repository's three directories are, and how it is scheduled.

use std::path::{Path, PathBuf};

// R4: the store classification is plan 06's, and it is the only one.
use crate::mount::StoreClass;

use super::error::{GitError, GitResult};

/// The runtime scheduler key: the actual mounted device or share (§4.7). Supplied by the
/// `MountResolver` seam; this module only groups by it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StoreKey(String);

impl StoreKey {
    /// Wrap a resolver-produced key.
    #[must_use]
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    /// The key as text, which is what `location.store_key` stores.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

// [superseded by R4] `pub enum StoreClass { Fast, Slow }` was declared here. How many
// concurrent git processes one store tolerates is `core::mount::StoreClass::per_store_cap()`
// (plan 06) — `Local`/`Unknown` cap 4, `Network`/`Hdd`/`Fuse`/`Removable` cap 1. Two spellings
// of §3.4's rule is one place for it to drift; this module now only groups by the class.

/// One repository, resolved.
#[derive(Debug, Clone)]
pub struct RepoHandle {
    /// The directory git runs in (`-C`). For a bare repository this is the git dir.
    pub work_dir: PathBuf,
    /// This checkout's git dir. A linked worktree has its own.
    pub git_dir: PathBuf,
    /// The shared git dir: refs, packed-refs, config, objects.
    pub common_dir: PathBuf,
    /// Scheduling key.
    pub store: StoreKey,
    /// Scheduling class.
    pub store_class: StoreClass,
    /// True when `location.trusted_at` is non-NULL, which adds `-c safe.directory=<path>`.
    pub trusted: bool,
}

impl RepoHandle {
    /// Resolve a working tree. Reads `.git` (directory or `gitdir:` file) and `commondir`;
    /// spawns nothing.
    pub fn resolve(work_dir: &Path, store: StoreKey, store_class: StoreClass) -> GitResult<Self> {
        let dot_git = work_dir.join(".git");
        let meta = std::fs::metadata(&dot_git).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => GitError::PathGone {
                detail: e.to_string(),
            },
            std::io::ErrorKind::PermissionDenied => GitError::PermissionDenied {
                detail: e.to_string(),
            },
            _ => GitError::Stale {
                detail: e.to_string(),
            },
        })?;

        let git_dir = if meta.is_dir() {
            dot_git
        } else {
            let text = std::fs::read_to_string(&dot_git).map_err(|e| GitError::Stale {
                detail: e.to_string(),
            })?;
            let target = text
                .lines()
                .find_map(|l| l.trim().strip_prefix("gitdir:"))
                .ok_or_else(|| GitError::Unreadable {
                    detail: "`.git` file carries no gitdir:".to_owned(),
                })?
                .trim();
            let p = PathBuf::from(target);
            if p.is_absolute() {
                p
            } else {
                work_dir.join(p)
            }
        };

        let common_dir = match std::fs::read_to_string(git_dir.join("commondir")) {
            Ok(text) => {
                let rel = text.trim();
                let p = PathBuf::from(rel);
                if p.is_absolute() {
                    p
                } else {
                    git_dir.join(p)
                }
            }
            Err(_) => git_dir.clone(),
        };

        Ok(Self {
            work_dir: work_dir.to_path_buf(),
            git_dir,
            common_dir,
            store,
            store_class,
            trusted: false,
        })
    }

    /// A bare repository: `git_dir` is also the working directory (§4.2).
    #[must_use]
    pub fn bare(git_dir: &Path, store: StoreKey, store_class: StoreClass) -> Self {
        Self {
            work_dir: git_dir.to_path_buf(),
            git_dir: git_dir.to_path_buf(),
            common_dir: git_dir.to_path_buf(),
            store,
            store_class,
            trusted: false,
        }
    }

    /// Set the trust flag from `location.trusted_at`.
    #[must_use]
    pub fn with_trust(mut self, trusted: bool) -> Self {
        self.trusted = trusted;
        self
    }

    /// True when this checkout does not own its refs — §1.3's `is_worktree`.
    #[must_use]
    pub fn is_linked_worktree(&self) -> bool {
        self.git_dir != self.common_dir
    }
}
