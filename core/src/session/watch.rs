//! A path-yielding recursive watch over a live session's worktree.
//!
//! **Not §6's watch set.** [`crate::freshness::watch::WatchSet`] watches the ~30 most recently
//! touched projects to invalidate `worktree_observed_at`, and returns `LocationId` with the path
//! names deliberately discarded (§6). §9 needs the path, because the in-scope filter cannot run
//! without it. Same crate, two consumers, two appetites; only a *live* session is watched here,
//! so this set is a handful of roots rather than thirty.
//!
//! **Paths only.** Nothing here opens a file, stats one, or records a size (§10.1).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc;

use notify::{RecommendedWatcher, RecursiveMode, Watcher as _};

use crate::protocol::SessionId;
use crate::session::SessionError;

/// Distinct paths buffered per session between drains.
///
/// Beyond this, further paths are **dropped** rather than treated as activity: an overflow
/// counted as activity would credit the dev-server case §9 exists to exclude. The trade is
/// deliberate — a burst that large may hide one in-scope save until the next tick, and a file
/// genuinely being worked on is seen again.
pub const ACTIVITY_PATHS_CAP: usize = 16_384;

/// The distinct paths one live session's worktree reported since the last drain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivityBatch {
    /// The live session whose watched root the paths fall under.
    pub session: SessionId,
    /// Relative to the watched root, so `IgnoreCheck` can pass them to git unchanged.
    pub paths: Vec<PathBuf>,
}

/// Where the session manager gets worktree activity: a recursive watch per live session.
pub trait ActivitySource: Send + std::fmt::Debug {
    /// Start reporting changes under `root` against `session`.
    ///
    /// # Errors
    ///
    /// `SessionError::Watch` when the OS watch on `root` cannot be established.
    fn watch(&mut self, session: SessionId, root: &Path) -> Result<(), SessionError>;
    /// Stop watching the session's root and drop any paths still buffered for it.
    fn unwatch(&mut self, session: SessionId);
    /// Every distinct path seen since the last call, grouped by session. Empties the buffer.
    fn drain(&mut self) -> Vec<ActivityBatch>;
}

/// The watcher that is not there.
///
/// §9's activity watch is a nicety, not a requirement: without it a segment closes on its idle
/// deadline instead of on the last edit. `NotifyActivitySource::new` can fail for ordinary
/// environmental reasons — an exhausted inotify budget is the common one on Linux — and that
/// must not stop the core from starting, so the composition root falls back to this and emits
/// `core/degraded`.
///
/// **Not `#[cfg(test)]`**: the production binary is its only caller.
#[derive(Debug, Default)]
pub struct NullActivitySource;

impl ActivitySource for NullActivitySource {
    fn watch(&mut self, _session: SessionId, _root: &Path) -> Result<(), SessionError> {
        Ok(())
    }

    fn unwatch(&mut self, _session: SessionId) {}

    /// Always empty — never a fabricated edit. A segment closes on its idle deadline instead.
    fn drain(&mut self) -> Vec<ActivityBatch> {
        Vec::new()
    }
}

/// Coalescing buffer shared by both implementations, so the cap and the grouping are written
/// once and the fake cannot drift from the real watcher.
///
/// Keyed on the raw row id rather than on `SessionId`: the generated ids carry `Hash` but not
/// `Ord`, and a drain has to come out in a stable order.
#[derive(Debug, Default)]
struct Pending {
    roots: BTreeMap<i64, PathBuf>,
    paths: BTreeMap<i64, BTreeSet<PathBuf>>,
}

impl Pending {
    fn insert(&mut self, session: SessionId, rel: PathBuf) {
        let slot = self.paths.entry(session.0).or_default();
        if slot.len() < ACTIVITY_PATHS_CAP {
            slot.insert(rel);
        }
    }

    /// Attribute an absolute path to the session whose root contains it, relative to that root.
    fn absorb(&mut self, absolute: &Path) {
        let found = self.roots.iter().find_map(|(session, root)| {
            absolute
                .strip_prefix(root)
                .ok()
                .map(|rel| (SessionId(*session), rel.to_path_buf()))
        });
        if let Some((session, rel)) = found {
            self.insert(session, rel);
        }
    }

    fn forget(&mut self, session: SessionId) -> Option<PathBuf> {
        self.paths.remove(&session.0);
        self.roots.remove(&session.0)
    }

    fn take(&mut self) -> Vec<ActivityBatch> {
        std::mem::take(&mut self.paths)
            .into_iter()
            .filter(|(_, paths)| !paths.is_empty())
            .map(|(session, paths)| ActivityBatch {
                session: SessionId(session),
                paths: paths.into_iter().collect(),
            })
            .collect()
    }
}

/// The production [`ActivitySource`]: one recursive OS watcher over every live session's root.
#[derive(Debug)]
pub struct NotifyActivitySource {
    watcher: RecommendedWatcher,
    rx: mpsc::Receiver<notify::Result<notify::Event>>,
    pending: Pending,
}

impl NotifyActivitySource {
    /// Create the OS watcher, watching nothing until [`ActivitySource::watch`] names a root.
    ///
    /// # Errors
    ///
    /// `SessionError::Watch` when the OS refuses a watcher, e.g. an exhausted inotify budget.
    pub fn new() -> Result<Self, SessionError> {
        let (tx, rx) = mpsc::channel();
        let watcher = notify::recommended_watcher(move |event| {
            let _ = tx.send(event);
        })
        .map_err(|err| SessionError::Watch(err.to_string()))?;
        Ok(Self {
            watcher,
            rx,
            pending: Pending::default(),
        })
    }
}

impl ActivitySource for NotifyActivitySource {
    fn watch(&mut self, session: SessionId, root: &Path) -> Result<(), SessionError> {
        self.watcher
            .watch(root, RecursiveMode::Recursive)
            .map_err(|err| SessionError::Watch(err.to_string()))?;
        // The stored root is the one paths are made relative to, so it must be the same string
        // the OS reports back; a watcher that resolved symlinks would break the strip_prefix.
        self.pending.roots.insert(session.0, root.to_path_buf());
        Ok(())
    }

    fn unwatch(&mut self, session: SessionId) {
        if let Some(root) = self.pending.forget(session) {
            let _ = self.watcher.unwatch(&root);
        }
    }

    fn drain(&mut self) -> Vec<ActivityBatch> {
        while let Ok(Ok(event)) = self.rx.try_recv() {
            for path in event.paths {
                self.pending.absorb(&path);
            }
        }
        self.pending.take()
    }
}

/// A hand-driven [`ActivitySource`]: `push` stands in for an OS event.
#[cfg(feature = "testkit")]
#[derive(Debug, Default)]
pub struct FakeActivitySource {
    pending: Pending,
}

#[cfg(feature = "testkit")]
impl FakeActivitySource {
    /// A source watching nothing and holding no paths.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record one relative path against a watched session. A path for a session that is not
    /// watched is dropped, exactly as the real watcher drops a path under no root.
    pub fn push(&mut self, session: SessionId, rel: &str) {
        if self.pending.roots.contains_key(&session.0) {
            self.pending.insert(session, PathBuf::from(rel));
        }
    }

    /// Every session with a watched root, in id order.
    #[must_use]
    pub fn watched(&self) -> Vec<SessionId> {
        self.pending.roots.keys().copied().map(SessionId).collect()
    }
}

#[cfg(feature = "testkit")]
impl ActivitySource for FakeActivitySource {
    fn watch(&mut self, session: SessionId, root: &Path) -> Result<(), SessionError> {
        self.pending.roots.insert(session.0, root.to_path_buf());
        Ok(())
    }

    fn unwatch(&mut self, session: SessionId) {
        self.pending.forget(session);
    }

    fn drain(&mut self) -> Vec<ActivityBatch> {
        self.pending.take()
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

    #[test]
    fn the_fake_yields_what_was_pushed_and_drains_exactly_once() {
        let mut src = FakeActivitySource::new();
        src.watch(SessionId(1), Path::new("/nowhere")).unwrap();
        src.push(SessionId(1), "src/main.rs");
        src.push(SessionId(1), "src/main.rs");
        let batches = src.drain();
        assert_eq!(batches.len(), 1);
        assert_eq!(
            batches.first().unwrap().paths.len(),
            1,
            "distinct paths, coalesced"
        );
        assert!(src.drain().is_empty(), "a drain empties the buffer");
    }

    #[test]
    fn two_live_sessions_do_not_share_a_batch() {
        let mut src = FakeActivitySource::new();
        src.watch(SessionId(1), Path::new("/a")).unwrap();
        src.watch(SessionId(2), Path::new("/b")).unwrap();
        src.push(SessionId(1), "a.rs");
        src.push(SessionId(2), "b.rs");
        let batches = src.drain();
        assert_eq!(batches.len(), 2);
        assert!(batches.iter().all(|b| b.paths.len() == 1));
    }

    #[test]
    fn unwatching_a_session_drops_its_pending_paths() {
        let mut src = FakeActivitySource::new();
        src.watch(SessionId(1), Path::new("/a")).unwrap();
        src.push(SessionId(1), "a.rs");
        src.unwatch(SessionId(1));
        assert!(src.drain().is_empty());
        assert!(src.watched().is_empty());
    }

    #[test]
    fn the_batch_is_capped_so_a_burst_cannot_grow_the_process() {
        let mut src = FakeActivitySource::new();
        src.watch(SessionId(1), Path::new("/a")).unwrap();
        for n in 0..ACTIVITY_PATHS_CAP + 100 {
            src.push(SessionId(1), &format!("dist/chunk-{n}.js"));
        }
        assert_eq!(src.drain().first().unwrap().paths.len(), ACTIVITY_PATHS_CAP);
    }

    #[test]
    fn a_real_watcher_reports_a_write_under_its_root_as_a_relative_path() {
        let dir = tempfile::tempdir().unwrap();
        // The OS reports the canonical path; a temp dir behind a symlink would otherwise fail
        // `strip_prefix` and yield nothing, which reads as a broken watcher and is not one.
        let root = dir.path().canonicalize().unwrap();
        std::fs::create_dir(root.join("src")).unwrap();
        let mut src = NotifyActivitySource::new().unwrap();
        src.watch(SessionId(1), &root).unwrap();
        std::fs::write(root.join("src/main.rs"), b"fn main() {}\n").unwrap();

        let mut seen = Vec::new();
        for _ in 0..40 {
            std::thread::sleep(std::time::Duration::from_millis(50));
            seen = src.drain();
            if !seen.is_empty() {
                break;
            }
        }
        let batch = seen
            .first()
            .expect("the watcher reported nothing within two seconds");
        assert_eq!(batch.session, SessionId(1));
        assert!(
            batch.paths.iter().any(|p| p == Path::new("src/main.rs")),
            "paths are relative to the watched root, so the filter can ask git about them; \
             got {:?}",
            batch.paths
        );
    }

    #[test]
    fn a_real_watcher_stops_reporting_after_unwatch() {
        // The production leak this guards: a session that ended keeps its OS watch, and its
        // paths keep arriving to be attributed to a session that no longer exists.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let mut src = NotifyActivitySource::new().unwrap();
        src.watch(SessionId(1), &root).unwrap();
        src.unwatch(SessionId(1));
        std::fs::write(root.join("after.rs"), b"//\n").unwrap();

        for _ in 0..10 {
            std::thread::sleep(std::time::Duration::from_millis(50));
            assert!(src.drain().is_empty(), "an unwatched root yields nothing");
        }
    }
}
