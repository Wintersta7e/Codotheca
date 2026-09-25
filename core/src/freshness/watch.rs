//! §6's bounded worktree watch set. Change notification only — path names and mtimes, never file
//! contents (§10.1). Its single effect is to invalidate `worktree_observed_at`. It starts no
//! scan and discovers no repository, so it is not a fifth rescan trigger (§10.6).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};

use notify::{RecommendedWatcher, RecursiveMode, Watcher as _};

use crate::protocol::LocationId;

/// "The ~30 most recently touched projects plus every live session's project."
///
/// Bounded because watching 1,000 repositories is ruled out by inotify limits and by the stack,
/// not because 30 is a tuning parameter.
pub const WATCH_SET_MAX: usize = 30;

/// One directory the watcher is asked to follow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchTarget {
    /// The copy this root belongs to.
    pub location_id: LocationId,
    /// The worktree root, never the git directory.
    pub root: PathBuf,
    /// A live session. Pinned targets are additional to the cap, not counted against it.
    pub pinned: bool,
}

/// Choose what to watch.
///
/// `recent` is (location, worktree root, last-touched epoch seconds), any order.
#[must_use]
pub fn select_targets(
    recent: &[(LocationId, PathBuf, i64)],
    live_sessions: &[LocationId],
    max: usize,
) -> Vec<WatchTarget> {
    let mut ordered = recent.to_vec();
    ordered.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| a.0 .0.cmp(&b.0 .0)));

    let mut out: Vec<WatchTarget> = Vec::new();
    for (id, root, _) in ordered.iter().take(max) {
        out.push(WatchTarget {
            location_id: *id,
            root: root.clone(),
            pinned: live_sessions.contains(id),
        });
    }
    for id in live_sessions {
        if out.iter().any(|t| t.location_id == *id) {
            continue;
        }
        if let Some((_, root, _)) = ordered.iter().find(|(l, _, _)| l == id) {
            out.push(WatchTarget {
                location_id: *id,
                root: root.clone(),
                pinned: true,
            });
        }
    }
    out
}

/// Why a watch could not be established.
#[derive(Debug, thiserror::Error)]
pub enum WatchError {
    /// The platform watcher refused. Diagnostic, never rendered raw (§2.4).
    #[error("watch: {0}")]
    Notify(String),
}

/// Which watched root a changed path belongs to, longest prefix first.
///
/// A free function rather than a method so the resolution rule is testable without standing up
/// a real platform watcher — nested worktrees are the case that matters and they are exactly
/// what a shortest-prefix match would get wrong.
#[must_use]
pub fn resolve_root(roots: &BTreeMap<PathBuf, LocationId>, path: &Path) -> Option<LocationId> {
    roots
        .iter()
        .filter(|(root, _)| path.starts_with(root))
        .max_by_key(|(root, _)| root.as_os_str().len())
        .map(|(_, id)| *id)
}

/// The live set of platform watches.
#[derive(Debug)]
pub struct WatchSet {
    watcher: RecommendedWatcher,
    events: Receiver<PathBuf>,
    /// Watched root -> location. Longest-prefix match resolves an event to a location.
    roots: BTreeMap<PathBuf, LocationId>,
}

impl WatchSet {
    /// Start a watcher with nothing watched yet.
    pub fn new() -> Result<Self, WatchError> {
        let (tx, rx) = channel::<PathBuf>();
        let watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            if let Ok(ev) = res {
                for path in ev.paths {
                    // Path names only. Nothing reads a byte of a watched file.
                    let _ = tx.send(path);
                }
            }
        })
        .map_err(|e| WatchError::Notify(e.to_string()))?;
        Ok(Self {
            watcher,
            events: rx,
            roots: BTreeMap::new(),
        })
    }

    /// Make the watched set exactly `wanted`, adding and dropping the difference.
    pub fn retarget(&mut self, wanted: &[WatchTarget]) -> Result<(), WatchError> {
        let wanted_roots: BTreeMap<PathBuf, LocationId> = wanted
            .iter()
            .map(|t| (t.root.clone(), t.location_id))
            .collect();

        let to_drop: Vec<PathBuf> = self
            .roots
            .keys()
            .filter(|p| !wanted_roots.contains_key(*p))
            .cloned()
            .collect();
        for path in to_drop {
            let _ = self.watcher.unwatch(&path);
            self.roots.remove(&path);
        }
        for (path, id) in wanted_roots {
            if self.roots.contains_key(&path) {
                continue;
            }
            self.watcher
                .watch(&path, RecursiveMode::Recursive)
                .map_err(|e| WatchError::Notify(e.to_string()))?;
            self.roots.insert(path, id);
        }
        Ok(())
    }

    /// Drain pending notifications into the set of locations whose `worktree_observed_at`
    /// must be cleared. Nothing else happens: no scan starts, no job is enqueued here.
    pub fn take_invalidations(&mut self) -> Vec<LocationId> {
        let mut hit: Vec<LocationId> = Vec::new();
        while let Ok(path) = self.events.try_recv() {
            if let Some(id) = resolve_root(&self.roots, &path) {
                if !hit.contains(&id) {
                    hit.push(id);
                }
            }
        }
        hit
    }

    /// What is watched right now, for a test or a diagnostic.
    #[must_use]
    pub fn watched(&self) -> &BTreeMap<PathBuf, LocationId> {
        &self.roots
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
    use std::path::PathBuf;

    fn recent(n: i64) -> Vec<(LocationId, PathBuf, i64)> {
        (0..n)
            .map(|i| (LocationId(i), PathBuf::from(format!("/w/{i}")), 1_000 - i))
            .collect()
    }

    #[test]
    fn the_set_is_bounded_at_thirty() {
        // §6: watching 1,000 repositories is ruled out by inotify limits and by the stack.
        let picked = select_targets(&recent(200), &[], WATCH_SET_MAX);
        assert_eq!(picked.len(), WATCH_SET_MAX);
        assert_eq!(WATCH_SET_MAX, 30);
    }

    #[test]
    fn the_most_recently_touched_win() {
        let picked = select_targets(&recent(200), &[], 3);
        let ids: Vec<i64> = picked.iter().map(|t| t.location_id.0).collect();
        assert_eq!(ids, vec![0, 1, 2]);
    }

    #[test]
    fn every_live_session_is_pinned_beyond_the_cap() {
        let picked = select_targets(&recent(200), &[LocationId(150)], 3);
        assert_eq!(picked.len(), 4);
        let pinned: Vec<i64> = picked
            .iter()
            .filter(|t| t.pinned)
            .map(|t| t.location_id.0)
            .collect();
        assert_eq!(pinned, vec![150]);
    }

    #[test]
    fn a_live_session_already_in_the_recent_list_is_not_duplicated() {
        let picked = select_targets(&recent(200), &[LocationId(1)], 3);
        assert_eq!(picked.len(), 3);
        assert!(picked
            .iter()
            .any(|t| t.location_id == LocationId(1) && t.pinned));
    }

    #[test]
    fn targets_are_worktree_roots_not_git_dirs() {
        // §6: the whole finding is that .git/index does not move when a tracked file is edited,
        // so watching .git would watch the one place the change is not.
        let picked = select_targets(&recent(1), &[], 1);
        assert_eq!(picked[0].root, PathBuf::from("/w/0"));
        assert!(!picked[0].root.ends_with(".git"));
    }

    /// A submodule or a nested checkout sits underneath its parent's root. Shortest-prefix
    /// matching would file every one of its edits against the parent.
    #[test]
    fn a_nested_worktree_resolves_to_itself_not_its_parent() {
        let mut roots = BTreeMap::new();
        roots.insert(PathBuf::from("/w/outer"), LocationId(1));
        roots.insert(PathBuf::from("/w/outer/vendor/inner"), LocationId(2));

        assert_eq!(
            resolve_root(&roots, Path::new("/w/outer/src/a.rs")),
            Some(LocationId(1))
        );
        assert_eq!(
            resolve_root(&roots, Path::new("/w/outer/vendor/inner/src/b.rs")),
            Some(LocationId(2))
        );
        assert_eq!(resolve_root(&roots, Path::new("/elsewhere/c.rs")), None);
    }
}
