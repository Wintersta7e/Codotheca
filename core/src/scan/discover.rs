//! §4.2 — what a directory is.
//!
//! Four answers, and the one `rev-parse` probe that separates the ambiguous ones. Nothing here
//! decides identity: which `project` row a candidate belongs to is plan 08's question, and a
//! linked worktree is flagged as such precisely so plan 08 can attach it definitively (§1.1).
//!
//! **Deviation from plan 07 Task 5.** The plan probes through
//! `GitBackend::run(GitRequest { args: ["rev-parse", "--absolute-git-dir", "--git-common-dir"] })`.
//! Plan 05 shipped `GitBackend` as one method per operation with no argv at the boundary, so
//! that call does not exist; `repo_facts` asks §4.2's question in one invocation, under the §3.4
//! slot caps, and its `is_linked_worktree` **canonicalises** before comparing — which matters,
//! because git answers `--absolute-git-dir` with forward slashes and `--git-common-dir`
//! relatively, so a textual comparison reports every ordinary Windows checkout as a linked
//! worktree.

use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use crate::cancel::CancelToken;
use crate::git::{GitBackend, JobClass, JobContext, RepoFacts, RepoHandle, StoreKey};
use crate::mount::StoreClass;
use crate::paths::path_display;
use crate::scan::{ScanProblem, ScanProblemKind, WalkEvent, WalkOptions, WalkSink};

/// A `.git` file is a one-line pointer. Anything larger is not one.
const DOT_GIT_FILE_CAP: u64 = 4 * 1024;
/// This probe is a handful of file reads behind a process spawn; it is not a job budget (§4.1).
pub const GIT_PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// How a discovered directory holds its repository — the value `location.repo_kind` stores.
///
/// **R7: serde and an inverse are both required, not conveniences.** `location.repo_kind` is a
/// TEXT column, so every value `as_str` writes has to be readable back; and plan 18's in-distro
/// worker returns a classified candidate across the protocol, so the enum has to serialise.
///
/// **The renames are per variant and not `rename_all = "snake_case"`, which is what the plan
/// specifies and is wrong.** `snake_case` spells `WorkTree` as `work_tree`, while `as_str` and
/// the `location.repo_kind` CHECK constraint both say `worktree` — one value with two spellings,
/// the wire form being the one the DDL rejects. R26's shape exactly, and caught by the round-trip
/// test rather than by review.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum RepoKind {
    /// A `.git` directory at `path`.
    #[serde(rename = "worktree")]
    WorkTree,
    /// A `.git` file whose git dir differs from its common dir — the same project, elsewhere.
    #[serde(rename = "linked_worktree")]
    LinkedWorktree,
    /// A `.git` file whose git dir *is* its common dir — its own repository, checked out here.
    #[serde(rename = "separate_git_dir")]
    SeparateGitDir,
    /// No `.git` entry: `HEAD` + `objects/` + `refs/`, confirmed by `--is-bare-repository`.
    #[serde(rename = "bare")]
    Bare,
}

impl RepoKind {
    /// False for `Bare` alone. A bare repository has no files to inventory and no submodules.
    #[must_use]
    pub const fn has_worktree(self) -> bool {
        !matches!(self, Self::Bare)
    }

    /// The `location.repo_kind` text, which the serde renames spell identically.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::WorkTree => "worktree",
            Self::LinkedWorktree => "linked_worktree",
            Self::SeparateGitDir => "separate_git_dir",
            Self::Bare => "bare",
        }
    }

    /// R7: `as_str`'s inverse. `None` for anything else — a `repo_kind` the code does not know is
    /// a row from a newer schema, and guessing at one would attach a worktree to a bare
    /// repository. It is an inherent function, not `std::str::FromStr`: the caller wants
    /// `Option`, and a `FromStr` impl would need an error type nothing else uses.
    ///
    /// The `allow` is required, not cosmetic: `clippy::should_implement_trait` is in
    /// `clippy::all`, which this crate denies, and it fires on any inherent `from_str`. R7 names
    /// the method `from_str`, so the lint is silenced here rather than the ruling renamed.
    #[must_use]
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(raw: &str) -> Option<Self> {
        match raw {
            "worktree" => Some(Self::WorkTree),
            "linked_worktree" => Some(Self::LinkedWorktree),
            "separate_git_dir" => Some(Self::SeparateGitDir),
            "bare" => Some(Self::Bare),
            _ => None,
        }
    }
}

/// A directory the walk found to be a repository, before identity decides which project it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoCandidate {
    /// The directory itself: the worktree, or for a bare repository the repository.
    pub path: PathBuf,
    /// How the directory holds its repository.
    pub kind: RepoKind,
    /// The repository's git dir: the `.git` directory, the target of a `.git` file, or the bare
    /// repository.
    pub git_dir: PathBuf,
    /// Where the objects and refs live. For a linked worktree, the main repository's git dir;
    /// otherwise the same directory as `git_dir`.
    pub common_dir: PathBuf,
}

/// What a discovery probe needs beyond the directory itself.
///
/// The store key and class are **not** decoration: `SystemGit` takes a per-store slot for every
/// invocation (§3.4), so a probe that could not name its store would be capped globally or not
/// at all. They are the enclosing root's, because that is the store the run sized its queues
/// for. The cancel token is R4's, so a scan that is cancelled mid-walk does not keep spawning
/// git for directories nobody will look at.
#[derive(Debug, Clone)]
pub struct ProbeCtx<'a> {
    git: &'a dyn GitBackend,
    store: StoreKey,
    store_class: StoreClass,
    cancel: &'a CancelToken,
}

impl<'a> ProbeCtx<'a> {
    /// A probe that runs git against the enclosing root's store and stops when `cancel` fires.
    #[must_use]
    pub fn new(
        git: &'a dyn GitBackend,
        store: StoreKey,
        store_class: StoreClass,
        cancel: &'a CancelToken,
    ) -> Self {
        Self {
            git,
            store,
            store_class,
            cancel,
        }
    }

    /// The run's cancellation token, so the walk does not carry a second copy of it.
    #[must_use]
    pub const fn cancel(&self) -> &CancelToken {
        self.cancel
    }

    fn job(&self) -> JobContext<'_> {
        JobContext::new(JobClass::Background, self.cancel, Some(GIT_PROBE_TIMEOUT))
    }

    /// `rev-parse` for one candidate. `None` when git could not answer; the caller reports it
    /// and falls back rather than dropping the repository (criterion 5).
    fn facts(&self, repo: &RepoHandle) -> Option<RepoFacts> {
        self.git.repo_facts(repo, &self.job()).ok()
    }

    /// §4.4's gitlink OIDs for one superproject. An unreadable index yields an empty map, so the
    /// edge is written with `gitlink_oid` absent — never zero and never a placeholder.
    pub(crate) fn gitlinks(
        &self,
        parent: &Path,
        paths: &[Vec<u8>],
    ) -> std::collections::BTreeMap<Vec<u8>, String> {
        RepoHandle::resolve(parent, self.store.clone(), self.store_class)
            .ok()
            .and_then(|repo| self.git.submodule_gitlinks(&repo, paths, &self.job()).ok())
            .unwrap_or_default()
    }
}

/// The `gitdir:` line of a `.git` file.
#[must_use]
pub fn parse_gitdir_pointer(bytes: &[u8]) -> Option<PathBuf> {
    let text = std::str::from_utf8(bytes).ok()?;
    text.lines().find_map(|line| {
        let value = line.trim().strip_prefix("gitdir:")?.trim();
        (!value.is_empty()).then(|| PathBuf::from(value))
    })
}

/// Classify one directory. Returns `None` for a directory that is not a repository, and for one
/// that is unreadable — in which case a `ScanProblem` has already gone to `sink`.
pub fn classify_dir(
    dir: &Path,
    opts: &WalkOptions,
    probe: &ProbeCtx<'_>,
    sink: &WalkSink<'_>,
) -> Option<RepoCandidate> {
    let dot = dir.join(".git");
    match std::fs::metadata(&dot) {
        Ok(meta) if meta.is_dir() => Some(RepoCandidate {
            path: dir.to_path_buf(),
            kind: RepoKind::WorkTree,
            git_dir: dot.clone(),
            common_dir: dot,
        }),
        Ok(meta) if meta.is_file() => classify_dot_git_file(dir, &dot, meta.len(), probe, sink),
        _ => classify_bare(dir, opts, probe, sink),
    }
}

fn classify_dot_git_file(
    dir: &Path,
    dot: &Path,
    len: u64,
    probe: &ProbeCtx<'_>,
    sink: &WalkSink<'_>,
) -> Option<RepoCandidate> {
    if len > DOT_GIT_FILE_CAP {
        problem(
            sink,
            dir,
            format!("`.git` is {len} bytes; a gitdir pointer is one line"),
        );
        return None;
    }
    let bytes = match std::fs::read(dot) {
        Ok(bytes) => bytes,
        Err(err) => {
            problem(sink, dir, format!("`.git` unreadable: {err}"));
            return None;
        }
    };
    let Some(pointer) = parse_gitdir_pointer(&bytes) else {
        problem(
            sink,
            dir,
            "`.git` is a file with no `gitdir:` line".to_owned(),
        );
        return None;
    };
    // A `gitdir:` may be relative, and it is relative to the directory holding the `.git` file.
    let pointer = if pointer.is_absolute() {
        pointer
    } else {
        dir.join(pointer)
    };

    let probed = RepoHandle::resolve(dir, probe.store.clone(), probe.store_class)
        .ok()
        .and_then(|repo| probe.facts(&repo));

    let (git_dir, common_dir, linked) = if let Some(facts) = probed {
        let linked = facts.is_linked_worktree();
        (facts.git_dir, facts.common_dir, linked)
    } else {
        problem(
            sink,
            dir,
            "rev-parse could not read this repository; classified from the gitdir pointer"
                .to_owned(),
        );
        let (git_dir, common_dir) = fallback_from_pointer(&pointer);
        let linked = git_dir != common_dir;
        (git_dir, common_dir, linked)
    };

    let kind = if linked {
        RepoKind::LinkedWorktree
    } else {
        RepoKind::SeparateGitDir
    };
    Some(RepoCandidate {
        path: dir.to_path_buf(),
        kind,
        git_dir,
        common_dir,
    })
}

/// A linked worktree's git dir is `<common>/worktrees/<name>`; nothing else in git's layout uses
/// that component, so the shape alone answers the question when the probe cannot.
fn fallback_from_pointer(pointer: &Path) -> (PathBuf, PathBuf) {
    let components: Vec<Component<'_>> = pointer.components().collect();
    if let Some(idx) = components
        .iter()
        .rposition(|c| c.as_os_str() == "worktrees")
    {
        let common: PathBuf = components.iter().copied().take(idx).collect();
        return (pointer.to_path_buf(), common);
    }
    (pointer.to_path_buf(), pointer.to_path_buf())
}

fn problem(sink: &WalkSink<'_>, dir: &Path, detail: String) {
    sink(WalkEvent::Problem(ScanProblem {
        kind: ScanProblemKind::UnreadableRepo,
        path_display: path_display(dir),
        detail,
    }));
}

/// §4.2 — a bare repository has **no `.git` entry at all**, which is why v1's detector could not
/// find them despite the spec claiming they were indexed. Found by shape, confirmed by git.
///
/// `opts.bare_candidates` is the "only under explicitly enabled roots" clause: the run layer sets
/// it from the `scan_root` row and clears it for any directory reached another way.
///
/// **Every `.git` directory has this shape too.** The only thing preventing a second, phantom
/// repository per real one is that the walk never descends into a directory named `.git`; that
/// guard lives in `walk.rs` and is asserted there by removing it.
fn classify_bare(
    dir: &Path,
    opts: &WalkOptions,
    probe: &ProbeCtx<'_>,
    sink: &WalkSink<'_>,
) -> Option<RepoCandidate> {
    if !opts.bare_candidates {
        return None;
    }
    if !dir.join("HEAD").is_file() || !dir.join("objects").is_dir() || !dir.join("refs").is_dir() {
        return None;
    }
    let repo = RepoHandle::bare(dir, probe.store.clone(), probe.store_class);
    let Some(facts) = probe.facts(&repo) else {
        problem(
            sink,
            dir,
            "rev-parse could not read this bare-shaped directory".to_owned(),
        );
        return None;
    };
    if !facts.is_bare {
        return None;
    }
    Some(RepoCandidate {
        path: dir.to_path_buf(),
        kind: RepoKind::Bare,
        git_dir: dir.to_path_buf(),
        common_dir: dir.to_path_buf(),
    })
}
