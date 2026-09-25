//! §45.6 step 4: the worktree — §45.2 rows 5–8 and 11, read live through
//! [`GitBackend::worktree_scan`] and classified by §45.4's junk rule.
//!
//! **Live, never from cache.** `location.is_dirty` and `untracked_count` are badge values keyed on
//! the freshness basis, right for a badge and wrong for clearing a working copy. A worktree that
//! cannot be read is `never_observed` (the shipped mapping), and **`precious: None` then means not
//! enumerated, never none**.
//!
//! Precious directories are walked to size them and junk directories for a nested `.git` only;
//! **no symlink is followed**. A directory holding `.git` is never precious here and never junk:
//! it is another repository, and step 5 analyses it.

use std::path::{Path, PathBuf};

use crate::analyser::junk::is_junk;
use crate::git::{GitBackend, JobContext, RepoHandle, StatusEntry};
use crate::protocol::{PreciousEntry, PreciousSummary, UninstallBlocker};

/// How many precious entries the verdict lists. The renderer reads `truncated` and `totalCount`
/// and never restates this (§45.12).
pub const PRECIOUS_DISPLAY_CAP: usize = 20;

/// A gitlink's mode in the index: a submodule's commit, not a file.
const GITLINK_MODE: &str = "160000";

/// What step 4 found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorktreeFindings {
    /// Rows 5–8 and 11's blockers.
    pub blockers: Vec<UninstallBlocker>,
    /// Row 8 and 11's precious paths; `None` when the worktree could not be enumerated.
    pub precious: Option<PreciousSummary>,
    /// Row 9's gitlinks, from the index: submodule paths, relative to the root.
    pub gitlinks: Vec<PathBuf>,
    /// Row 9's in-tree directories holding `.git`, relative to the root — untracked, ignored or
    /// inside junk.
    pub in_tree: Vec<PathBuf>,
}

/// Does `dir` hold a `.git`, directory or gitfile? Read without following a symlink.
pub(crate) fn holds_git(dir: &Path) -> bool {
    std::fs::symlink_metadata(dir.join(".git")).is_ok()
}

/// Step 4 over `repo`'s worktree.
#[must_use]
pub fn analyse_worktree(
    repo: &RepoHandle,
    git: &dyn GitBackend,
    ctx: &JobContext<'_>,
) -> WorktreeFindings {
    let Ok(scan) = git.worktree_scan(repo, ctx) else {
        return WorktreeFindings {
            blockers: vec![UninstallBlocker::NeverObserved],
            ..WorktreeFindings::default()
        };
    };
    let root = &repo.work_dir;
    let mut found = WorktreeFindings::default();
    let mut entries: Vec<PreciousEntry> = Vec::new();

    for entry in &scan.entries {
        match entry {
            // Rows 5 and 7: staged, unstaged, both, or conflicted.
            StatusEntry::Changed { .. } | StatusEntry::Unmerged { .. } => {
                found.blockers.push(UninstallBlocker::UncommittedChanges);
            }
            // Row 8.
            StatusEntry::Untracked { path } | StatusEntry::Ignored { path } => {
                let is_dir = path.last() == Some(&b'/');
                let bytes = path.strip_suffix(b"/").unwrap_or(path);
                let rel = crate::paths::path_from_bytes(bytes);
                let abs = root.join(&rel);
                if holds_git(&abs) {
                    found.in_tree.push(rel);
                    continue;
                }
                if is_junk(root, &rel, is_dir) {
                    if is_dir && !find_nested(&abs, &rel, &mut found.in_tree) {
                        found.blockers.push(UninstallBlocker::NeverObserved);
                    }
                    continue;
                }
                found
                    .blockers
                    .push(if matches!(entry, StatusEntry::Ignored { .. }) {
                        UninstallBlocker::IgnoredPrecious
                    } else {
                        UninstallBlocker::UntrackedPrecious
                    });
                entries.push(PreciousEntry {
                    path_display: String::from_utf8_lossy(path).into_owned(),
                    bytes: size_of(&abs, &rel, &mut found.in_tree),
                });
            }
        }
    }

    // Row 6: a hidden file present on disk — `status` cannot see its edits.
    if scan.hidden.iter().any(|path| {
        std::fs::symlink_metadata(root.join(crate::paths::path_from_bytes(path))).is_ok()
    }) {
        found.blockers.push(UninstallBlocker::HiddenFromStatus);
    }

    // Row 11: the repository's own hooks directory, every file but a `*.sample`.
    match std::fs::read_dir(repo.common_dir.join("hooks")) {
        Ok(hooks) => {
            for hook in hooks.flatten() {
                let name = hook.file_name().to_string_lossy().into_owned();
                let Ok(meta) = std::fs::symlink_metadata(hook.path()) else {
                    found.blockers.push(UninstallBlocker::NeverObserved);
                    continue;
                };
                if meta.is_dir() || name.ends_with(".sample") {
                    continue;
                }
                found.blockers.push(UninstallBlocker::UntrackedPrecious);
                entries.push(PreciousEntry {
                    path_display: format!(".git/hooks/{name}"),
                    bytes: i64::try_from(meta.len()).ok(),
                });
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => found.blockers.push(UninstallBlocker::NeverObserved),
    }

    found.gitlinks = scan
        .index
        .iter()
        .filter(|entry| entry.mode == GITLINK_MODE && entry.stage == 0)
        .map(|entry| crate::paths::path_from_bytes(&entry.path))
        .collect();
    found.precious = Some(summarise(entries));
    found
}

/// §45.12's summary: sorted by bytes descending then path, capped, the total null when any size
/// is unknown — never a partial sum.
fn summarise(mut entries: Vec<PreciousEntry>) -> PreciousSummary {
    entries.sort_by(|a, b| {
        b.bytes
            .cmp(&a.bytes)
            .then_with(|| a.path_display.cmp(&b.path_display))
    });
    let total_bytes = entries.iter().try_fold(0_i64, |sum, entry| {
        entry.bytes.map(|b| sum.saturating_add(b))
    });
    let total_count = i64::try_from(entries.len()).unwrap_or(i64::MAX);
    let truncated = entries.len() > PRECIOUS_DISPLAY_CAP;
    entries.truncate(PRECIOUS_DISPLAY_CAP);
    PreciousSummary {
        entries,
        total_count,
        total_bytes,
        truncated,
    }
}

/// The bytes under `abs`, never following a symlink; a directory holding `.git` inside it is
/// recorded in `nested` and not counted. `None` when any part could not be read.
fn size_of(abs: &Path, rel: &Path, nested: &mut Vec<PathBuf>) -> Option<i64> {
    let meta = std::fs::symlink_metadata(abs).ok()?;
    if !meta.is_dir() {
        return i64::try_from(meta.len()).ok();
    }
    let mut total = 0_i64;
    let mut known = true;
    for child in std::fs::read_dir(abs).ok()? {
        let Ok(child) = child else {
            known = false;
            continue;
        };
        let child_abs = child.path();
        let child_rel = rel.join(child.file_name());
        if child
            .file_type()
            .is_ok_and(|kind| kind.is_dir() && !kind.is_symlink())
            && holds_git(&child_abs)
        {
            nested.push(child_rel);
            continue;
        }
        match size_of(&child_abs, &child_rel, nested) {
            Some(bytes) => total = total.saturating_add(bytes),
            None => known = false,
        }
    }
    known.then_some(total)
}

/// Walk a junk directory for a nested `.git` **only** — never to size it. False when some part of
/// it could not be read, so whether it hides a repository is not known.
pub(crate) fn find_nested(abs: &Path, rel: &Path, nested: &mut Vec<PathBuf>) -> bool {
    let Ok(children) = std::fs::read_dir(abs) else {
        return false;
    };
    let mut complete = true;
    for child in children {
        let Ok(child) = child else {
            complete = false;
            continue;
        };
        let Ok(kind) = child.file_type() else {
            complete = false;
            continue;
        };
        if !kind.is_dir() || kind.is_symlink() {
            continue;
        }
        let child_abs = child.path();
        let child_rel = rel.join(child.file_name());
        if holds_git(&child_abs) {
            nested.push(child_rel);
        } else if !find_nested(&child_abs, &child_rel, nested) {
            complete = false;
        }
    }
    complete
}
