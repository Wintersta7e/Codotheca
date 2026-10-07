//! The scan's hand-off on real repositories at paths a test chooses, for the removal tests: a
//! copy is indexed, removed the way `locations.uninstall` removes it, and something is found at
//! its path again.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};

use codotheca_core::assembly::handoff::{hand_off_discovered, HandoffCtx, Indexed};
use codotheca_core::cancel::CancelToken;
use codotheca_core::clock::SystemClock;
use codotheca_core::git::{GitBackend, GitExec, GitSlots, RepoHandle, StoreKey, SystemGit};
use codotheca_core::index::Index;
use codotheca_core::mount::StoreClass;
use codotheca_core::paths::{path_bytes, path_display, path_key};
use codotheca_core::protocol::LocationId;
use codotheca_core::scan::discover::{RepoCandidate, RepoKind};
use codotheca_core::scan::run::{platform_of, Discovered};

pub(crate) const NOW: i64 = 1_760_000_000;
const GENERATION: i64 = 7;

/// A neutral git, so the machine's own config cannot change what a fixture is. Author, committer
/// and dates are fixed, so a commit's message alone decides its id.
pub(crate) fn git_at(cwd: &Path, home: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .current_dir(cwd)
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", home)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("LC_ALL", "C")
        .env("GIT_AUTHOR_NAME", "Fixture")
        .env("GIT_AUTHOR_EMAIL", "fixture@example.invalid")
        .env("GIT_AUTHOR_DATE", "2024-01-02T03:04:05+00:00")
        .env("GIT_COMMITTER_NAME", "Fixture")
        .env("GIT_COMMITTER_EMAIL", "fixture@example.invalid")
        .env("GIT_COMMITTER_DATE", "2024-01-02T03:04:05+00:00")
        .args(["-c", "commit.gpgsign=false", "-c", "core.autocrlf=false"])
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "fixture git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

pub(crate) struct Rig {
    pub dir: tempfile::TempDir,
    pub index: Arc<Mutex<Index>>,
    git: Arc<dyn GitBackend>,
    cancel: CancelToken,
}

impl Rig {
    pub(crate) fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("home")).unwrap();
        let index = Arc::new(Mutex::new(Index::open(&dir.path().join("index")).unwrap()));
        let hooks = codotheca_core::git::ensure_empty_hooks_dir(&dir.path().join("hooks")).unwrap();
        let git: Arc<dyn GitBackend> = Arc::new(SystemGit::new(
            Arc::new(GitExec::system(hooks)),
            Arc::new(GitSlots::for_machine()),
            Arc::new(SystemClock::new()),
        ));
        Self {
            dir,
            index,
            git,
            cancel: CancelToken::new(),
        }
    }

    fn home(&self) -> PathBuf {
        self.dir.path().join("home")
    }

    /// `<tmp>/<name>`, whatever is there.
    pub(crate) fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    /// A repository at `<tmp>/<name>` holding one commit with `message`. Two messages are two
    /// root commits, so two lineages; one message is one lineage.
    pub(crate) fn repo(&self, name: &str, message: &str) -> PathBuf {
        let path = self.empty_repo(name);
        std::fs::write(path.join("a.txt"), b"one\n").unwrap();
        git_at(&path, &self.home(), &["add", "-A"]);
        git_at(&path, &self.home(), &["commit", "-m", message]);
        path
    }

    /// A repository at `<tmp>/<name>` that has never been committed to: it has no lineage.
    pub(crate) fn empty_repo(&self, name: &str) -> PathBuf {
        let path = self.path(name);
        std::fs::create_dir_all(&path).unwrap();
        git_at(&path, &self.home(), &["init", "-b", "main", "."]);
        path
    }

    /// The walk finds `path` and hands it on.
    pub(crate) fn hand_off(&self, path: &Path) -> Indexed {
        let ctx = HandoffCtx {
            git: self.git.as_ref(),
            cancel: &self.cancel,
            store_class: StoreClass::Local,
            generation: GENERATION,
            now: NOW,
        };
        hand_off_discovered(&self.index, &ctx, &discovered_at(path)).unwrap()
    }

    /// `locations.uninstall`'s own row write for `location`, then the bytes leave the disk.
    pub(crate) fn uninstall(&self, location: LocationId, path: &Path) {
        self.remove_row(location);
        std::fs::remove_dir_all(path).unwrap();
    }

    /// `locations.uninstall`'s own row write for `location`, and nothing on disk.
    pub(crate) fn remove_row(&self, location: LocationId) {
        let mut guard = self.index.lock().unwrap();
        guard
            .with_tx(|tx| {
                codotheca_core::uninstall::command::commit_removal(tx, location, NOW).unwrap();
                Ok(())
            })
            .unwrap();
        drop(guard);
    }

    /// One read against the index, under its lock.
    pub(crate) fn read<T>(&self, f: impl FnOnce(&rusqlite::Connection) -> T) -> T {
        let guard = self.index.lock().unwrap();
        let out = f(guard.conn());
        drop(guard);
        out
    }

    /// One write against the index, under its lock.
    pub(crate) fn execute(&self, sql: &str, params: impl rusqlite::Params) {
        self.read(|conn| conn.execute(sql, params).unwrap());
    }

    pub(crate) fn count(&self, table: &str) -> i64 {
        self.read(|conn| {
            conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
                .unwrap()
        })
    }
}

/// What the walk hands on, built from what `classify` would have resolved.
fn discovered_at(path: &Path) -> Discovered {
    let handle = RepoHandle::resolve(path, StoreKey::new("store-a"), StoreClass::Local).unwrap();
    Discovered {
        candidate: RepoCandidate {
            path: path.to_path_buf(),
            kind: RepoKind::WorkTree,
            git_dir: handle.git_dir.clone(),
            common_dir: handle.common_dir,
        },
        root_id: 1,
        kind: "linux".to_owned(),
        distro: String::new(),
        path_bytes: path_bytes(path),
        path_key: path_key(path, platform_of("linux")),
        path_display: path_display(path),
        store_key: "store-a".to_owned(),
        volume_key: Some("vol-a".to_owned()),
    }
}
