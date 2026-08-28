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

pub const UPSTREAM: &str = "upstream";
pub const OTHER_UPSTREAM: &str = "other-upstream";
pub const ZERO_COMMIT: &str = "zero-commit";
pub const BARE: &str = "bare";
pub const SHALLOW: &str = "shallow";
pub const MULTI_ROOT: &str = "multi-root";
pub const FUTURE_DATED: &str = "future-dated";
pub const INDEX_LOCK_HELD: &str = "index-lock-held";
pub const HUGE_UNTRACKED: &str = "huge-untracked";
pub const FORK: &str = "fork";
pub const COPY_ONE: &str = "copy-one";
pub const COPY_TWO: &str = "copy-two";
pub const AMBIGUOUS_LINEAGE: &str = "ambiguous-lineage";
pub const REPO_INSIDE_REPO_OUTER: &str = "repo-inside-repo-outer";
pub const REPO_INSIDE_REPO_INNER: &str = "repo-inside-repo-inner";
pub const WORKTREE_PARENT: &str = "worktree-parent";
pub const LINKED_WORKTREE: &str = "linked-worktree";
pub const SUBMODULE_PARENT: &str = "submodule-parent";
pub const SUBMODULE_CHILD: &str = "submodule-child";
pub const SUBMODULE_NESTED: &str = "submodule-nested";
pub const NON_UTF8_PATH: &str = "non-utf8-path";
pub const LONG_PATH: &str = "long-path";
pub const SYMLINK_CYCLE: &str = "symlink-cycle";
pub const DUBIOUS_OWNERSHIP: &str = "dubious-ownership";
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
    pub git: &'a CorpusGit,
    /// The fixed local volume.
    pub vol_a: &'a Path,
    /// The simulated removable volume.
    pub vol_b: &'a Path,
    /// Clone sources, outside both volumes so a scan never discovers them.
    pub sources: &'a Path,
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
    fn unhandled(result: &Result<Vec<CorpusFixture>, CorpusError>) -> bool {
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
