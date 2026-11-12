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
    ensure_empty_hooks_dir, establish_ownership, translate_wsl_pointers_for_tests, GitBackend,
    GitDirSelect, GitError, GitExec, GitResult, GitSlots, JobClass, JobContext, RepoHandle,
    RunLimits, StatusOptions, StoreKey, SystemGit,
};
use codotheca_core::mount::StoreClass;
use support::scan_rig::{scan, scan_with};
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

    /// Rewrite the pointer as WSL spells it, for a host whose translation is the test switch's:
    /// `/mnt/q/<absolute path>`, which git here cannot follow and the core translates back.
    fn respell_for_this_host(&self) {
        let wsl = format!("/mnt/q{}", self.pointer());
        std::fs::write(self.lane.join(".git"), format!("gitdir: {wsl}\n")).unwrap();
        assert!(
            !self.repo.try_git(&["-C", &forward(&self.lane), "status"]),
            "the fixture must reproduce the defect: git refuses the WSL pointer"
        );
    }

    /// git behind a recorder whose owner can be switched: every argv is logged, and once
    /// `foreign` is called git sees every repository as someone else's.
    fn wrapped(&self) -> Wrapped {
        let dir = self.repo.scratch().join("wrapped-git");
        std::fs::create_dir_all(&dir).unwrap();
        let log = dir.join("argv.log");
        let flag = dir.join("foreign");
        let script = dir.join("git");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{log}'\n\
                 if [ -e '{flag}' ]; then {DIFFERENT_OWNER}=1; export {DIFFERENT_OWNER}; fi\n\
                 exec '{git}' \"$@\"\n",
                log = log.display(),
                flag = flag.display(),
                git = support::test_git().display(),
            ),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let hooks = ensure_empty_hooks_dir(&dir).unwrap();
        Wrapped {
            exec: Arc::new(GitExec::new(script, hooks)),
            log,
            flag,
        }
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

/// `handle` through the one establishing function, as every production site takes it.
fn established(git: &dyn GitBackend, handle: RepoHandle) -> GitResult<RepoHandle> {
    let cancel = CancelToken::new();
    let ctx = JobContext::new(
        JobClass::Interactive,
        &cancel,
        Some(Duration::from_secs(30)),
    );
    establish_ownership(git, handle, &ctx)
}

/// Real git behind a recorder ([`World::wrapped`]).
struct Wrapped {
    exec: Arc<GitExec>,
    log: PathBuf,
    flag: PathBuf,
}

impl Wrapped {
    fn exec(&self) -> GitExec {
        (*self.exec).clone()
    }

    fn git(&self) -> SystemGit {
        SystemGit::new(
            Arc::clone(&self.exec),
            Arc::new(GitSlots::new(4)),
            Arc::new(SystemClock::new()),
        )
    }

    /// Every invocation since the last `clear`, one argv per line.
    fn calls(&self) -> Vec<String> {
        std::fs::read_to_string(&self.log)
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    fn clear(&self) {
        std::fs::write(&self.log, b"").unwrap();
    }

    /// Whether any invocation since the last `clear` named the repository explicitly.
    fn explicit(&self) -> bool {
        self.calls().iter().any(|argv| argv.contains("--git-dir="))
    }

    /// From now on every repository is someone else's: the owner changed after the scan.
    fn foreign(&self) {
        std::fs::write(&self.flag, b"").unwrap();
        self.clear();
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
    assert!(w.status_ok(&established(&w.git(), resolved.clone()).unwrap()));
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

/// The hand-off: a WSL-spelled pointer is read by naming it only once the probes in the hand-off's
/// own resolution pass. An owner change after one scan is caught by the next: one `untrusted_repo`
/// row, nothing indexed, no explicit invocation and no filter run.
#[cfg(unix)]
#[test]
fn the_hand_off_establishes_ownership_in_the_resolution_that_reads() {
    translate_wsl_pointers_for_tests();
    let w = world();
    w.respell_for_this_host();
    let git = w.wrapped();

    let owned = scan_with(git.exec(), &w.root());
    assert_eq!(owned.problem_rows, 0, "{:?}", owned.problems);
    assert_eq!(owned.indexed, 1);
    assert_eq!(owned.locations[0].1, "linked_worktree");
    assert!(
        git.explicit(),
        "an owned translated pointer is read by naming it"
    );

    git.foreign();
    w.touch("c\n");
    let _ = std::fs::remove_file(&w.marker);
    let foreign = scan_with(git.exec(), &w.root());
    assert_eq!(foreign.problem_rows, 1, "{:?}", foreign.problems);
    assert_eq!(foreign.problems[0].0, "untrusted_repo");
    assert_eq!(foreign.indexed, 0);
    assert_ne!(
        git.calls(),
        Vec::<String>::new(),
        "the foreign scan ran git"
    );
    assert!(!git.explicit(), "{:?}", git.calls());
    assert!(!w.marker.exists(), "no foreign filter ran");
}

/// S1c: a translated pointer whose target is not there is unreadable, not untrusted — trusting it
/// would not make it readable, so the row must not offer that.
#[cfg(unix)]
#[test]
fn a_wsl_pointer_to_nothing_is_unreadable_not_untrusted() {
    translate_wsl_pointers_for_tests();
    let w = world();
    let gone = format!("/mnt/q{}/gone/.git/worktrees/lane-a", forward(&w.root()));
    std::fs::write(w.lane.join(".git"), format!("gitdir: {gone}\n")).unwrap();
    let scanned = scan(&w.repo, &w.root());
    assert_eq!(scanned.problem_rows, 1, "{:?}", scanned.problems);
    assert_eq!(scanned.problems[0].0, "unreadable_repo");
    assert_eq!(scanned.indexed, 0);
}

/// Jobs: every run establishes ownership for itself — the row the scan wrote stands in for
/// nothing. An owner change after the scan fails the job as `Untrusted`, with no explicit
/// invocation and no filter run, and the scheduler files it `failed` without a retry.
#[cfg(unix)]
#[test]
fn a_job_establishes_ownership_each_run_and_fails_untrusted_without_retry() {
    use codotheca_core::jobs::scheduler::JobRunner;
    use codotheca_core::jobs::{
        run_one, Job, JobDeps, JobError, JobKind, JobOrigin, JobSink as _, Priority,
    };
    use codotheca_core::protocol::{LocationId, ProjectId};

    translate_wsl_pointers_for_tests();
    let w = world();
    w.respell_for_this_host();
    let git = w.wrapped();
    let scanned = scan_with(git.exec(), &w.root());
    assert_eq!(scanned.indexed, 1, "{:?}", scanned.problems);
    let index = Arc::clone(&scanned.index);
    let (project, location): (i64, i64) = index
        .lock()
        .unwrap()
        .conn()
        .query_row("SELECT project_id, id FROM location", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap();
    let set_trusted = |at: Option<i64>| {
        index
            .lock()
            .unwrap()
            .conn()
            .execute("UPDATE location SET trusted_at = ?1", [at])
            .unwrap();
    };
    let deps = JobDeps {
        exclusions: Arc::new(codotheca_core::exclusion::gate::ExclusionGate::default()),
        git: Arc::new(git.git()),
        clock: Arc::new(SystemClock::new()),
        cancel: CancelToken::new(),
        zone: Arc::new(codotheca_core::testing::zone::FixedZone(0)),
    };
    let job = Job {
        kind: JobKind::J2Status,
        project_id: ProjectId(project),
        location_id: LocationId(location),
        store_key: "store-a".to_owned(),
        store_kind: StoreClass::Local,
        priority: Priority::Interactive,
        not_before: 0,
        origin: JobOrigin::Walk,
    };
    let run = || run_one(&index, &deps, &job, &std::cell::RefCell::new(Vec::new()));

    // The measured cost: the probe pair, on translated untrusted locations only.
    git.clear();
    run().unwrap();
    let untrusted_owned = git.calls().len();
    assert!(git.explicit());
    set_trusted(Some(1));
    git.clear();
    run().unwrap();
    let trusted = git.calls().len();
    eprintln!("J2 spawns: untrusted+owned {untrusted_owned}, trusted {trusted}");

    set_trusted(None);
    git.foreign();
    w.touch("d\n");
    let _ = std::fs::remove_file(&w.marker);
    let err = run().unwrap_err();
    assert!(
        matches!(err, JobError::Git(GitError::Untrusted { .. })),
        "{err:?}"
    );
    assert!(!git.explicit(), "{:?}", git.calls());
    assert!(!w.marker.exists(), "no foreign filter ran");

    // The retry policy, end to end: J1 fails hard and is not requeued.
    let events = Arc::new(codotheca_core::testing::ScanEventFake::default());
    let runner = JobRunner::new(Arc::clone(&index), deps.clone(), events);
    runner.on_location_indexed(
        ProjectId(project),
        LocationId(location),
        "store-a",
        StoreClass::Local,
    );
    runner.start(1);
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    while !runner.is_idle() {
        assert!(
            std::time::Instant::now() < deadline,
            "the runner never settled"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    runner.request_stop();
    runner.join();
    let states: Vec<(String, String, i64)> = index
        .lock()
        .unwrap()
        .conn()
        .prepare("SELECT job, state, fail_count FROM project_job_state ORDER BY job")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    let j1 = states.iter().find(|(j, ..)| j == "j1").expect("a j1 row");
    assert_eq!((j1.1.as_str(), j1.2), ("failed", 1), "{states:?}");
    assert!(!git.explicit(), "{:?}", git.calls());
    assert_eq!(untrusted_owned, trusted + 2, "the probe pair, and only it");
}

/// The analyser and a launch session's `check-ignore`: each establishes ownership where it reads.
#[cfg(unix)]
#[test]
fn the_analyser_and_a_session_establish_ownership_where_they_read() {
    use codotheca_core::analyser::{resolve, LocationRow};
    use codotheca_core::protocol::LocationId;
    use codotheca_core::session::activity::{GitIgnoreCheck, IgnoreCheck as _};

    translate_wsl_pointers_for_tests();
    let w = world();
    w.respell_for_this_host();
    let git = w.wrapped();
    let row = LocationRow {
        id: LocationId(1),
        path: w.lane.clone(),
        lineage_key: None,
        removed_at: None,
        refstate_observed_at: None,
        worktree_observed_at: None,
        store: StoreKey::new("store-a"),
        trusted: false,
        scan_roots: Vec::new(),
        live_session: false,
        others: Vec::new(),
    };
    let check = GitIgnoreCheck {
        exec: Arc::clone(&git.exec),
        limits: RunLimits::after(Duration::from_secs(30)),
    };
    let session =
        RepoHandle::resolve(&w.lane, StoreKey::new("store-a"), StoreClass::Local).unwrap();
    assert_eq!(session.git_dir_select, GitDirSelect::Translated);
    let rel = [PathBuf::from("a.txt")];
    let cancel = CancelToken::new();
    let ctx = JobContext::new(
        JobClass::Interactive,
        &cancel,
        Some(Duration::from_secs(30)),
    );

    let analysed = resolve(&row, &git.git(), &ctx).expect("an owned location resolves");
    assert_eq!(analysed.git_dir_select, GitDirSelect::Established);
    check.ignored(&session, &rel).unwrap();
    assert!(git.explicit());

    git.foreign();
    assert!(resolve(&row, &git.git(), &ctx).is_none());
    let refused = check.ignored(&session, &rel).unwrap_err();
    assert!(matches!(refused, GitError::Untrusted { .. }), "{refused:?}");
    assert_ne!(git.calls(), Vec::<String>::new());
    assert!(!git.explicit(), "{:?}", git.calls());
}
