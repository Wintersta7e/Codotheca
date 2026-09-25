//! §9's activity scope, and the fold from a batch of observed paths to one [`Signal`].
//!
//! §9 counts *a change to a tracked file, or to a non-ignored untracked file*, and the reason it
//! is that narrow is stated there: a dev server writing into a build directory must not hold a
//! segment open. v2 left this unscoped, so a watch process could have credited a whole night.
//!
//! The verdict itself belongs to [`crate::git::check_ignore`], which puts the invocation where
//! the §17 read-only audit reads it. What lives here is the policy around it: what never reaches
//! git at all, what is remembered, and how a batch of paths becomes one signal.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use crate::git::{GitError, RepoHandle};
use crate::session::segment::Signal;

pub use crate::git::CHECK_IGNORE_BATCH;

/// Distinct verdicts kept per session. Bounded so a pathological writer cannot grow the process.
pub const SCOPE_CACHE_CAP: usize = 4_096;

/// The source of §9's scope verdicts: is a worktree path ignored by the repository's rules?
pub trait IgnoreCheck: Send + Sync + std::fmt::Debug {
    /// One verdict per input, in order: `true` means "ignored, and therefore out of §9's scope".
    ///
    /// # Errors
    ///
    /// A `GitError` when no verdict could be had for the batch, e.g. the `check-ignore`
    /// invocation failed. [`ScopeFilter::fold`] treats that as out of scope and caches nothing.
    fn ignored(&self, repo: &RepoHandle, rel: &[PathBuf]) -> Result<Vec<bool>, GitError>;
}

/// `.git` is metadata, and the app's own observation jobs write into it. Never activity, and
/// never worth a git process either — asking git about its own directory would be circular.
#[must_use]
pub fn is_git_internal(rel: &Path) -> bool {
    rel.components()
        .any(|c| matches!(c, Component::Normal(name) if name == ".git"))
}

/// The production [`IgnoreCheck`]: the repository's own rules, via `check-ignore`.
#[derive(Debug)]
pub struct GitIgnoreCheck {
    /// The shared git executor every invocation goes through.
    pub exec: Arc<crate::git::GitExec>,
    /// The time and output bounds applied to each `check-ignore` run.
    pub limits: crate::git::RunLimits,
}

impl IgnoreCheck for GitIgnoreCheck {
    fn ignored(&self, repo: &RepoHandle, rel: &[PathBuf]) -> Result<Vec<bool>, GitError> {
        crate::git::check_ignore(
            &self.exec,
            repo,
            self.limits,
            &crate::cancel::CancelToken::new(),
            rel,
        )
    }
}

/// Folds a batch of observed paths into the one question §9 asks: did anything in scope change?
#[derive(Debug)]
pub struct ScopeFilter {
    check: Arc<dyn IgnoreCheck>,
    /// Keyed on the full relative path. A parent-directory key would collapse a build directory
    /// to one question but would take a force-added tracked file inside it down too.
    verdicts: BTreeMap<PathBuf, bool>,
}

impl ScopeFilter {
    /// A filter with no verdicts remembered yet, asking `check` for each new path.
    #[must_use]
    pub fn new(check: Arc<dyn IgnoreCheck>) -> Self {
        Self {
            check,
            verdicts: BTreeMap::new(),
        }
    }

    /// How many verdicts are remembered. Never above [`SCOPE_CACHE_CAP`].
    #[must_use]
    pub fn cached(&self) -> usize {
        self.verdicts.len()
    }

    /// `Signal::Worktree` when any path is in §9's scope, else `Signal::None`.
    ///
    /// Paths under `.git` are skipped, remembered verdicts answer without git, and the rest go to
    /// the check in one batch.
    pub fn fold(&mut self, repo: &RepoHandle, paths: &[PathBuf]) -> Signal {
        let mut ask: Vec<PathBuf> = Vec::new();
        let mut in_scope = false;
        for path in paths {
            if is_git_internal(path) {
                continue;
            }
            match self.verdicts.get(path) {
                Some(true) => {}
                Some(false) => in_scope = true,
                None => ask.push(path.clone()),
            }
        }
        if in_scope || ask.is_empty() {
            return if in_scope {
                Signal::Worktree
            } else {
                Signal::None
            };
        }
        // A failed check is treated as "not in scope". Crediting on a git failure would credit
        // exactly the dev-server case §9 exists to exclude, and a failure is not evidence of
        // work. Nothing is cached from a failed batch, so the question is asked again.
        let Ok(ignored) = self.check.ignored(repo, &ask) else {
            return Signal::None;
        };
        for (path, is_ignored) in ask.into_iter().zip(ignored) {
            if !is_ignored {
                in_scope = true;
            }
            if self.verdicts.len() < SCOPE_CACHE_CAP {
                self.verdicts.insert(path, is_ignored);
            }
        }
        if in_scope {
            Signal::Worktree
        } else {
            Signal::None
        }
    }
}

/// A recording [`IgnoreCheck`] that treats any path under one of `ignored_prefixes` as ignored.
#[cfg(feature = "testkit")]
#[derive(Debug, Default)]
pub struct FakeIgnoreCheck {
    prefixes: Vec<String>,
    calls: std::sync::atomic::AtomicUsize,
    asked: std::sync::Mutex<Vec<PathBuf>>,
}

#[cfg(feature = "testkit")]
impl FakeIgnoreCheck {
    /// A check that calls a path ignored when its text starts with any of `ignored_prefixes`.
    #[must_use]
    pub fn new(ignored_prefixes: &[&str]) -> Self {
        Self {
            prefixes: ignored_prefixes.iter().map(|p| (*p).to_owned()).collect(),
            calls: std::sync::atomic::AtomicUsize::new(0),
            asked: std::sync::Mutex::new(Vec::new()),
        }
    }

    /// How many times a verdict was asked for. The cache is what keeps this at one.
    #[must_use]
    pub fn calls(&self) -> usize {
        self.calls.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Every path a verdict was asked about, in order.
    ///
    /// # Panics
    /// If a previous caller panicked while holding the lock.
    #[must_use]
    pub fn asked(&self) -> Vec<PathBuf> {
        self.asked
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

#[cfg(feature = "testkit")]
impl IgnoreCheck for FakeIgnoreCheck {
    fn ignored(&self, _repo: &RepoHandle, rel: &[PathBuf]) -> Result<Vec<bool>, GitError> {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.asked
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .extend(rel.iter().cloned());
        Ok(rel
            .iter()
            .map(|p| {
                let text = p.to_string_lossy().into_owned();
                self.prefixes.iter().any(|prefix| text.starts_with(prefix))
            })
            .collect())
    }
}

#[cfg(all(test, feature = "testkit"))]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use super::*;
    use crate::git::StoreKey;
    use crate::mount::StoreClass;

    /// The fake never touches it, and no test in this file spawns git.
    fn repo() -> RepoHandle {
        RepoHandle::bare(Path::new("/w/repo"), StoreKey::new("s"), StoreClass::Local)
    }

    fn rel(paths: &[&str]) -> Vec<PathBuf> {
        paths.iter().map(PathBuf::from).collect()
    }

    #[test]
    fn a_build_directory_never_extends_a_segment() {
        // §9: "A dev server writing into dist/ must not hold a segment open."
        let check = Arc::new(FakeIgnoreCheck::new(&["dist/", "node_modules/"]));
        let mut filter = ScopeFilter::new(check);
        let signal = filter.fold(&repo(), &rel(&["dist/app.js", "dist/app.css"]));
        assert_eq!(signal, Signal::None);
    }

    #[test]
    fn one_source_save_among_a_thousand_build_writes_is_in_scope() {
        let check = Arc::new(FakeIgnoreCheck::new(&["dist/"]));
        let mut filter = ScopeFilter::new(check);
        let mut paths = rel(&["src/main.rs"]);
        paths.extend((0..1_000).map(|n| PathBuf::from(format!("dist/chunk-{n}.js"))));
        assert_eq!(filter.fold(&repo(), &paths), Signal::Worktree);
    }

    #[test]
    fn gits_own_metadata_is_never_activity_and_never_reaches_git() {
        let check = Arc::new(FakeIgnoreCheck::new(&[]));
        let mut filter = ScopeFilter::new(check.clone());
        let signal = filter.fold(&repo(), &rel(&[".git/index.lock", ".git/refs/heads/main"]));
        assert_eq!(signal, Signal::None);
        assert_eq!(check.calls(), 0, "no git process for a path under .git");
        assert!(is_git_internal(Path::new(".git/index")));
        assert!(!is_git_internal(Path::new("src/git/mod.rs")));
    }

    #[test]
    fn a_verdict_is_asked_for_once_and_then_cached() {
        let check = Arc::new(FakeIgnoreCheck::new(&["dist/"]));
        let mut filter = ScopeFilter::new(check.clone());
        for _ in 0..50 {
            assert_eq!(filter.fold(&repo(), &rel(&["dist/app.js"])), Signal::None);
        }
        assert_eq!(check.calls(), 1, "50 events, one question");
        assert_eq!(filter.cached(), 1);
    }

    #[test]
    fn the_question_is_asked_about_the_path_and_never_about_its_directory() {
        // The rejected optimisation, guarded: collapsing a batch to its parent directories would
        // be cheaper and would drop a force-added tracked file inside an ignored one, because
        // `check-ignore` is index-aware for a path and a directory is not a tracked path.
        let check = Arc::new(FakeIgnoreCheck::new(&["dist/"]));
        let mut filter = ScopeFilter::new(check.clone());
        filter.fold(&repo(), &rel(&["dist/a.js", "dist/b.js"]));
        assert_eq!(check.asked(), rel(&["dist/a.js", "dist/b.js"]));
        assert_eq!(filter.cached(), 2, "one verdict each, not one for the pair");
    }

    #[test]
    fn a_failed_check_is_not_treated_as_activity_and_is_not_remembered() {
        // Crediting on a git failure would credit exactly the case §9 excludes, and a failure is
        // not a verdict, so nothing may be cached from it.
        #[derive(Debug)]
        struct Failing;
        impl IgnoreCheck for Failing {
            fn ignored(&self, _repo: &RepoHandle, _rel: &[PathBuf]) -> Result<Vec<bool>, GitError> {
                Err(GitError::Internal {
                    detail: "no git".to_owned(),
                })
            }
        }
        let mut filter = ScopeFilter::new(Arc::new(Failing));
        assert_eq!(filter.fold(&repo(), &rel(&["src/main.rs"])), Signal::None);
        assert_eq!(filter.cached(), 0);
    }

    #[test]
    fn the_cache_is_bounded_so_a_pathological_writer_cannot_grow_it_forever() {
        let check = Arc::new(FakeIgnoreCheck::new(&[]));
        let mut filter = ScopeFilter::new(check);
        let paths: Vec<PathBuf> = (0..SCOPE_CACHE_CAP + 500)
            .map(|n| PathBuf::from(format!("src/f{n}.rs")))
            .collect();
        filter.fold(&repo(), &paths);
        assert!(filter.cached() <= SCOPE_CACHE_CAP);
    }

    #[test]
    fn an_empty_batch_is_not_activity() {
        let check = Arc::new(FakeIgnoreCheck::new(&[]));
        let mut filter = ScopeFilter::new(check.clone());
        assert_eq!(filter.fold(&repo(), &[]), Signal::None);
        assert_eq!(check.calls(), 0);
    }
}
