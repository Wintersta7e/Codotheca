//! §45.6 step 2 and §45.5's Lane-0 refusals: the gates that are all local.
//!
//! Every one of them answers in the same direction: **a fact that cannot be established blocks**.
//! The failure this module exists to prevent is a pre-flight that reports *safe* about something
//! it could not read, and every `Err` and every `None` below becomes a blocker rather than a
//! silence. Each refusal here is undischargeable for Uninstall (§45.9), so when one holds the
//! network step does not run.

use std::path::{Path, PathBuf};

use crate::git::RepoHandle;
use crate::protocol::UninstallBlocker;

/// Another location the index holds, as §45.7 lets the analyser read it: its path, and its
/// common dir when a scan recorded one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OtherLocation {
    /// `location.path_bytes`, decoded.
    pub path: PathBuf,
    /// `location.common_dir_bytes`, decoded; `None` when never recorded.
    pub common_dir: Option<PathBuf>,
}

/// §24.7B: a shallow clone is never uninstallable, read **live** (§45.5).
///
/// *Every local ref present upstream* cannot be proved over a truncated graph, and unknown behaves
/// as unsafe. The same exclusion already applies to span, best-year and commit-day maths.
#[must_use]
pub const fn gate_shallow(is_shallow: bool) -> Option<UninstallBlocker> {
    if is_shallow {
        Some(UninstallBlocker::ShallowClone)
    } else {
        None
    }
}

/// §24.7D's refusal list. Each of these is `refused_path`, in the **blocked** class.
///
/// `roots` are the configured scan roots. A path under one is fine; a path that **is** one is not,
/// and neither is one outside every root — the containment check runs in both directions, which is
/// what `core/src/firstrun/roots.rs` gets wrong for root nesting and what this must not repeat.
///
/// [p4] **`.git` must be a directory** (§45.5), read with `symlink_metadata`: a gitfile names a
/// git dir somewhere else, and `exists()` followed a symlinked `.git` to wherever it pointed.
#[must_use]
pub fn gate_path(path: &Path, roots: &[PathBuf]) -> Option<UninstallBlocker> {
    // A symlink is never followed, here or while removing.
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => return Some(UninstallBlocker::RefusedPath),
        Ok(_) => {}
        // A path that cannot be stated cannot be cleared.
        Err(_) => return Some(UninstallBlocker::RefusedPath),
    }

    // The path must hold its own `.git` **directory**. A gitfile is a linked worktree or a
    // separate git dir, and removing the copy would break whatever holds the real one.
    match std::fs::symlink_metadata(path.join(".git")) {
        Ok(meta) if meta.is_dir() => {}
        _ => return Some(UninstallBlocker::RefusedPath),
    }

    // A drive root or a filesystem root has no parent to be contained by.
    if path.parent().is_none() {
        return Some(UninstallBlocker::RefusedPath);
    }

    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from);
    if home.as_deref() == Some(path) {
        return Some(UninstallBlocker::RefusedPath);
    }

    // **Both directions.** Under a root: fine. Equal to a root: refused. Outside every root:
    // refused, because the app was never given consent to touch it.
    let mut contained = false;
    for root in roots {
        if path == root.as_path() {
            return Some(UninstallBlocker::RefusedPath);
        }
        if path.starts_with(root) {
            contained = true;
        }
    }
    if contained {
        None
    } else {
        Some(UninstallBlocker::RefusedPath)
    }
}

/// §45.5: the path **contains** another present location or a scan root. Removing it would
/// remove them too.
#[must_use]
pub fn gate_contains(
    path: &Path,
    others: &[OtherLocation],
    roots: &[PathBuf],
) -> Option<UninstallBlocker> {
    let inside = |other: &Path| other != path && other.starts_with(path);
    (others.iter().any(|o| inside(&o.path)) || roots.iter().any(|r| inside(r)))
        .then_some(UninstallBlocker::RefusedPath)
}

/// §45.5: the git dir or common dir lies outside the path — `--separate-git-dir`, a gitfile
/// target elsewhere — and this is not a linked worktree (which [`gate_linked_worktree`] names).
#[must_use]
pub fn gate_gitdir_outside(repo: &RepoHandle, path: &Path) -> Option<UninstallBlocker> {
    if repo.git_dir != repo.common_dir {
        return None;
    }
    (!repo.git_dir.starts_with(path) || !repo.common_dir.starts_with(path))
        .then_some(UninstallBlocker::RefusedPath)
}

/// §45.5: a linked worktree, **either direction** — this location is one (`git_dir ≠
/// common_dir`), or `common_dir/worktrees/` names any.
///
/// A `worktrees/` directory that exists and cannot be listed is `refs_unreadable`: nothing about
/// what depends on this git dir is known.
#[must_use]
pub fn gate_linked_worktree(repo: &RepoHandle) -> Option<UninstallBlocker> {
    if repo.git_dir != repo.common_dir {
        return Some(UninstallBlocker::LinkedWorktree);
    }
    match std::fs::read_dir(repo.common_dir.join("worktrees")) {
        Ok(mut entries) => entries.next().map(|entry| {
            entry.map_or(UninstallBlocker::RefsUnreadable, |_| {
                UninstallBlocker::LinkedWorktree
            })
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => Some(UninstallBlocker::RefsUnreadable),
    }
}

/// §45.5 (PA2): another present location **borrows** from this tree.
///
/// Its `objects/info/alternates` resolves into this path, or this tree holds its git dir.
/// Removing it would take objects or refs another repository cannot live without.
///
/// An alternates file that exists and cannot be read is `refs_unreadable`: whether it points
/// here is not known.
#[must_use]
pub fn gate_borrowed(path: &Path, others: &[OtherLocation]) -> Option<UninstallBlocker> {
    let mut unreadable = false;
    for other in others {
        // A location this path contains is `refused_path`'s, not a borrower.
        if other.path.starts_with(path) {
            continue;
        }
        let Some(common_dir) = &other.common_dir else {
            continue;
        };
        if common_dir.starts_with(path) {
            return Some(UninstallBlocker::BorrowedByAnotherRepository);
        }
        let objects = common_dir.join("objects");
        match std::fs::read_to_string(objects.join("info").join("alternates")) {
            Ok(text) => {
                for line in text.lines().map(str::trim) {
                    if line.is_empty() || line.starts_with('#') {
                        continue;
                    }
                    // A relative entry is relative to the borrower's own objects directory.
                    let lender = objects.join(line);
                    let lender = lender.canonicalize().unwrap_or(lender);
                    if lender.starts_with(path) {
                        return Some(UninstallBlocker::BorrowedByAnotherRepository);
                    }
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => unreadable = true,
        }
    }
    unreadable.then_some(UninstallBlocker::RefsUnreadable)
}

/// §45.2 row 10: a non-empty `.git/lfs/objects/`. No read here can verify server presence, and
/// a bundle carries pointers only. A directory that cannot be listed is the same unknown.
#[must_use]
pub fn gate_lfs(repo: &RepoHandle) -> Option<UninstallBlocker> {
    match std::fs::read_dir(repo.common_dir.join("lfs").join("objects")) {
        Ok(mut entries) => entries.next().map(|_| UninstallBlocker::LfsUnverified),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => Some(UninstallBlocker::LfsUnverified),
    }
}

/// §24.7D: a repository with a live launch session.
///
/// # Errors
/// Fails when the `session` table cannot be read — which is itself a refusal, never a pass.
pub fn gate_live_session(
    tx: &rusqlite::Transaction<'_>,
    location: crate::protocol::LocationId,
) -> rusqlite::Result<Option<UninstallBlocker>> {
    let live: i64 = tx.query_row(
        "SELECT COUNT(*) FROM session WHERE location_id = ?1 AND ended_at IS NULL",
        [location.0],
        |row| row.get(0),
    )?;
    Ok((live > 0).then_some(UninstallBlocker::LiveSession))
}

/// §24.7G's first-day lock: **you cannot uninstall what the app has never successfully looked at.**
///
/// *Never observed* and *stale but once known* are distinct and neither may be rendered as the
/// other: this gate fires only on the first, which is why it takes both observation clocks rather
/// than an error flag.
#[must_use]
pub const fn gate_first_day(
    refstate_observed_at: Option<i64>,
    worktree_observed_at: Option<i64>,
) -> Option<UninstallBlocker> {
    if refstate_observed_at.is_none() || worktree_observed_at.is_none() {
        Some(UninstallBlocker::NeverObserved)
    } else {
        None
    }
}
