//! Compiled only under `testkit`: these link `codotheca_core::testing`, which the feature gates.
#![cfg(feature = "testkit")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
// The scan half is Windows behaviour; off Windows its helpers still compile and are linted.
#![cfg_attr(not(windows), allow(dead_code, unused_imports))]

//! A linked worktree created from WSL records `gitdir: /mnt/<d>/…`, which Windows git refuses.
//! On Windows the core translates the pointer and names the repository with `--git-dir` and
//! `--work-tree` — but only once ownership is established, because explicit selection skips the
//! ownership check git runs on discovery, and that check is what keeps a foreign-owned
//! repository's config (a `filter.<x>.clean`, run by `status`) from running.

mod support;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use codotheca_core::cancel::CancelToken;
use codotheca_core::clock::SystemClock;
use codotheca_core::git::{
    GitBackend, GitDirSelect, GitSlots, JobClass, JobContext, RepoHandle, StatusOptions, StoreKey,
    SystemGit,
};
use codotheca_core::mount::StoreClass;
use support::scan_rig::scan;
use support::TestRepo;

/// git's own switch for its ownership check: every repository reads as someone else's. Honoured
/// by git's discovery path; the product never sets it.
const DIFFERENT_OWNER: &str = "GIT_TEST_ASSUME_DIFFERENT_OWNER";

/// A committed repository whose `*.txt` runs a clean filter that leaves a marker behind, and a
/// real linked worktree of it, `lane-a`, in a directory of its own.
struct World {
    repo: TestRepo,
    lanes: tempfile::TempDir,
    lane: PathBuf,
    marker: PathBuf,
}

fn forward(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn world() -> World {
    let repo = TestRepo::init();
    repo.write(".gitattributes", b"*.txt filter=probe\n");
    repo.write("a.txt", b"a\n");
    repo.commit("first");
    let marker = support::canonical(repo.scratch()).join("filter-ran");
    repo.git(&[
        "config",
        "filter.probe.clean",
        &format!("echo ran >> '{}' && cat", forward(&marker)),
    ]);
    let lanes = tempfile::tempdir().unwrap();
    let lane = support::canonical(lanes.path()).join("lane-a");
    repo.git(&["worktree", "add", "-q", "-b", "lane-a", &forward(&lane)]);
    World {
        repo,
        lanes,
        lane,
        marker,
    }
}

impl World {
    fn root(&self) -> PathBuf {
        support::canonical(self.lanes.path())
    }

    /// The worktree's own git dir, as git wrote it into the `.git` file.
    fn pointer(&self) -> String {
        let text = std::fs::read_to_string(self.lane.join(".git")).unwrap();
        text.trim()
            .strip_prefix("gitdir:")
            .unwrap()
            .trim()
            .to_owned()
    }

    /// Rewrite the pointer the way a worktree created from WSL spells it.
    fn respell_as_wsl(&self) {
        let pointer = self.pointer().replace('\\', "/");
        let (drive, rest) = pointer.split_once(":/").expect("a drive-lettered pointer");
        let wsl = format!("/mnt/{}/{rest}", drive.to_ascii_lowercase());
        std::fs::write(self.lane.join(".git"), format!("gitdir: {wsl}\n")).unwrap();
    }

    /// Change a filtered file so the next `status` hashes it through the clean filter.
    fn touch(&self, text: &str) {
        std::fs::write(self.lane.join("a.txt"), text).unwrap();
    }

    fn git(&self) -> SystemGit {
        SystemGit::new(
            Arc::new(self.repo.exec()),
            Arc::new(GitSlots::new(4)),
            Arc::new(SystemClock::new()),
        )
    }

    fn status_ok(&self, handle: &RepoHandle) -> bool {
        let cancel = CancelToken::new();
        let ctx = JobContext::new(
            JobClass::Interactive,
            &cancel,
            Some(Duration::from_secs(30)),
        );
        self.git()
            .worktree_status(handle, StatusOptions::full(), &ctx)
            .is_ok()
    }

    fn handle(&self, git_dir: PathBuf, select: GitDirSelect, trusted: bool) -> RepoHandle {
        RepoHandle {
            work_dir: self.lane.clone(),
            common_dir: support::canonical(self.repo.path()).join(".git"),
            git_dir,
            store: StoreKey::new("store-a"),
            store_class: StoreClass::Local,
            trusted,
            git_dir_select: select,
        }
    }
}

/// `base_args`' explicit selection against real git, on every host: a worktree whose `.git` file
/// git cannot follow reads once the handle names its directories — trusted, or with ownership
/// established — and a translated handle with neither fails closed.
#[test]
fn explicit_selection_reads_a_worktree_its_gitdir_file_cannot_reach() {
    let w = world();
    let git_dir = PathBuf::from(w.pointer());
    std::fs::write(
        w.lane.join(".git"),
        b"gitdir: /nowhere/.git/worktrees/lane-a\n",
    )
    .unwrap();

    assert!(!w.status_ok(&w.handle(git_dir.clone(), GitDirSelect::Discover, false)));
    assert!(!w.status_ok(&w.handle(git_dir.clone(), GitDirSelect::Translated, false)));
    assert!(w.status_ok(&w.handle(git_dir.clone(), GitDirSelect::Translated, true)));
    assert!(w.status_ok(&w.handle(git_dir, GitDirSelect::Established, false)));
}

/// The user's case end to end on Windows: a real linked worktree, its pointer respelled
/// `/mnt/<d>/…`. Owned, it indexes as the linked worktree it is and reports nothing. Owned by
/// someone else, it is ONE `untrusted_repo` row, nothing indexes, and no filter runs.
///
/// One test, not two: the different-owner half sets a process-wide variable that would turn
/// every other repository in this binary into someone else's.
#[cfg(windows)]
#[test]
fn a_wsl_spelled_pointer_indexes_when_owned_and_is_untrusted_when_not() {
    let w = world();
    w.respell_as_wsl();
    assert!(
        !w.repo.try_git(&["-C", &forward(&w.lane), "status"]),
        "the fixture must reproduce the defect: Windows git refuses the WSL pointer"
    );

    let owned = scan(&w.repo, &w.root());
    assert_eq!(owned.problem_rows, 0, "{:?}", owned.problems);
    assert_eq!(owned.indexed, 1);
    assert_eq!(owned.locations.len(), 1, "{:?}", owned.locations);
    assert!(owned.locations[0].0.contains("lane-a"));
    assert_eq!(owned.locations[0].1, "linked_worktree");

    // What a job reads for that location: explicit selection reaches it, and the filter it
    // configures is real — which is exactly why ownership has to be established first.
    let resolved =
        RepoHandle::resolve(&w.lane, StoreKey::new("store-a"), StoreClass::Local).unwrap();
    assert_eq!(resolved.git_dir_select, GitDirSelect::Translated);
    w.touch("b\n");
    assert!(w.status_ok(&resolved.clone().ownership_established()));
    assert!(w.marker.exists(), "an owned repository's filter runs");
    std::fs::remove_file(&w.marker).unwrap();

    std::env::set_var(DIFFERENT_OWNER, "1");
    assert!(
        !w.repo.try_git(&["status"]),
        "git must honour {DIFFERENT_OWNER} for this half to mean anything"
    );
    w.touch("c\n");
    let foreign = scan(&w.repo, &w.root());
    let untranslated_status = w.status_ok(&resolved);
    std::env::remove_var(DIFFERENT_OWNER);

    assert_eq!(foreign.problem_rows, 1, "{:?}", foreign.problems);
    assert_eq!(foreign.problems.len(), 1);
    assert_eq!(foreign.problems[0].0, "untrusted_repo");
    assert!(foreign.problems[0].1.contains("lane-a"));
    assert_eq!(foreign.indexed, 0);
    assert!(foreign.locations.is_empty(), "{:?}", foreign.locations);
    assert!(
        !untranslated_status,
        "an untrusted translated handle fails closed"
    );
    assert!(!w.marker.exists(), "no foreign filter ran");
}
