#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

//! §39.4 — Ship's evidence: the user's own push record, read from remote-tracking reflogs by file
//! reads and persisted by J1's production path. Real git, real bare remotes in temporary
//! directories, a real migrated index.

mod support;

use std::process::Command;
use std::sync::{Arc, Mutex};

use codotheca_core::cancel::CancelToken;
use codotheca_core::clock::{Clock, SystemClock};
use codotheca_core::git::{GitSlots, PushRecord, SystemGit, PUSH_REFLOG_MESSAGE};
use codotheca_core::index::Index;
use codotheca_core::jobs::{run_one, JobDeps, JobKind, JobOrigin, JobOutcome, Priority};
use codotheca_core::mount::StoreClass;
use codotheca_core::protocol::{LocationId, ProjectId};
use support::TestRepo;

/// The pushes' committer date: a non-zero offset, so a reader that dropped the line's own frame
/// for UTC or the app's zone would store a different offset.
const PUSH_DATE: &str = "1790000000 +0530";

/// The persisted triple: `(push_record, last_push_at, last_push_tz_offset_min)`.
type Triple = (Option<String>, Option<i64>, Option<i64>);

/// [`TestRepo::git`] with a fixed committer date, which the push's reflog line carries.
fn git_dated(repo: &TestRepo, args: &[&str]) {
    let home = repo.scratch().join("home");
    let out = Command::new("git")
        .current_dir(repo.path())
        .env("HOME", &home)
        .env("XDG_CONFIG_HOME", &home)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("LC_ALL", "C")
        .env("GIT_AUTHOR_NAME", "Fixture")
        .env("GIT_AUTHOR_EMAIL", "fixture@example.invalid")
        .env("GIT_COMMITTER_NAME", "Fixture")
        .env("GIT_COMMITTER_EMAIL", "fixture@example.invalid")
        .env("GIT_COMMITTER_DATE", PUSH_DATE)
        .args(["-c", "commit.gpgsign=false", "-c", "core.autocrlf=false"])
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "fixture git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn commit(repo: &TestRepo, name: &str) {
    repo.write(name, name.as_bytes());
    repo.git(&["add", name]);
    repo.git(&["commit", "-q", "-m", name]);
}

/// A working copy with one commit and `origin` pointing at `remote`. No upstream is configured,
/// so J1's divergence count spawns nothing and the reading is file reads alone.
fn with_origin(remote: &TestRepo) -> TestRepo {
    let repo = TestRepo::init();
    commit(&repo, "a.txt");
    repo.git(&["remote", "add", "origin", &remote.path().to_string_lossy()]);
    repo
}

/// A copy that pushed once, with [`PUSH_DATE`] as the committer date.
fn pushed(remote: &TestRepo) -> TestRepo {
    let repo = with_origin(remote);
    git_dated(&repo, &["push", "-q", "origin", "main"]);
    repo
}

fn tracking_reflog(repo: &TestRepo) -> std::path::PathBuf {
    repo.path()
        .join(".git")
        .join("logs")
        .join("refs")
        .join("remotes")
        .join("origin")
        .join("main")
}

/// Every fixture §39.4's table names, each as `(name, repository, expected triple)`. The bare
/// remotes ride along so their directories outlive the reading.
fn fixtures() -> Vec<(&'static str, TestRepo, Triple, Vec<TestRepo>)> {
    let read = |at: Option<i64>, offset: Option<i64>| (Some("read".to_owned()), at, offset);
    let only = |record: &str| (Some(record.to_owned()), None, None);
    let mut out = Vec::new();

    out.push({
        let remote = TestRepo::init_bare();
        let repo = pushed(&remote);
        (
            "push",
            repo,
            read(Some(1_790_000_000), Some(330)),
            vec![remote],
        )
    });

    // A second clone pushes; this one only ever fetches, so its tracking ref moves by fetch lines.
    out.push({
        let remote = TestRepo::init_bare();
        let pusher = pushed(&remote);
        let fetcher = TestRepo::init();
        fetcher.git(&["remote", "add", "origin", &remote.path().to_string_lossy()]);
        fetcher.git(&["fetch", "-q", "origin"]);
        commit(&pusher, "b.txt");
        git_dated(&pusher, &["push", "-q", "origin", "main"]);
        fetcher.git(&["fetch", "-q", "origin"]);
        let fetch_lines = std::fs::read_to_string(tracking_reflog(&fetcher)).unwrap();
        assert_eq!(
            fetch_lines.lines().count(),
            2,
            "two fetches, two lines: {fetch_lines}"
        );
        (
            "fetch only",
            fetcher,
            read(None, None),
            vec![remote, pusher],
        )
    });

    out.push({
        let remote = TestRepo::init_bare();
        let repo = pushed(&remote);
        repo.git(&["config", "core.logAllRefUpdates", "false"]);
        ("logAllRefUpdates=false", repo, only("absent"), vec![remote])
    });

    out.push({
        let remote = TestRepo::init_bare();
        let repo = pushed(&remote);
        let logs = repo
            .path()
            .join(".git")
            .join("logs")
            .join("refs")
            .join("remotes");
        std::fs::remove_dir_all(logs).unwrap();
        ("no tracking reflog", repo, only("absent"), vec![remote])
    });

    out.push({
        let remote = TestRepo::init_bare();
        let repo = pushed(&remote);
        let config = repo.path().join(".git").join("config");
        let mut text = std::fs::read_to_string(&config).unwrap();
        text.push_str("[extensions]\n\trefStorage = reftable\n");
        std::fs::write(&config, text).unwrap();
        (
            "refStorage=reftable",
            repo,
            only("unsupported"),
            vec![remote],
        )
    });

    out.push({
        let repo = TestRepo::init();
        commit(&repo, "a.txt");
        ("no remote", repo, only("no_remote"), Vec::new())
    });

    out.push({
        let remote = TestRepo::init_bare();
        let repo = pushed(&remote);
        let reflog = tracking_reflog(&repo);
        std::fs::remove_file(&reflog).unwrap();
        std::fs::create_dir(&reflog).unwrap();
        (
            "reflog is a directory",
            repo,
            only("unreadable"),
            vec![remote],
        )
    });

    out
}

/// One migrated index, one project and location per fixture.
struct Sweep {
    _dir: tempfile::TempDir,
    index: Mutex<Index>,
}

impl Sweep {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let index = Index::open(&dir.path().join("index")).unwrap();
        Self {
            _dir: dir,
            index: Mutex::new(index),
        }
    }

    fn file(&self, repo: &TestRepo) -> (ProjectId, LocationId) {
        let path = repo.path().to_string_lossy().into_owned();
        let guard = self.index.lock().unwrap();
        let conn = guard.conn();
        conn.execute(
            "INSERT INTO project (name, seed_basename, created_at, updated_at, authored_by_user)
             VALUES ('p', 'p', 0, 0, 1)",
            [],
        )
        .unwrap();
        let project = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO location (project_id, kind, path_bytes, path_key, path_display,
                                   store_key, presence, repo_kind)
             VALUES (?1, 'linux', ?2, ?2, ?3, 'store', 'present', 'worktree')",
            rusqlite::params![project, path.as_bytes(), path],
        )
        .unwrap();
        let location = conn.last_insert_rowid();
        drop(guard);
        (ProjectId(project), LocationId(location))
    }

    /// J1 through `run_one`, the dispatch `JobRunner` itself calls, over the production git seam.
    fn j1(&self, repo: &TestRepo) -> Triple {
        let (project, location) = self.file(repo);
        let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
        let deps = JobDeps {
            git: Arc::new(SystemGit::new(
                Arc::new(repo.exec()),
                Arc::new(GitSlots::new(4)),
                Arc::clone(&clock),
            )),
            clock,
            cancel: CancelToken::new(),
            zone: Arc::new(codotheca_core::testing::zone::FixedZone(0)),
        };
        let outcome = run_one(
            &self.index,
            &deps,
            &codotheca_core::jobs::Job {
                kind: JobKind::J1Refstate,
                project_id: project,
                location_id: location,
                store_key: "store".to_owned(),
                store_kind: StoreClass::Local,
                priority: Priority::Standard,
                not_before: 0,
                origin: JobOrigin::Walk,
            },
            &std::cell::RefCell::new(Vec::new()),
        )
        .unwrap();
        assert_eq!(outcome, JobOutcome::Done);
        self.index
            .lock()
            .unwrap()
            .conn()
            .query_row(
                "SELECT push_record, last_push_at, last_push_tz_offset_min FROM location
                 WHERE id = ?1",
                [location.0],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap()
    }
}

fn git_version() -> String {
    let out = Command::new("git").arg("--version").output().unwrap();
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

/// **AC-P4-39-5.** Each row of §39.4's record table, one fixture each, persisted by J1's
/// production path. The push fixture's stored stamp is the line's own — a non-zero offset — and
/// the fetch-advanced tracking ref reads `read` with no push: a fetch is never evidence of one.
#[test]
fn ac_p4_39_5_push_evidence_through_j1() {
    eprintln!("{}", git_version());
    let sweep = Sweep::new();
    let fixtures = fixtures();
    for (name, repo, expected, _remotes) in &fixtures {
        let stored = sweep.j1(repo);
        eprintln!("{name:<24} {stored:?}");
        assert_eq!(&stored, expected, "fixture `{name}`");
    }
    // The literal is pinned against the git this run uses: the push fixture's reflog holds it.
    let pushed = std::fs::read_to_string(tracking_reflog(&fixtures[0].1)).unwrap();
    assert!(
        pushed
            .lines()
            .any(|line| line.split_once('\t').map(|(_, m)| m) == Some(PUSH_REFLOG_MESSAGE)),
        "this git wrote no `{PUSH_REFLOG_MESSAGE}` line: {pushed}"
    );
    assert_eq!(fixtures.len(), 7);
}

/// Every slug the reader can produce is one the column's CHECK accepts.
#[test]
fn every_push_record_slug_is_accepted_by_the_column() {
    let sweep = Sweep::new();
    let repo = TestRepo::init();
    let (_, location) = sweep.file(&repo);
    let guard = sweep.index.lock().unwrap();
    let mut accepted = 0;
    for record in PushRecord::ALL {
        guard
            .conn()
            .execute(
                "UPDATE location SET push_record = ?2 WHERE id = ?1",
                rusqlite::params![location.0, record.as_str()],
            )
            .unwrap_or_else(|e| panic!("`{}` refused: {e}", record.as_str()));
        accepted += 1;
    }
    let foreign = guard.conn().execute(
        "UPDATE location SET push_record = 'pushed' WHERE id = ?1",
        [location.0],
    );
    drop(guard);
    eprintln!("push_record slugs accepted: {accepted}");
    assert_eq!(accepted, PushRecord::ALL.len());
    assert!(foreign.is_err(), "a slug outside the set was accepted");
}

/// §39.12: after the sweep, a push time only under `read`, and the two push columns NULL
/// together, over every row J1 wrote.
#[test]
fn the_push_columns_hold_their_invariants_over_every_row() {
    let sweep = Sweep::new();
    let fixtures = fixtures();
    for (_, repo, _, _) in &fixtures {
        sweep.j1(repo);
    }
    let guard = sweep.index.lock().unwrap();
    let conn = guard.conn();
    let rows: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM location WHERE push_record IS NOT NULL",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let broken: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM location
             WHERE (last_push_at IS NOT NULL AND push_record IS NOT 'read')
                OR ((last_push_at IS NULL) <> (last_push_tz_offset_min IS NULL))",
            [],
            |row| row.get(0),
        )
        .unwrap();
    drop(guard);
    eprintln!("rows checked: {rows}, broken: {broken}");
    assert!(rows > 0, "the sweep wrote no push record");
    assert_eq!(rows, i64::try_from(fixtures.len()).unwrap());
    assert_eq!(broken, 0);
}
