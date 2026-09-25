//! §45.6 step 5: **every** nested repository — §45.2 row 9's three kinds, each analysed in full.
//!
//! - `submodule`: a gitlink in the index whose directory is checked out;
//! - `module_gitdir`: every git dir under `.git/modules/` — a de-initialised submodule keeps its
//!   refs there — less those a checked-out submodule already names;
//! - `independent`: any directory holding `.git` in the tree, junk directories included.
//!
//! Each is analysed through steps 2–9 with **resolve-only identity** (it has no row) and **its own
//! remotes**, to depth ≤ [`MAX_NESTING`]; one deeper is `nesting_too_deep`.

use std::path::{Path, PathBuf};

use crate::analyser::worktree::holds_git;
use crate::git::RepoHandle;
use crate::protocol::NestedKind;

/// The deepest nested repository the analyser reads; one deeper is `nesting_too_deep` (§45.6).
pub const MAX_NESTING: u32 = 3;

/// One nested repository to analyse.
#[derive(Debug, Clone)]
pub struct Candidate {
    /// Which of row 9's kinds.
    pub kind: NestedKind,
    /// Relative to the repository that holds it, for `pathDisplay`.
    pub rel: PathBuf,
    /// The repository, resolved; `None` when it could not be.
    pub repo: Option<RepoHandle>,
}

/// Row 9's candidates in `parent`: its checked-out `gitlinks`, its `.git/modules/` git dirs that
/// no checked-out submodule names, and its `in_tree` directories holding `.git`.
#[must_use]
pub fn candidates(
    parent: &RepoHandle,
    gitlinks: &[PathBuf],
    in_tree: &[PathBuf],
) -> Vec<Candidate> {
    let resolve = |dir: &Path| {
        RepoHandle::resolve(dir, parent.store.clone(), parent.store_class)
            .ok()
            .map(|repo| repo.with_trust(parent.trusted))
    };
    let mut out = Vec::new();
    for rel in gitlinks {
        let dir = parent.work_dir.join(rel);
        // A gitlink that is not checked out holds no bytes of its own in the tree; its git dir,
        // if any, is found below.
        if holds_git(&dir) {
            out.push(Candidate {
                kind: NestedKind::Submodule,
                rel: rel.clone(),
                repo: resolve(&dir),
            });
        }
    }
    let named: Vec<PathBuf> = out
        .iter()
        .filter_map(|c| c.repo.as_ref())
        .map(|repo| canonical(&repo.git_dir))
        .collect();
    let mut gitdirs = Vec::new();
    module_gitdirs(&parent.common_dir.join("modules"), &mut gitdirs);
    for gitdir in gitdirs {
        if named.contains(&canonical(&gitdir)) {
            continue;
        }
        let rel = gitdir
            .strip_prefix(&parent.common_dir)
            .map_or_else(|_| gitdir.clone(), |rest| Path::new(".git").join(rest));
        out.push(Candidate {
            kind: NestedKind::ModuleGitdir,
            rel,
            repo: Some(
                RepoHandle::bare(&gitdir, parent.store.clone(), parent.store_class)
                    .with_trust(parent.trusted),
            ),
        });
    }
    for rel in in_tree {
        out.push(Candidate {
            kind: NestedKind::Independent,
            rel: rel.clone(),
            repo: resolve(&parent.work_dir.join(rel)),
        });
    }
    out
}

fn canonical(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

/// Every git dir under `dir`, not descending into one it found: a git dir's own `modules/` is its
/// nested repositories, read when it is analysed.
fn module_gitdirs(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(children) = std::fs::read_dir(dir) else {
        return;
    };
    for child in children.flatten() {
        let Ok(kind) = child.file_type() else {
            continue;
        };
        if !kind.is_dir() || kind.is_symlink() {
            continue;
        }
        let path = child.path();
        if path.join("HEAD").is_file() && path.join("objects").is_dir() {
            out.push(path);
        } else {
            module_gitdirs(&path, out);
        }
    }
}
