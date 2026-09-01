//! §10.6's rescan triggers.
//!
//! Four mechanisms, and only two of them live here. On focus, J2 for visible tiles is the
//! renderer's and never arms the rescan line — an indicator that runs on every alt-tab is a
//! scheduled animation at idle wearing a different name. Manual rescan is a control in settings
//! and in the scan summary, and both call `scan.start`.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};

use notify::{RecommendedWatcher, RecursiveMode, Watcher as _};

use crate::scan::presence::ScanRootRow;
use crate::scan::run::ScanMode;

/// A full walk runs on launch only when the last one is older than this.
pub const FULL_WALK_AFTER_SECS: i64 = 86_400;

/// What the launch trigger asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchDecision {
    FullWalk,
    Incremental,
}

impl LaunchDecision {
    #[must_use]
    pub fn mode(self) -> ScanMode {
        match self {
            LaunchDecision::FullWalk => ScanMode::Full,
            LaunchDecision::Incremental => ScanMode::Incremental,
        }
    }
}

/// §10.6 mechanism 1.
///
/// A generation walk always runs; the question is only whether it is the full one. A clock that
/// has moved backwards reads as recent rather than as stale, so a bad clock cannot pin the app
/// into a full walk on every launch.
#[must_use]
pub fn on_launch(last_scan_at: Option<i64>, now: i64) -> LaunchDecision {
    match last_scan_at {
        None => LaunchDecision::FullWalk,
        Some(at) if now.saturating_sub(at) >= FULL_WALK_AFTER_SECS => LaunchDecision::FullWalk,
        Some(_) => LaunchDecision::Incremental,
    }
}

/// §10.6 mechanism 3: every enabled root and its direct children, and nothing deeper.
///
/// A new repository is almost always a new *direct* child of a place the user already keeps
/// repositories, so two levels catch nearly all of them. Watching the whole tree would be
/// 100,000 watch descriptors for the same answer.
#[must_use]
pub fn watch_targets(
    roots: &[ScanRootRow],
    children_of: &dyn Fn(&Path) -> Vec<PathBuf>,
) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for root in roots.iter().filter(|r| r.enabled) {
        let path = crate::paths::path_from_bytes(&root.path_bytes);
        if !out.contains(&path) {
            out.push(path.clone());
        }
        for child in children_of(&path) {
            if !out.contains(&child) {
                out.push(child);
            }
        }
    }
    out
}

/// The failure a watch can produce.
#[derive(Debug)]
pub enum RootWatchError {
    Notify(String),
}

impl std::fmt::Display for RootWatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RootWatchError::Notify(detail) => write!(f, "watch failed: {detail}"),
        }
    }
}

impl std::error::Error for RootWatchError {}

/// A non-recursive watch over a fixed set of directories.
pub struct RootWatch {
    watcher: RecommendedWatcher,
    events: Receiver<PathBuf>,
    watched: Vec<PathBuf>,
}

impl RootWatch {
    /// # Errors
    /// Returns [`RootWatchError`] when the platform watcher cannot be created.
    pub fn new() -> Result<RootWatch, RootWatchError> {
        let (tx, rx) = channel::<PathBuf>();
        let watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            if let Ok(event) = res {
                if matches!(
                    event.kind,
                    notify::EventKind::Create(_) | notify::EventKind::Remove(_)
                ) {
                    for path in event.paths {
                        let _ = tx.send(path);
                    }
                }
            }
        })
        .map_err(|e| RootWatchError::Notify(e.to_string()))?;
        Ok(RootWatch {
            watcher,
            events: rx,
            watched: Vec::new(),
        })
    }

    /// Watch exactly `wanted`, dropping anything else.
    ///
    /// # Errors
    /// Returns [`RootWatchError`] when a directory cannot be watched. A directory that has gone
    /// away is skipped rather than failing the whole retarget.
    pub fn retarget(&mut self, wanted: &[PathBuf]) -> Result<(), RootWatchError> {
        for old in &self.watched {
            if !wanted.contains(old) {
                let _ = self.watcher.unwatch(old);
            }
        }
        for path in wanted {
            if self.watched.contains(path) {
                continue;
            }
            if !path.is_dir() {
                continue;
            }
            self.watcher
                .watch(path, RecursiveMode::NonRecursive)
                .map_err(|e| RootWatchError::Notify(e.to_string()))?;
        }
        self.watched = wanted.to_vec();
        Ok(())
    }

    /// Every path reported since the last call.
    pub fn take_hits(&mut self) -> Vec<PathBuf> {
        let mut out = Vec::new();
        while let Ok(path) = self.events.try_recv() {
            if !out.contains(&path) {
                out.push(path);
            }
        }
        out
    }
}

impl std::fmt::Debug for RootWatch {
    /// Hand-written because the platform watcher and the channel are not `Debug`, and neither is
    /// what a reader wants here — how many directories are armed is.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RootWatch")
            .field("watched", &self.watched.len())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
mod tests {
    use super::*;
    use crate::index::path::PathPlatform;
    use std::path::{Path, PathBuf};

    const NOW: i64 = 1_760_000_000;

    fn root(id: i64, path: &str, enabled: bool) -> crate::scan::presence::ScanRootRow {
        crate::scan::presence::ScanRootRow {
            root_id: id,
            kind: "linux".to_owned(),
            distro: String::new(),
            path_bytes: path.as_bytes().to_vec(),
            path_key: crate::paths::path_key(Path::new(path), PathPlatform::Unix),
            enabled,
            descend_into_repos: false,
        }
    }

    // §10.6: a full J0 walk of 100k directories does not run on every launch; that would
    // violate the quiet-at-idle posture.
    #[test]
    fn a_full_walk_runs_only_when_the_last_one_is_a_day_old() {
        assert_eq!(on_launch(None, NOW), LaunchDecision::FullWalk);
        assert_eq!(
            on_launch(Some(NOW - 3_600), NOW),
            LaunchDecision::Incremental
        );
        assert_eq!(
            on_launch(Some(NOW - FULL_WALK_AFTER_SECS + 1), NOW),
            LaunchDecision::Incremental
        );
        assert_eq!(
            on_launch(Some(NOW - FULL_WALK_AFTER_SECS), NOW),
            LaunchDecision::FullWalk
        );
        // A clock that has gone backwards must not pin the app into a walk on every launch.
        assert_eq!(
            on_launch(Some(NOW + 10_000), NOW),
            LaunchDecision::Incremental
        );
    }

    #[test]
    fn the_watch_reaches_depth_two_and_no_further() {
        let children = |p: &Path| -> Vec<PathBuf> {
            match p.to_string_lossy().as_ref() {
                "/home/u/dev" => vec![
                    PathBuf::from("/home/u/dev/a"),
                    PathBuf::from("/home/u/dev/b"),
                ],
                "/home/u/dev/a" => vec![PathBuf::from("/home/u/dev/a/deep")],
                _ => Vec::new(),
            }
        };
        let targets = watch_targets(&[root(1, "/home/u/dev", true)], &children);
        assert_eq!(
            targets,
            vec![
                PathBuf::from("/home/u/dev"),
                PathBuf::from("/home/u/dev/a"),
                PathBuf::from("/home/u/dev/b"),
            ]
        );
    }

    #[test]
    fn a_disabled_root_is_not_watched() {
        let children = |_: &Path| Vec::new();
        assert!(watch_targets(&[root(1, "/home/u/dev", false)], &children).is_empty());
    }

    #[test]
    fn a_watch_reports_a_directory_created_directly_under_a_root() {
        let dir = tempfile::tempdir().unwrap();
        let mut watch = RootWatch::new().unwrap();
        watch.retarget(&[dir.path().to_path_buf()]).unwrap();

        std::fs::create_dir(dir.path().join("fresh")).unwrap();

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let mut hits = Vec::new();
        while std::time::Instant::now() < deadline && hits.is_empty() {
            std::thread::sleep(std::time::Duration::from_millis(50));
            hits = watch.take_hits();
        }
        assert!(
            !hits.is_empty(),
            "the watch reported nothing within ten seconds"
        );
    }
}
