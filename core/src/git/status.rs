//! §3.3's second row: `status --porcelain=v2 -z --branch`, and §4.1's J2 degrade.
//!
//! Worktree state is the one class §6 says is never cacheable, so everything here is a
//! timestamped observation. Absence of dirty means "no changes as of `observed_at`", never
//! "clean".

use std::ffi::OsStr;

use crate::cancel::CancelToken;
use crate::clock::Clock;

use super::error::{GitError, GitResult};
use super::exec::{GitExec, RunLimits};
use super::repo::RepoHandle;

/// Whether untracked files are enumerated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UntrackedMode {
    /// `--untracked-files=all`: the full count.
    All,
    /// `--untracked-files=no`: §4.1's degrade — tracked only, count unknown.
    None,
}

/// How one status observation is taken.
#[derive(Debug, Clone, Copy)]
pub struct StatusOptions {
    /// Untracked enumeration mode.
    pub untracked: UntrackedMode,
}

impl StatusOptions {
    /// The ordinary observation.
    #[must_use]
    pub const fn full() -> Self {
        Self {
            untracked: UntrackedMode::All,
        }
    }

    /// The J2 degrade after the 500 ms budget is exceeded (§4.1).
    #[must_use]
    pub const fn degraded() -> Self {
        Self {
            untracked: UntrackedMode::None,
        }
    }
}

/// The raw counts a porcelain-v2 stream carries.
#[derive(Debug, Clone, Default)]
pub struct StatusCounts {
    /// Entries of type `1`, `2` or `u`.
    pub tracked_changes: u32,
    /// Entries of type `?`.
    pub untracked: u32,
    /// `# branch.head`, or `None` when detached.
    pub branch: Option<String>,
    /// `# branch.oid`, or `None` on an unborn HEAD.
    pub head_oid: Option<String>,
    /// The `+N` half of `# branch.ab`.
    pub ahead: Option<u32>,
    /// The `-N` half of `# branch.ab`.
    pub behind: Option<u32>,
}

/// One observation of the working tree.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WorktreeStatus {
    /// `location.is_dirty`: any tracked modification.
    pub is_dirty: bool,
    /// How many tracked entries changed.
    pub tracked_changes: u32,
    /// `location.untracked_count`. `None` means **not enumerated** (the degrade), never zero.
    pub untracked_count: Option<u32>,
    /// The branch as status saw it.
    pub branch: Option<String>,
    /// The tip as status saw it.
    pub head_oid: Option<String>,
    /// Ahead, from the branch header, when an upstream is configured.
    pub ahead: Option<u32>,
    /// Behind, from the branch header, when an upstream is configured.
    pub behind: Option<u32>,
    /// `location.worktree_observed_at`.
    pub observed_at: i64,
}

/// Parse a `--porcelain=v2 -z --branch` stream.
///
/// Records are NUL-separated. A type `2` (rename/copy) record is followed by a **second**
/// NUL-terminated field holding the original path, which must be consumed with it.
///
/// # Errors
///
/// `GitError::Internal` when a record starts with a type byte porcelain v2 does not define.
pub fn parse_status_v2(bytes: &[u8]) -> GitResult<StatusCounts> {
    let mut counts = StatusCounts::default();
    let mut fields = bytes.split(|b| *b == 0).filter(|f| !f.is_empty());
    while let Some(field) = fields.next() {
        let kind = field.first().copied().unwrap_or(b' ');
        match kind {
            b'#' => {
                let text = String::from_utf8_lossy(field);
                let line = text.trim_start_matches("# ").trim();
                if let Some(v) = line.strip_prefix("branch.oid ") {
                    let v = v.trim();
                    counts.head_oid = (v != "(initial)").then(|| v.to_owned());
                } else if let Some(v) = line.strip_prefix("branch.head ") {
                    let v = v.trim();
                    counts.branch = (v != "(detached)").then(|| v.to_owned());
                } else if let Some(v) = line.strip_prefix("branch.ab ") {
                    for part in v.split_whitespace() {
                        if let Some(n) = part.strip_prefix('+') {
                            counts.ahead = n.parse::<u32>().ok();
                        } else if let Some(n) = part.strip_prefix('-') {
                            counts.behind = n.parse::<u32>().ok();
                        }
                    }
                }
            }
            b'1' | b'u' => counts.tracked_changes = counts.tracked_changes.saturating_add(1),
            b'2' => {
                counts.tracked_changes = counts.tracked_changes.saturating_add(1);
                let _original_path = fields.next(); // the second field of a rename record
            }
            b'?' => counts.untracked = counts.untracked.saturating_add(1),
            b'!' => {}
            other => {
                return Err(GitError::Internal {
                    detail: format!("unknown porcelain v2 record type {}", char::from(other)),
                })
            }
        }
    }
    Ok(counts)
}

/// Observe the working tree.
///
/// # Errors
///
/// The `status` invocation's failure as [`GitExec::run_piped`] classifies it, or
/// [`parse_status_v2`]'s.
pub fn worktree_status(
    exec: &GitExec,
    repo: &RepoHandle,
    opts: StatusOptions,
    limits: RunLimits,
    cancel: &CancelToken,
    clock: &dyn Clock,
) -> GitResult<WorktreeStatus> {
    let untracked = match opts.untracked {
        UntrackedMode::All => "--untracked-files=all",
        UntrackedMode::None => "--untracked-files=no",
    };
    let out = exec.run(
        repo,
        &[
            OsStr::new("status"),
            OsStr::new("--porcelain=v2"),
            OsStr::new("-z"),
            OsStr::new("--branch"),
            OsStr::new(untracked),
        ],
        limits,
        cancel,
    )?;
    let counts = parse_status_v2(&out.stdout)?;
    Ok(WorktreeStatus {
        is_dirty: counts.tracked_changes > 0,
        tracked_changes: counts.tracked_changes,
        untracked_count: match opts.untracked {
            UntrackedMode::All => Some(counts.untracked),
            UntrackedMode::None => None,
        },
        branch: counts.branch,
        head_oid: counts.head_oid,
        ahead: counts.ahead,
        behind: counts.behind,
        observed_at: clock.now_unix(),
    })
}
