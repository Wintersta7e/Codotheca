//! J0 — the directory walk.
//!
//! Measured at 101k dirs/s over 32k directories at 8 threads: one second for 100k directories.
//! It was written up as the headline risk and is not one, so this reads for correctness.

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use ignore::{DirEntry, WalkBuilder, WalkState};

use crate::paths::path_display;
use crate::scan::discover::{classify_dir, ProbeCtx};
use crate::scan::links::LinkPolicy;
use crate::scan::skiplist::SkipList;
use crate::scan::submodules::enumerate_submodules;
use crate::scan::wsl::wsl_boundary;
use crate::scan::{ScanProblem, ScanProblemKind, WalkEvent, WalkOptions, WalkSink};

/// One `Walked` event per this many directories, so a large root does not flood the pipe.
const PROGRESS_EVERY: u64 = 512;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WalkStats {
    pub walked_dirs: u64,
    pub repos_found: u64,
    pub links_refused: u64,
    pub cancelled: bool,
}

/// Everything one root's walk needs. Shared across worker threads, hence `Sync` throughout.
///
/// The cancellation token lives on `probe` rather than beside it: the walk and the `rev-parse`
/// probes it makes are cancelled by the same §4.8 token, and two fields would be two places for
/// one run's cancellation to be wired up wrongly.
pub struct WalkCtx<'a> {
    pub opts: &'a WalkOptions,
    pub skip: &'a SkipList,
    pub probe: &'a ProbeCtx<'a>,
    pub links: Arc<LinkPolicy>,
}

impl std::fmt::Debug for WalkCtx<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WalkCtx")
            .field("opts", self.opts)
            .finish_non_exhaustive()
    }
}

pub fn walk_root(root: &Path, ctx: &WalkCtx<'_>, sink: &WalkSink<'_>) -> WalkStats {
    let walked = AtomicU64::new(0);
    let found = AtomicU64::new(0);
    let quit = AtomicBool::new(false);

    let mut builder = WalkBuilder::new(root);
    builder
        // Every "standard filter" is a gitignore behaviour. This is a filesystem census, not a
        // git operation: nothing here may be hidden by a `.gitignore` or by a leading dot.
        .standard_filters(false)
        .follow_links(ctx.opts.follow_links)
        .threads(ctx.opts.threads);

    {
        let links = Arc::clone(&ctx.links);
        builder.filter_entry(move |entry: &DirEntry| {
            !entry.path_is_symlink() || links.judge(entry.path()).is_follow()
        });
    }

    // The visitor is a `move` closure built once per worker thread, so it must capture things
    // that are `Copy`. Shared references to the counters are; the counters themselves are not,
    // and moving them would leave nothing to read the totals from below.
    let (walked, found, quit) = (&walked, &found, &quit);

    builder.build_parallel().run(|| {
        Box::new(move |result| {
            if ctx.probe.cancel().is_cancelled() {
                quit.store(true, Ordering::Relaxed);
                return WalkState::Quit;
            }
            let entry = match result {
                Ok(entry) => entry,
                Err(err) => {
                    if let Some(problem) = problem_from(&err) {
                        sink(WalkEvent::Problem(problem));
                    }
                    return WalkState::Continue;
                }
            };
            if !entry.file_type().is_some_and(|ft| ft.is_dir()) {
                return WalkState::Continue;
            }
            let dir = entry.path();
            let seen = walked.fetch_add(1, Ordering::Relaxed) + 1;
            if seen % PROGRESS_EVERY == 0 {
                sink(WalkEvent::Walked { dirs: seen });
            }

            // 1. A `.git` directory has the bare shape (HEAD + objects/ + refs/). Entering one
            //    would report a second, phantom repository for every real one.
            if dir.file_name().is_some_and(|name| name == ".git") {
                return WalkState::Skip;
            }
            // 2. §4.5 — the bridge is registered, never traversed.
            if let Some(bridge) = wsl_boundary(dir) {
                sink(WalkEvent::WslBridge {
                    distro: bridge.distro,
                    path_display: path_display(dir),
                });
                return WalkState::Skip;
            }
            // 3. §4.3 — descendants only. A root the user chose is always read.
            //    `root` is a `&Path` and therefore `Copy`, so the `move` closure may hold it;
            //    a `PathBuf` would be moved out of an `FnMut` and would not compile.
            if dir != root && ctx.skip.skips(dir) {
                return WalkState::Skip;
            }

            let Some(candidate) = classify_dir(dir, ctx.opts, ctx.probe, sink) else {
                return WalkState::Continue;
            };
            found.fetch_add(1, Ordering::Relaxed);
            let has_worktree = candidate.kind.has_worktree();
            sink(WalkEvent::Repo(candidate));

            if has_worktree {
                let edges = enumerate_submodules(
                    dir,
                    ctx.opts,
                    ctx.probe,
                    // When descending we will reach them anyway; enumerate for the edges only.
                    !ctx.opts.descend_into_repos,
                    sink,
                );
                if !ctx.opts.descend_into_repos {
                    found.fetch_add(
                        u64::try_from(edges.len()).unwrap_or(u64::MAX),
                        Ordering::Relaxed,
                    );
                }
            }

            if ctx.opts.descend_into_repos {
                WalkState::Continue
            } else {
                WalkState::Skip
            }
        })
    });

    WalkStats {
        walked_dirs: walked.load(Ordering::Relaxed),
        repos_found: found.load(Ordering::Relaxed),
        links_refused: ctx.links.refused(),
        cancelled: quit.load(Ordering::Relaxed),
    }
}

/// A symlink loop is not a problem group — §11.1's eight are closed — so it is counted by the
/// link policy and dropped here.
fn problem_from(err: &ignore::Error) -> Option<ScanProblem> {
    let path = error_path(err).map_or_else(String::new, path_display);
    let inner = innermost(err);
    if matches!(inner, ignore::Error::Loop { .. }) {
        return None;
    }
    let io = inner.io_error()?;
    let kind = match io.kind() {
        std::io::ErrorKind::PermissionDenied => ScanProblemKind::PermissionDenied,
        _ => ScanProblemKind::UnreadableRepo,
    };
    Some(ScanProblem {
        kind,
        path_display: path,
        detail: io.to_string(),
    })
}

/// The path an error is about. `ignore::Error` carries it in a `WithPath` wrapper and exposes no
/// accessor, so the wrappers are unwrapped by hand; a problem row with no path is unactionable in
/// §11.1's window, which is why this is worth doing rather than reporting an empty string.
fn error_path(err: &ignore::Error) -> Option<&Path> {
    match err {
        ignore::Error::WithPath { path, .. } => Some(path),
        ignore::Error::WithDepth { err, .. } | ignore::Error::WithLineNumber { err, .. } => {
            error_path(err)
        }
        ignore::Error::Loop { child, .. } => Some(child),
        _ => None,
    }
}

fn innermost(err: &ignore::Error) -> &ignore::Error {
    match err {
        ignore::Error::WithPath { err, .. }
        | ignore::Error::WithDepth { err, .. }
        | ignore::Error::WithLineNumber { err, .. } => innermost(err),
        other => other,
    }
}
