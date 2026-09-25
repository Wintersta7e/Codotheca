//! One builder per fixture family. Every builder is handed the same context and returns the
//! manifest rows it produced.
//!
//! Fixture ids are described by shape. Nothing here names a real project, and nothing here
//! contains a real path — every path is composed from the corpus root at run time.

use std::path::{Path, PathBuf};

use crate::corpus::{CorpusError, CorpusFixture, CorpusGit, CorpusOptions, FixtureExpect};

mod basic;
mod hostile;
mod identity;
mod large;
mod nesting;

/// Three commits and a remote; every fixture that needs a source clones it.
pub const UPSTREAM: &str = "upstream";
/// An unrelated history, the ambiguous-lineage fixture's second candidate.
pub const OTHER_UPSTREAM: &str = "other-upstream";
/// `git init` and nothing else: an unborn `HEAD`.
pub const ZERO_COMMIT: &str = "zero-commit";
/// A bare clone of the upstream, with no `.git` entry at all.
pub const BARE: &str = "bare";
/// A depth-1 clone of the upstream, excluded from every history statistic.
pub const SHALLOW: &str = "shallow";
/// Two unrelated histories merged, so `HEAD` reaches two root commits.
pub const MULTI_ROOT: &str = "multi-root";
/// A commit dated 2100-01-01, which must never become the most recently touched.
pub const FUTURE_DATED: &str = "future-dated";
/// A repository whose `index.lock` is held, so every git write refuses.
pub const INDEX_LOCK_HELD: &str = "index-lock-held";
/// A large untracked area and almost no history — what makes `git status` expensive.
pub const HUGE_UNTRACKED: &str = "huge-untracked";
/// The upstream's root commit plus one of its own on a different remote: two projects, one
/// lineage.
pub const FORK: &str = "fork";
/// The upstream cloned onto the fixed volume; [`COPY_TWO`] is the same clone on the removable
/// one.
pub const COPY_ONE: &str = "copy-one";
/// The upstream cloned onto the removable volume, pointing at the same remote as [`COPY_ONE`].
pub const COPY_TWO: &str = "copy-two";
/// Two root commits and no remote: two candidate lineages and nothing to break the tie.
pub const AMBIGUOUS_LINEAGE: &str = "ambiguous-lineage";
/// An ordinary repository whose worktree holds [`REPO_INSIDE_REPO_INNER`].
pub const REPO_INSIDE_REPO_OUTER: &str = "repo-inside-repo-outer";
/// A plain `git init` inside the outer repository's worktree, which nothing tracks.
pub const REPO_INSIDE_REPO_INNER: &str = "repo-inside-repo-inner";
/// A one-commit repository that [`LINKED_WORKTREE`] adds a second working tree to.
pub const WORKTREE_PARENT: &str = "worktree-parent";
/// A second working tree of the worktree parent: the same project in an extra location.
pub const LINKED_WORKTREE: &str = "linked-worktree";
/// A repository carrying a submodule that carries a submodule of its own.
pub const SUBMODULE_PARENT: &str = "submodule-parent";
/// The submodule at `sub` inside the submodule parent.
pub const SUBMODULE_CHILD: &str = "submodule-child";
/// The submodule at `inner` inside the submodule child.
pub const SUBMODULE_NESTED: &str = "submodule-nested";
/// A tracked path whose bytes are not valid UTF-8, built through the index.
pub const NON_UTF8_PATH: &str = "non-utf8-path";
/// A tracked file whose absolute path is longer than 260 characters.
pub const LONG_PATH: &str = "long-path";
/// A directory symlink pointing at its own parent; skipped where the platform refuses symlinks.
pub const SYMLINK_CYCLE: &str = "symlink-cycle";
/// An ordinary repository standing in for one another user owns; git's refusal is injected by
/// the fake backend.
pub const DUBIOUS_OWNERSHIP: &str = "dubious-ownership";
/// A long generated history, built only with `--large`.
pub const DEEP_HISTORY: &str = "deep-history";

/// Build order. A fixture may only depend on one that appears before it.
pub const ORDER: &[&str] = &[
    UPSTREAM,
    OTHER_UPSTREAM,
    ZERO_COMMIT,
    BARE,
    SHALLOW,
    MULTI_ROOT,
    FUTURE_DATED,
    INDEX_LOCK_HELD,
    HUGE_UNTRACKED,
    FORK,
    COPY_ONE,
    COPY_TWO,
    AMBIGUOUS_LINEAGE,
    REPO_INSIDE_REPO_OUTER,
    REPO_INSIDE_REPO_INNER,
    WORKTREE_PARENT,
    LINKED_WORKTREE,
    SUBMODULE_PARENT,
    SUBMODULE_CHILD,
    SUBMODULE_NESTED,
    NON_UTF8_PATH,
    LONG_PATH,
    SYMLINK_CYCLE,
    DUBIOUS_OWNERSHIP,
    DEEP_HISTORY,
];

/// What a fixture needs built before it. `--only` expands through this.
#[must_use]
pub fn dependencies(name: &str) -> &'static [&'static str] {
    match name {
        BARE | SHALLOW | FORK | COPY_ONE | COPY_TWO => &[UPSTREAM],
        AMBIGUOUS_LINEAGE => &[UPSTREAM, OTHER_UPSTREAM],
        LINKED_WORKTREE => &[WORKTREE_PARENT],
        REPO_INSIDE_REPO_INNER => &[REPO_INSIDE_REPO_OUTER],
        SUBMODULE_CHILD | SUBMODULE_NESTED => &[SUBMODULE_PARENT],
        _ => &[],
    }
}

/// What every fixture builder is given.
#[derive(Debug, Clone, Copy)]
pub struct Ctx<'a> {
    /// The hermetic git every builder runs.
    pub git: &'a CorpusGit,
    /// The fixed local volume.
    pub vol_a: &'a Path,
    /// The simulated removable volume.
    pub vol_b: &'a Path,
    /// Clone sources, outside both volumes so a scan never discovers them.
    pub sources: &'a Path,
    /// The run's options: whether large fixtures are built, and how big.
    pub options: &'a CorpusOptions,
}

pub(crate) fn row(name: &str, volume: &str, path: PathBuf, expect: FixtureExpect) -> CorpusFixture {
    CorpusFixture {
        name: name.to_owned(),
        volume: volume.to_owned(),
        path,
        materialised: true,
        skip_reason: None,
        expect,
    }
}

/// A fixture the platform refused to build. Recorded with its reason rather than omitted, so
/// `require` fails loudly and no test passes over a repository that is not there.
pub(crate) fn skipped(name: &str, volume: &str, path: PathBuf, reason: &str) -> CorpusFixture {
    CorpusFixture {
        name: name.to_owned(),
        volume: volume.to_owned(),
        path,
        materialised: false,
        skip_reason: Some(reason.to_owned()),
        expect: FixtureExpect::default(),
    }
}

/// HEAD and every root commit reachable from it. Both are absent on an unborn HEAD, which is
/// a state, not a failure.
pub(crate) fn observe(
    ctx: &Ctx<'_>,
    path: &Path,
) -> Result<(Option<String>, Vec<String>), CorpusError> {
    let head = ctx.git.run(path, 0, &["rev-parse", "HEAD"]).ok();
    let roots = match head.as_ref() {
        None => Vec::new(),
        Some(_) => ctx
            .git
            .run(path, 0, &["rev-list", "--max-parents=0", "HEAD"])?
            .lines()
            .map(str::to_owned)
            .collect(),
    };
    Ok((head, roots))
}

pub(crate) fn build(ctx: &Ctx<'_>, name: &str) -> Result<Vec<CorpusFixture>, CorpusError> {
    const fn unhandled(result: &Result<Vec<CorpusFixture>, CorpusError>) -> bool {
        matches!(result, Err(CorpusError::UnknownFixture(_)))
    }
    // Written as a chain of statements rather than an array of function items: each builder
    // is its own type, and coercing them to a common pointer over a lifetime-generic `Ctx`
    // is more subtlety than a dispatch table is worth.
    let mut result = basic::build(ctx, name);
    if unhandled(&result) {
        result = identity::build(ctx, name);
    }
    if unhandled(&result) {
        result = nesting::build(ctx, name);
    }
    if unhandled(&result) {
        result = hostile::build(ctx, name);
    }
    if unhandled(&result) {
        result = large::build(ctx, name);
    }
    result
}
