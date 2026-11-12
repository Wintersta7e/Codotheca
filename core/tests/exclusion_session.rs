#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! GA33: a live session's `check-ignore` asks the exclusion gate first. A copy excluded
//! mid-session spawns nothing for the next batch, remembers nothing from it, and the session stays
//! open; a removal resumes the reads at the batch after it.
//!
//! Driven through the production [`GitIgnoreCheck`] over a real repository. "Asked the gate before
//! spawning" is read from the gate's own record on every platform; on Unix every git spawn is also
//! counted by a `sh` shim on the executor's git path, which Windows cannot run.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use codotheca_core::exclusion::gate::{ExclusionGate, GateSite};
use codotheca_core::exclusion::list::{ExclusionList, PathEntry};
use codotheca_core::git::{GitExec, RepoHandle, RunLimits, StoreKey};
use codotheca_core::index::path::native_platform;
use codotheca_core::mount::StoreClass;
use codotheca_core::paths::{path_bytes, path_display, path_key};
use codotheca_core::protocol::{LocationId, ProjectId, SessionId, TargetId};
use codotheca_core::session::activity::{GitIgnoreCheck, ScopeFilter};
use codotheca_core::session::manager::{LaunchedSession, SessionManager};
use codotheca_core::session::segment::Signal;
use codotheca_core::session::watch::{ActivityBatch, ActivitySource, FakeActivitySource};
use codotheca_core::session::SessionError;
use codotheca_core::testing::events::ValidatingSink;
use codotheca_core::testing::zone::FixedZone;
use codotheca_core::testing::{FakeClock, TempIndex};

const T0: i64 = 1_700_000_000;

/// The manager owns its activity source, so the test keeps a handle on the same buffer.
#[derive(Debug, Clone, Default)]
struct SharedActivity(Arc<Mutex<FakeActivitySource>>);

impl ActivitySource for SharedActivity {
    fn watch(&mut self, session: SessionId, root: &Path) -> Result<(), SessionError> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .watch(session, root)
    }
    fn unwatch(&mut self, session: SessionId) {
        self.0.lock().unwrap().unwatch(session);
    }
    fn drain(&mut self) -> Vec<ActivityBatch> {
        self.0.lock().unwrap().drain()
    }
}

/// A real repository, a copy row naming it, and a git executor whose every spawn is logged.
struct World {
    _dir: tempfile::TempDir,
    repo_dir: PathBuf,
    /// The shim's spawn log; `None` where no shim runs.
    log: Option<PathBuf>,
    index: TempIndex,
    project: ProjectId,
    location: LocationId,
    target: TargetId,
    check: Arc<GitIgnoreCheck>,
    gate: Arc<ExclusionGate>,
}

fn run_git(dir: &Path, args: &[&str]) {
    let out = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {out:?}");
}

fn world() -> World {
    let dir = tempfile::tempdir().unwrap();
    let repo_dir = dir.path().join("zone").join("widget");
    std::fs::create_dir_all(repo_dir.join("src")).unwrap();
    run_git(&repo_dir, &["init", "-q"]);
    std::fs::write(repo_dir.join(".gitignore"), "dist/\n").unwrap();

    let (git, log) = git_under_test(dir.path());
    let hooks = dir.path().join("hooks");
    std::fs::create_dir_all(&hooks).unwrap();
    let check = Arc::new(GitIgnoreCheck {
        exec: Arc::new(GitExec::new(git, hooks)),
        limits: RunLimits::none(),
    });

    let index = TempIndex::new();
    let project = index.insert_project();
    let location = index.insert_location(project, repo_dir.to_str().unwrap());
    // The row keyed as the scanner keys it, so a path entry for the directory covers it on
    // every platform (Windows folds case).
    index
        .index()
        .conn()
        .execute(
            "UPDATE location SET path_bytes = ?1, path_key = ?2, path_display = ?3 WHERE id = ?4",
            rusqlite::params![
                path_bytes(&repo_dir),
                path_key(&repo_dir, native_platform()),
                path_display(&repo_dir),
                location.0
            ],
        )
        .unwrap();
    index
        .index()
        .conn()
        .execute(
            "INSERT INTO launch_target (project_id, location_id, kind, name, exec_bytes,
                                        sort_index)
             VALUES (?1, ?2, 'editor', 'an editor', X'2F65', 0)",
            rusqlite::params![project.0, location.0],
        )
        .unwrap();
    let target = TargetId(index.index().conn().last_insert_rowid());
    World {
        _dir: dir,
        repo_dir,
        log,
        index,
        project,
        location,
        target,
        check,
        gate: Arc::new(ExclusionGate::default()),
    }
}

/// On Unix a `sh` shim that logs every spawn, then runs git; elsewhere git itself, unlogged.
#[cfg(unix)]
fn git_under_test(dir: &Path) -> (PathBuf, Option<PathBuf>) {
    use std::os::unix::fs::PermissionsExt as _;
    let log = dir.join("spawns.log");
    let shim = dir.join("git-shim");
    std::fs::write(
        &shim,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\nexec git \"$@\"\n",
            log.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755)).unwrap();
    (shim, Some(log))
}

#[cfg(not(unix))]
fn git_under_test(_dir: &Path) -> (PathBuf, Option<PathBuf>) {
    let git =
        std::env::var_os("CODOTHECA_TEST_GIT").map_or_else(|| PathBuf::from("git"), PathBuf::from);
    (git, None)
}

impl World {
    /// Every git spawn so far; `None` where no shim counts them.
    fn spawns(&self) -> Option<usize> {
        self.log
            .as_ref()
            .map(|log| std::fs::read_to_string(log).map_or(0, |text| text.lines().count()))
    }

    /// The gate's session checks from position `from` on, as `admitted` flags.
    fn session_checks(&self, from: usize) -> Vec<bool> {
        self.gate.checks()[from..]
            .iter()
            .filter(|(site, _)| *site == GateSite::Session)
            .map(|(_, admitted)| *admitted)
            .collect()
    }

    fn cover(&self) {
        let _ = self.gate.swap(Arc::new(ExclusionList::new(
            &[],
            vec![PathEntry {
                distro: String::new(),
                platform: native_platform(),
                match_key: path_key(&self.repo_dir, native_platform()),
            }],
        )));
    }

    fn uncover(&self) {
        let _ = self.gate.swap(Arc::new(ExclusionList::default()));
    }

    fn repo(&self) -> RepoHandle {
        RepoHandle::resolve(&self.repo_dir, StoreKey::new("store"), StoreClass::Local).unwrap()
    }
}

#[test]
fn a_session_on_a_copy_excluded_mid_session_spawns_nothing_and_stays_open() {
    let mut w = world();
    let clock = Arc::new(FakeClock::new(T0));
    let activity = SharedActivity::default();
    let mut manager = SessionManager::new(
        clock.clone(),
        Arc::new(ValidatingSink::default()),
        Box::new(activity.clone()),
        w.check.clone(),
        Arc::new(FixedZone(0)),
        Arc::clone(&w.gate),
    );
    let repo = w.repo();
    let session = manager
        .launch(
            w.index.index_mut(),
            LaunchedSession {
                project_id: w.project,
                location_id: Some(w.location),
                target_id: Some(w.target),
                repo,
                waiter: None,
            },
        )
        .unwrap();

    // Each batch answers its spawn count (where a shim counts them) and the session checks the
    // gate recorded for it.
    let batch = |rel: &str, live: &mut SessionManager, rig: &mut World| {
        let (spawned, checked) = (rig.spawns(), rig.gate.checks().len());
        activity.0.lock().unwrap().push(session, rel);
        clock.advance(15);
        live.tick(rig.index.index_mut()).unwrap();
        (
            rig.spawns().zip(spawned).map(|(now, before)| now - before),
            rig.session_checks(checked),
        )
    };

    let first = batch("src/main.rs", &mut manager, &mut w);
    w.cover();
    let second = batch("src/other.rs", &mut manager, &mut w);
    w.uncover();
    let third = batch("src/other.rs", &mut manager, &mut w);
    eprintln!("per batch (spawns, session checks): allowed {first:?}, excluded {second:?}, removed {third:?}");

    // The gate was asked first: admitted, refused, admitted again after the removal.
    assert_eq!(first.1, [true], "the first batch's session check");
    assert_eq!(second.1, [false], "the covered batch was not refused");
    assert_eq!(
        third.1,
        [true],
        "the batch after the removal was not admitted"
    );
    assert_eq!(manager.live(), vec![session], "the session was ended");
    if let (Some(first), Some(second), Some(third)) = (first.0, second.0, third.0) {
        assert!(first >= 1, "the first batch asked git nothing");
        assert_eq!(second, 0, "a covered copy's batch spawned git");
        // Nothing was remembered from the covered batch, and the removal resumed the reads.
        assert!(third >= 1, "the batch after the removal asked git nothing");
    }
}

#[test]
fn the_scope_filter_folds_a_covered_batch_to_none_and_caches_nothing() {
    let w = world();
    let key =
        codotheca_core::exclusion::store::location_key(w.index.index().conn(), w.location).unwrap();
    let mut filter = ScopeFilter::new(w.check.clone(), Arc::clone(&w.gate), Some(key));
    let repo = w.repo();

    assert_eq!(
        filter.fold(&repo, &[PathBuf::from("src/main.rs")]),
        Signal::Worktree
    );
    assert_eq!(filter.cached(), 1);

    w.cover();
    let (before, checked) = (w.spawns(), w.gate.checks().len());
    assert_eq!(
        filter.fold(&repo, &[PathBuf::from("src/other.rs")]),
        Signal::None
    );
    // A remembered in-scope verdict is not an answer either while the copy is covered.
    assert_eq!(
        filter.fold(&repo, &[PathBuf::from("src/main.rs")]),
        Signal::None
    );
    assert_eq!(filter.cached(), 1, "a covered batch was cached");
    assert_eq!(
        w.session_checks(checked),
        [false, false],
        "the gate was not asked first"
    );
    if let (Some(now), Some(before)) = (w.spawns(), before) {
        assert_eq!(now - before, 0, "a covered batch spawned git");
    }
}
