//! §45.8: **the snapshot** — the content an act was decided over, bound so that step 9 can see
//! it change.
//!
//! One entry per repository — the location, and each nested one keyed by its relative path:
//! `HEAD`, every row-1 ref as `(name, OID)` (an annotated tag's OID is its tag object's, so row 4
//! is bound here), every stash entry in reflog order with the files-backend readability fact, the
//! index list, and `(path, type, mode, size, SHA-256)` for every path of rows 6–8, 10 and 11 — a
//! symlink's target bytes recorded, never followed. **Junk is not in it; a repository nested
//! under junk is.**
//!
//! The digest is SHA-256 over a canonical serialisation sorted by name and path bytes, with
//! [`SNAPSHOT_FORMAT_VERSION`] inside the input, so two versions never compare equal — they
//! refuse, the safe direction. **In-core only: it never crosses the protocol** (§24.8).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use sha2::{Digest as _, Sha256};

use crate::analyser::junk::is_junk;
use crate::analyser::nested::{self, MAX_NESTING};
use crate::analyser::worktree::{find_nested, holds_git};
use crate::git::{GitBackend, HeadState, JobContext, RepoHandle, StashEntries, StatusEntry};
use crate::protocol::UninstallBlocker;

/// The serialisation's version. Part of every digest's input.
pub const SNAPSHOT_FORMAT_VERSION: u32 = 1;

/// One path of rows 6–8, 10 or 11.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct PathRecord {
    /// Raw path bytes, relative to the repository root (the git dir for `.git/…`).
    pub path: Vec<u8>,
    /// `f` file, `l` symlink, `-` absent.
    pub kind: char,
    /// The permission bits as the platform reports them.
    pub mode: u32,
    /// Its size in bytes.
    pub size: u64,
    /// SHA-256 of the content, or of a symlink's target bytes.
    pub sha256: [u8; 32],
}

/// One repository's part of the snapshot.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RepoSnapshot {
    /// `ref:<name>`, `oid:<oid>` or `unborn`.
    pub head: String,
    /// Every row-1 ref, `(name, OID)`, sorted by name.
    pub refs: Vec<(String, String)>,
    /// Every stash entry's commit, newest first.
    pub stash: Vec<String>,
    /// Whether the files-backend stash reflog could be read.
    pub stash_readable: bool,
    /// The index: `(mode, OID, stage, path)`.
    pub index: Vec<(String, String, u8, Vec<u8>)>,
    /// Rows 6–8, 10 and 11, sorted by path.
    pub paths: Vec<PathRecord>,
}

/// §45.8's snapshot of a location and every repository nested in it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Snapshot {
    /// Keyed by path relative to the location; the location itself is the empty key.
    pub repos: BTreeMap<String, RepoSnapshot>,
}

impl Snapshot {
    /// The digest at [`SNAPSHOT_FORMAT_VERSION`].
    #[must_use]
    pub fn digest(&self) -> String {
        self.digest_at(SNAPSHOT_FORMAT_VERSION)
    }

    /// The digest as serialisation `version` would compute it — two versions never compare
    /// equal.
    #[must_use]
    pub fn digest_at(&self, version: u32) -> String {
        use std::fmt::Write as _;

        let mut hasher = Sha256::new();
        let mut field = |bytes: &[u8]| {
            hasher.update(u64::try_from(bytes.len()).unwrap_or(u64::MAX).to_le_bytes());
            hasher.update(bytes);
        };
        field(&version.to_le_bytes());
        for (key, repo) in &self.repos {
            field(b"repository");
            field(key.as_bytes());
            field(repo.head.as_bytes());
            for (name, oid) in &repo.refs {
                field(name.as_bytes());
                field(oid.as_bytes());
            }
            field(b"stash");
            field(&[u8::from(repo.stash_readable)]);
            for oid in &repo.stash {
                field(oid.as_bytes());
            }
            field(b"index");
            for (mode, oid, stage, path) in &repo.index {
                field(mode.as_bytes());
                field(oid.as_bytes());
                field(&[*stage]);
                field(path);
            }
            field(b"paths");
            for record in &repo.paths {
                field(&record.path);
                field(record.kind.to_string().as_bytes());
                field(&record.mode.to_le_bytes());
                field(&record.size.to_le_bytes());
                field(&record.sha256);
            }
        }
        let digest = hasher.finalize();
        let mut hex = String::with_capacity(digest.len() * 2);
        for byte in digest {
            let _ = write!(hex, "{byte:02x}");
        }
        hex
    }
}

/// §45.8's snapshot of `repo` and every repository nested in it, read live.
///
/// # Errors
/// The unknown-class blocker of the read that failed: `refs_unreadable`, `stash_unreadable` or
/// `never_observed`.
pub fn snapshot(
    repo: &RepoHandle,
    git: &dyn GitBackend,
    ctx: &JobContext<'_>,
) -> Result<Snapshot, UninstallBlocker> {
    let mut out = Snapshot::default();
    take(repo, "", git, ctx, 0, &mut out)?;
    Ok(out)
}

fn take(
    repo: &RepoHandle,
    key: &str,
    git: &dyn GitBackend,
    ctx: &JobContext<'_>,
    depth: u32,
    out: &mut Snapshot,
) -> Result<(), UninstallBlocker> {
    let listing = git
        .enumerate_refs(repo, ctx)
        .map_err(|_| UninstallBlocker::RefsUnreadable)?;
    let mut part = RepoSnapshot {
        head: match listing.head {
            HeadState::Symbolic(name) => format!("ref:{name}"),
            HeadState::Detached(oid) => format!("oid:{oid}"),
            HeadState::Unborn => "unborn".to_owned(),
        },
        refs: listing.refs.into_iter().map(|r| (r.name, r.oid)).collect(),
        ..RepoSnapshot::default()
    };
    part.refs.sort();
    match git.stash_entries(repo, ctx) {
        Ok(StashEntries::Entries(entries)) => {
            part.stash = entries;
            part.stash_readable = true;
        }
        Ok(StashEntries::Unreadable) => part.stash_readable = false,
        Err(_) => return Err(UninstallBlocker::StashUnreadable),
    }

    let mut gitlinks = Vec::new();
    let mut in_tree = Vec::new();
    if repo.work_dir != repo.git_dir {
        let scan = git
            .worktree_scan(repo, ctx)
            .map_err(|_| UninstallBlocker::NeverObserved)?;
        let root = &repo.work_dir;
        for entry in &scan.index {
            part.index.push((
                entry.mode.clone(),
                entry.oid.clone(),
                entry.stage,
                entry.path.clone(),
            ));
            if entry.mode == "160000" && entry.stage == 0 {
                gitlinks.push(crate::paths::path_from_bytes(&entry.path));
            }
        }
        for entry in &scan.entries {
            match entry {
                StatusEntry::Changed { path, .. } | StatusEntry::Unmerged { path } => {
                    record(root, path, &mut part.paths);
                }
                StatusEntry::Untracked { path } | StatusEntry::Ignored { path } => {
                    let is_dir = path.last() == Some(&b'/');
                    let bytes = path.strip_suffix(b"/").unwrap_or(path);
                    let rel = crate::paths::path_from_bytes(bytes);
                    let abs = root.join(&rel);
                    if holds_git(&abs) {
                        in_tree.push(rel);
                    } else if is_junk(root, &rel, is_dir) {
                        if is_dir && !find_nested(&abs, &rel, &mut in_tree) {
                            return Err(UninstallBlocker::NeverObserved);
                        }
                    } else {
                        record_tree(root, &rel, &mut part.paths, &mut in_tree)?;
                    }
                }
            }
        }
        for hidden in &scan.hidden {
            record(root, hidden, &mut part.paths);
        }
    }
    // Rows 10 and 11, under the git dir.
    for dir in [Path::new("hooks"), Path::new("lfs/objects")] {
        let base = repo.common_dir.join(dir);
        if base.exists() {
            let mut files = Vec::new();
            record_tree(&repo.common_dir, dir, &mut files, &mut Vec::new())?;
            part.paths.extend(
                files
                    .into_iter()
                    .filter(|r| !(dir == Path::new("hooks") && r.path.ends_with(b".sample"))),
            );
        }
    }
    part.index.sort_by(|a, b| a.3.cmp(&b.3).then(a.2.cmp(&b.2)));
    part.paths.sort();
    out.repos.insert(key.to_owned(), part);

    if depth >= MAX_NESTING {
        return Ok(());
    }
    for candidate in nested::candidates(repo, &gitlinks, &in_tree) {
        let rel = candidate.rel.to_string_lossy().replace('\\', "/");
        let child_key = if key.is_empty() {
            rel
        } else {
            format!("{key}/{rel}")
        };
        let child = candidate.repo.ok_or(UninstallBlocker::NeverObserved)?;
        take(&child, &child_key, git, ctx, depth + 1, out)?;
    }
    Ok(())
}

/// One path's record: a file's content hash, a symlink's target hash, or its absence.
fn record(root: &Path, path: &[u8], out: &mut Vec<PathRecord>) {
    let abs = root.join(crate::paths::path_from_bytes(path));
    let Ok(meta) = std::fs::symlink_metadata(&abs) else {
        out.push(PathRecord {
            path: path.to_vec(),
            kind: '-',
            mode: 0,
            size: 0,
            sha256: [0; 32],
        });
        return;
    };
    let (kind, content) = if meta.file_type().is_symlink() {
        (
            'l',
            std::fs::read_link(&abs)
                .map(|target| crate::paths::path_bytes(&target))
                .unwrap_or_default(),
        )
    } else {
        ('f', std::fs::read(&abs).unwrap_or_default())
    };
    out.push(PathRecord {
        path: path.to_vec(),
        kind,
        mode: mode_of(&meta),
        size: meta.len(),
        sha256: Sha256::digest(&content).into(),
    });
}

/// Every file under `rel` (itself when it is one), never following a symlink; a directory holding
/// `.git` is recorded in `nested` and not descended.
fn record_tree(
    root: &Path,
    rel: &Path,
    out: &mut Vec<PathRecord>,
    nested: &mut Vec<PathBuf>,
) -> Result<(), UninstallBlocker> {
    let abs = root.join(rel);
    let Ok(meta) = std::fs::symlink_metadata(&abs) else {
        record(root, &crate::paths::path_bytes(rel), out);
        return Ok(());
    };
    if !meta.is_dir() {
        record(root, &crate::paths::path_bytes(rel), out);
        return Ok(());
    }
    let children = std::fs::read_dir(&abs).map_err(|_| UninstallBlocker::NeverObserved)?;
    for child in children {
        let child = child.map_err(|_| UninstallBlocker::NeverObserved)?;
        let child_rel = rel.join(child.file_name());
        let child_abs = child.path();
        let kind = child
            .file_type()
            .map_err(|_| UninstallBlocker::NeverObserved)?;
        if kind.is_dir() && !kind.is_symlink() && holds_git(&child_abs) {
            nested.push(child_rel);
            continue;
        }
        record_tree(root, &child_rel, out, nested)?;
    }
    Ok(())
}

/// The permission bits the platform reports.
#[cfg(unix)]
fn mode_of(meta: &std::fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt as _;
    meta.permissions().mode()
}

/// The permission bits the platform reports: on Windows, the read-only flag.
#[cfg(not(unix))]
fn mode_of(meta: &std::fs::Metadata) -> u32 {
    u32::from(meta.permissions().readonly())
}
