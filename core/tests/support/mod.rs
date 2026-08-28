#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::must_use_candidate,
    clippy::missing_panics_doc
)]
#![allow(dead_code)]
//! The fixture builder every git integration test in this plan shares.

use std::path::{Path, PathBuf};
use std::process::Command;

use codotheca_core::git::{ensure_empty_hooks_dir, GitExec, RepoHandle, StoreKey};
use codotheca_core::mount::StoreClass;

/// A throwaway repository built with real git plumbing.
///
/// Fixture git runs with its own `HOME` so the developer's global config never leaks in, and
/// with fixed identity and dates so history assertions are deterministic.
pub struct TestRepo {
    dir: tempfile::TempDir,
    root: PathBuf,
}

impl TestRepo {
    pub fn init() -> Self {
        let repo = Self::empty_dir();
        repo.git(&["init", "-q", "--initial-branch=main", "."]);
        repo
    }

    pub fn init_bare() -> Self {
        let repo = Self::empty_dir();
        repo.git(&["init", "-q", "--bare", "--initial-branch=main", "."]);
        repo
    }

    /// The working tree is a *subdirectory* of the temp dir, so the fake `HOME` and the empty
    /// hooks directory sit beside the repository rather than inside it — otherwise every
    /// untracked-count assertion would be counting the test harness.
    fn empty_dir() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(dir.path().join("home")).unwrap();
        Self { dir, root }
    }

    pub fn path(&self) -> &Path {
        &self.root
    }

    /// The temp directory the repository sits inside, for fixtures that need a sibling path.
    pub fn scratch(&self) -> &Path {
        self.dir.path()
    }

    pub fn handle(&self) -> RepoHandle {
        RepoHandle::resolve(&self.root, StoreKey::new("test-store"), StoreClass::Local).unwrap()
    }

    pub fn bare_handle(&self) -> RepoHandle {
        RepoHandle::bare(&self.root, StoreKey::new("test-store"), StoreClass::Local)
    }

    pub fn exec(&self) -> GitExec {
        let hooks = ensure_empty_hooks_dir(self.dir.path()).unwrap();
        GitExec::system(hooks)
    }

    pub fn write(&self, rel: &str, bytes: &[u8]) {
        let p = self.root.join(rel);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(p, bytes).unwrap();
    }

    pub fn git(&self, args: &[&str]) -> String {
        let out = Command::new("git")
            .current_dir(&self.root)
            .env("HOME", self.dir.path().join("home"))
            .env("XDG_CONFIG_HOME", self.dir.path().join("home"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("LC_ALL", "C")
            .env("GIT_AUTHOR_NAME", "Fixture")
            .env("GIT_AUTHOR_EMAIL", "fixture@example.invalid")
            .env("GIT_COMMITTER_NAME", "Fixture")
            .env("GIT_COMMITTER_EMAIL", "fixture@example.invalid")
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

    /// Like [`TestRepo::git`] but returns whether it succeeded, for the commands a fixture runs
    /// precisely because they are expected to fail — a merge that must conflict.
    pub fn try_git(&self, args: &[&str]) -> bool {
        Command::new("git")
            .current_dir(&self.root)
            .env("HOME", self.dir.path().join("home"))
            .env("XDG_CONFIG_HOME", self.dir.path().join("home"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("LC_ALL", "C")
            .env("GIT_AUTHOR_NAME", "Fixture")
            .env("GIT_AUTHOR_EMAIL", "fixture@example.invalid")
            .env("GIT_COMMITTER_NAME", "Fixture")
            .env("GIT_COMMITTER_EMAIL", "fixture@example.invalid")
            .args(["-c", "commit.gpgsign=false", "-c", "core.autocrlf=false"])
            .args(args)
            .output()
            .is_ok_and(|out| out.status.success())
    }

    pub fn commit(&self, message: &str) {
        self.commit_at(message, "2024-01-02T03:04:05+00:00");
    }

    pub fn commit_at(&self, message: &str, iso_date: &str) {
        self.git(&["add", "-A"]);
        let out = Command::new("git")
            .current_dir(&self.root)
            .env("HOME", self.dir.path().join("home"))
            .env("XDG_CONFIG_HOME", self.dir.path().join("home"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("LC_ALL", "C")
            .env("GIT_AUTHOR_NAME", "Fixture")
            .env("GIT_AUTHOR_EMAIL", "fixture@example.invalid")
            .env("GIT_COMMITTER_NAME", "Fixture")
            .env("GIT_COMMITTER_EMAIL", "fixture@example.invalid")
            .env("GIT_AUTHOR_DATE", iso_date)
            .env("GIT_COMMITTER_DATE", iso_date)
            .args([
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-q",
                "--allow-empty",
                "-m",
                message,
            ])
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}
