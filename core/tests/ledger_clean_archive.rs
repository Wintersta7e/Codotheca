//! §38.8.5's `clean_archive`: an archived project pays once, ever, when every copy was last
//! observed covered by a remote and clean, and its debt was last observed swept and empty.
//!
//! J1 runs as the job arm itself (`run_one`), so `location.archive_reached` is written by the real
//! reachability read; the evaluator is then reached through a real settle — a J2, the leaf whose
//! own observation of the worktree the evaluator also reads. Every project is archived through
//! `projects.setFlags`, the path a project archived before phase 4 took.

#![cfg(feature = "testkit")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use std::cell::RefCell;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use codotheca_core::art::testsupport::CollectingSink;
use codotheca_core::cancel::CancelToken;
use codotheca_core::git::{GitBackend, GitError, GitSlots, RefFingerprint, RefState, SystemGit};
use codotheca_core::index::Index;
use codotheca_core::jobs::scheduler::JobRunner;
use codotheca_core::jobs::{
    run_one, Job, JobDeps, JobKind, JobOrigin, JobOutcome, NullJobSink, Priority,
};
use codotheca_core::mount::StoreClass;
use codotheca_core::projects::{dispatch_projects_command, ProjectsCtx};
use codotheca_core::protocol::{LocationId, ProjectId};
use codotheca_core::testing::events::ValidatingSink;
use codotheca_core::testing::zone::FixedZone;
use codotheca_core::testing::{
    FakeClock, FakeGitBackend, FakeMountResolver, GitReply, RecordingGitBackend,
};
use serde_json::json;
use support::TestRepo;

const T0: i64 = 1_800_000_000;

/// One project on one present copy of its own repository, and the bare remote it pushes to.
struct Project {
    repo: TestRepo,
    _remote: TestRepo,
    id: ProjectId,
    location: LocationId,
}

/// A `clean_archive` row as stored: its receipt and the subject it was keyed on.
#[derive(Debug, PartialEq, Eq)]
struct Paid {
    what: String,
    when: String,
    observed_at: i64,
    dedupe_key: String,
}

struct Rig {
    _dir: tempfile::TempDir,
    index: Arc<Mutex<Index>>,
}

impl Rig {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let index = Index::open_at(dir.path(), T0).unwrap();
        Self {
            _dir: dir,
            index: Arc::new(Mutex::new(index)),
        }
    }

    fn sql(&self, sql: &str, params: impl rusqlite::Params) {
        let guard = self.index.lock().unwrap();
        guard.conn().execute(sql, params).unwrap();
    }

    /// The happy path's project: one commit, tagged and pushed to a bare remote, a clean tree, no
    /// debt item, one `complete` sweep observed at `T0`, authored by the user and not Reference.
    /// Not yet archived and not yet observed by J1 or J2. `pushed: false` leaves the remote empty.
    ///
    /// The tag is load-bearing: a settle that reads `tag_count = 0` opens a scored `no_release`
    /// item, which withholds the award on its own.
    fn project(&self, name: &str, pushed: bool) -> Project {
        let repo = TestRepo::init();
        repo.write("a.txt", b"one\n");
        repo.commit("first");
        repo.git(&["tag", "v1.0.0"]);
        let remote = TestRepo::init_bare();
        if pushed {
            let url = remote.path().to_string_lossy().into_owned();
            repo.git(&["remote", "add", "origin", &url]);
            repo.git(&["push", "-q", "origin", "main", "v1.0.0"]);
        }
        let path = repo.path().to_string_lossy().into_owned();
        let (id, location) = {
            let guard = self.index.lock().unwrap();
            let conn = guard.conn();
            conn.execute(
                "INSERT INTO project (name, seed_basename, authored_by_user, is_reference,
                                      created_at, updated_at)
                 VALUES (?1, ?1, 1, 0, 0, 0)",
                [name],
            )
            .unwrap();
            let id = ProjectId(conn.last_insert_rowid());
            conn.execute(
                "INSERT INTO location (project_id, kind, path_bytes, path_key, path_display,
                                       store_key, presence, repo_kind)
                 VALUES (?1, 'linux', ?2, ?2, ?3, 'store', 'present', 'worktree')",
                rusqlite::params![id.0, path.as_bytes(), path],
            )
            .unwrap();
            let location = LocationId(conn.last_insert_rowid());
            conn.execute(
                "INSERT INTO debt_sweep (project_id, source, outcome, item_count, observed_at)
                 VALUES (?1, 'todo_marker', 'complete', 0, ?2)",
                rusqlite::params![id.0, T0],
            )
            .unwrap();
            drop(guard);
            (id, location)
        };
        Project {
            repo,
            _remote: remote,
            id,
            location,
        }
    }

    /// `projects.setFlags` with `isArchived`, as the product's archive control sends it.
    fn archive(&self, p: &Project, archived: bool) {
        let guard = self.index.lock().unwrap();
        let sink = CollectingSink::default();
        let mounts = FakeMountResolver::default();
        let ctx = ProjectsCtx {
            index: &guard,
            events: &sink,
            jobs: &NullJobSink,
            mounts: &mounts,
            sync: &codotheca_core::sync::runner::NullSyncSink,
            now: T0,
            tz_offset_min: 0,
        };
        dispatch_projects_command(
            &ctx,
            "projects.setFlags",
            json!({ "id": p.id.0, "isArchived": archived }),
        )
        .expect("owned")
        .expect("ok");
        drop(guard);
    }

    fn deps(git: Arc<dyn GitBackend>, clock: Arc<FakeClock>) -> JobDeps {
        JobDeps {
            git,
            clock,
            cancel: CancelToken::new(),
            zone: Arc::new(FixedZone(0)),
        }
    }

    fn system_git(p: &Project, clock: &Arc<FakeClock>) -> SystemGit {
        SystemGit::new(
            Arc::new(p.repo.exec()),
            Arc::new(GitSlots::new(4)),
            clock.clone(),
        )
    }

    fn job(p: &Project, kind: JobKind) -> Job {
        Job {
            kind,
            project_id: p.id,
            location_id: p.location,
            store_key: "store".to_owned(),
            store_kind: StoreClass::Local,
            priority: Priority::Interactive,
            not_before: 0,
            origin: JobOrigin::Interactive,
        }
    }

    /// J1's arm, run as the scheduler runs it, through `git`.
    fn j1_with(&self, p: &Project, git: Arc<dyn GitBackend>, at: i64) {
        let deps = Self::deps(git, Arc::new(FakeClock::new(at)));
        run_one(
            &self.index,
            &deps,
            &Self::job(p, JobKind::J1Refstate),
            &RefCell::new(Vec::new()),
        )
        .unwrap();
    }

    /// J1's arm through native git.
    fn j1(&self, p: &Project, at: i64) {
        let clock = Arc::new(FakeClock::new(at));
        self.j1_with(p, Arc::new(Self::system_git(p, &clock)), at);
    }

    /// J7's arm through native git: the content scan, whose sweep is the debt reading §38.8.5
    /// rests on.
    fn j7(&self, p: &Project, at: i64) {
        let clock = Arc::new(FakeClock::new(at));
        let deps = Self::deps(Arc::new(Self::system_git(p, &clock)), clock);
        let outcome = run_one(
            &self.index,
            &deps,
            &Self::job(p, JobKind::J7Markers),
            &RefCell::new(Vec::new()),
        )
        .unwrap();
        assert_eq!(outcome, JobOutcome::Done);
    }

    /// One J2 — a leaf, so nothing chains a J1.5 that would rewrite the fixture — on a fresh
    /// runner whose clock reads `at`, spun until idle. Its settle runs the evaluator.
    fn settle(&self, p: &Project, at: i64) {
        let clock = Arc::new(FakeClock::new(at));
        let deps = Self::deps(Arc::new(Self::system_git(p, &clock)), clock);
        let runner = JobRunner::new(
            Arc::clone(&self.index),
            deps,
            Arc::new(ValidatingSink::default()),
        );
        assert!(runner.enqueue(Self::job(p, JobKind::J2Status)));
        runner.start(1);
        let deadline = Instant::now() + Duration::from_secs(30);
        while Instant::now() < deadline && !runner.is_idle() {
            std::thread::sleep(Duration::from_millis(25));
        }
        let idle = runner.is_idle();
        runner.request_stop();
        runner.join();
        assert!(idle, "J2 did not settle for project {}", p.id.0);
    }

    /// J1, then the settle after it.
    fn observe(&self, p: &Project, at: i64) {
        self.j1(p, at);
        self.settle(p, at + 1);
    }

    fn paid(&self, project: ProjectId) -> Vec<Paid> {
        let guard = self.index.lock().unwrap();
        let mut st = guard
            .conn()
            .prepare(
                "SELECT json_extract(meta, '$.what'), json_extract(meta, '$.when'),
                        json_extract(meta, '$.observed_at'), dedupe_key
                   FROM xp_events WHERE project_id = ?1 AND kind = 'clean_archive' ORDER BY id",
            )
            .unwrap();
        let rows = st
            .query_map([project.0], |r| {
                Ok(Paid {
                    what: r.get(0)?,
                    when: r.get(1)?,
                    observed_at: r.get(2)?,
                    dedupe_key: r.get(3)?,
                })
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        drop(st);
        drop(guard);
        rows
    }

    /// The project's `complete` sweeps and its content-scan sweep, as `source=outcome`, sorted by
    /// source.
    fn sweeps(&self, project: ProjectId) -> Vec<String> {
        let guard = self.index.lock().unwrap();
        let mut st = guard
            .conn()
            .prepare(
                "SELECT source || '=' || outcome FROM debt_sweep
                  WHERE project_id = ?1 AND (outcome = 'complete' OR source = 'todo_marker')
                  ORDER BY source",
            )
            .unwrap();
        let rows = st
            .query_map([project.0], |r| r.get(0))
            .unwrap()
            .collect::<Result<Vec<String>, _>>()
            .unwrap();
        drop(st);
        drop(guard);
        rows
    }

    /// §29.8's grant, as the setting stores it: `'1'` on, absent off.
    fn grant(&self, on: bool) {
        if on {
            self.sql(
                "INSERT INTO app_meta (k, v) VALUES ('content_scan_enabled', '1')
                 ON CONFLICT(k) DO UPDATE SET v = excluded.v",
                [],
            );
        } else {
            self.sql("DELETE FROM app_meta WHERE k = 'content_scan_enabled'", []);
        }
    }

    /// Acknowledge the project, which enrols it.
    fn enrol(&self, p: &Project) {
        self.sql(
            "UPDATE project SET acknowledged_at = ?2 WHERE id = ?1",
            rusqlite::params![p.id.0, T0],
        );
    }

    fn count(&self, project: ProjectId) -> usize {
        self.paid(project).len()
    }

    fn archive_reached(&self, location: LocationId) -> Option<i64> {
        let guard = self.index.lock().unwrap();
        guard
            .conn()
            .query_row(
                "SELECT archive_reached FROM location WHERE id = ?1",
                [location.0],
                |r| r.get(0),
            )
            .unwrap()
    }
}

/// Archived, observed, paid once — and a NULL `archive_reached` pays nothing: the copy was last
/// read by a J1 while the project was live, so every other observation already qualifies.
fn pays_once_at_the_settle_after_its_j1(rig: &Rig) {
    let p = rig.project("kept", true);
    rig.observe(&p, T0 + 10);
    rig.archive(&p, true);
    rig.settle(&p, T0 + 15);
    assert_eq!(rig.archive_reached(p.location), None);
    assert_eq!(rig.count(p.id), 0, "a NULL archive_reached pays nothing");

    rig.j1(&p, T0 + 20);
    assert_eq!(rig.archive_reached(p.location), Some(1));
    rig.settle(&p, T0 + 30);
    let paid = rig.paid(p.id);
    assert_eq!(paid.len(), 1);
    let row = &paid[0];
    assert_eq!(row.what, "clean_archive");
    assert_eq!(
        row.when, "2027-01-15",
        "the receipt's date is the paying settle's"
    );
    // The oldest observation it read: the sweep at T0, older than J1's and J2's.
    assert_eq!(row.observed_at, T0);
    assert!(
        row.dedupe_key.starts_with("clean_archive:"),
        "{}",
        row.dedupe_key
    );

    rig.observe(&p, T0 + 40);
    assert_eq!(rig.count(p.id), 1, "a later settle pays nothing more");
}

/// Un-archive and re-archive: one row, ever.
fn rearchiving_writes_one_row(rig: &Rig) {
    let p = rig.project("again", true);
    rig.archive(&p, true);
    rig.observe(&p, T0 + 100);
    assert_eq!(rig.count(p.id), 1);
    rig.archive(&p, false);
    rig.observe(&p, T0 + 110);
    assert_eq!(
        rig.archive_reached(p.location),
        None,
        "a live project reads NULL"
    );
    rig.archive(&p, true);
    rig.observe(&p, T0 + 120);
    assert_eq!(rig.count(p.id), 1);
}

/// One condition withholds the row; lifting that condition alone pays it.
fn withheld_until(
    rig: &Rig,
    name: &str,
    spoil: impl Fn(&Rig, &Project),
    lift: impl Fn(&Rig, &Project),
) {
    let p = rig.project(name, true);
    spoil(rig, &p);
    rig.archive(&p, true);
    rig.observe(&p, T0 + 200);
    assert_eq!(rig.count(p.id), 0, "{name}: withheld");
    lift(rig, &p);
    rig.observe(&p, T0 + 210);
    assert_eq!(
        rig.count(p.id),
        1,
        "{name}: paid once the condition is lifted"
    );
}

fn a_local_only_branch_withholds(rig: &Rig) {
    withheld_until(
        rig,
        "branch",
        |_, p| {
            p.repo.git(&["checkout", "-q", "-b", "side"]);
            p.repo.write("b.txt", b"local\n");
            p.repo.commit("local only");
            p.repo.git(&["checkout", "-q", "main"]);
        },
        |_, p| {
            p.repo.git(&["push", "-q", "origin", "side"]);
        },
    );
}

fn a_detached_head_on_a_local_only_commit_withholds(rig: &Rig) {
    withheld_until(
        rig,
        "detached",
        |_, p| {
            p.repo.git(&["checkout", "-q", "--detach"]);
            p.repo.write("b.txt", b"local\n");
            p.repo.commit("detached only");
        },
        |_, p| {
            p.repo
                .git(&["push", "-q", "origin", "HEAD:refs/heads/kept"]);
        },
    );
}

fn a_dirty_tree_withholds(rig: &Rig) {
    withheld_until(
        rig,
        "dirty",
        |_, p| p.repo.write("a.txt", b"edited\n"),
        |_, p| {
            p.repo.git(&["checkout", "-q", "--", "a.txt"]);
        },
    );
}

fn an_untracked_file_withholds(rig: &Rig) {
    withheld_until(
        rig,
        "untracked",
        |_, p| p.repo.write("loose.txt", b"loose\n"),
        |_, p| std::fs::remove_file(p.repo.path().join("loose.txt")).unwrap(),
    );
}

fn a_stash_withholds(rig: &Rig) {
    withheld_until(
        rig,
        "stash",
        |_, p| {
            p.repo.write("a.txt", b"stashed\n");
            p.repo.git(&["stash", "-q"]);
        },
        |_, p| {
            p.repo.git(&["stash", "drop", "-q"]);
        },
    );
}

fn an_unverified_scored_item_withholds(rig: &Rig) {
    withheld_until(
        rig,
        "unverified",
        |rig, p| {
            rig.sql(
                "INSERT INTO debt_item (project_id, subject_key, source, fingerprint, state,
                                        scoring, first_seen_at, last_seen_at)
                 VALUES (?1, 'k', 'todo_marker', 'f', 'unverified', 'scored', ?2, ?2)",
                rusqlite::params![p.id.0, T0],
            );
        },
        |rig, p| {
            rig.sql("DELETE FROM debt_item WHERE project_id = ?1", [p.id.0]);
        },
    );
}

/// A project with no sweep yet, pushed, whose tree answers every presence source a scan reads —
/// a README, a licence and a test — so a granted scan opens no item. The test is a source blob,
/// so the tree has something to content-scan.
fn unswept(rig: &Rig, name: &str) -> Project {
    let p = rig.project(name, true);
    rig.sql("DELETE FROM debt_sweep WHERE project_id = ?1", [p.id.0]);
    p.repo.write("README.md", b"# kept\n");
    p.repo.write("LICENSE", b"kept\n");
    p.repo.write("kept_test.rs", b"fn kept() {}\n");
    p.repo.commit("present");
    p.repo.git(&["push", "-q", "origin", "main"]);
    p
}

/// A settle sweeps the singleton sources itself before the evaluator runs, and on a copy J1 has
/// read it completes them. Without the grant those sweeps are the debt reading, so an archived
/// copy no scan has read pays once on them: no award depends on granting consent.
fn ungranted_pays_on_the_settles_sweeps(rig: &Rig) {
    rig.grant(false);
    let p = unswept(rig, "ungranted");
    rig.enrol(&p);
    rig.archive(&p, true);
    rig.observe(&p, T0 + 200);
    assert_eq!(
        rig.sweeps(p.id),
        ["abandoned_with_debt=complete", "no_release=complete"]
    );
    assert_eq!(rig.count(p.id), 1, "paid on the settle's own sweeps");
}

/// The grant is the user's for a project only once it is enrolled: an unenrolled one pays as an
/// ungranted one does.
fn unenrolled_pays_on_the_settles_sweeps(rig: &Rig) {
    rig.grant(true);
    let p = unswept(rig, "unenrolled");
    rig.archive(&p, true);
    rig.observe(&p, T0 + 250);
    assert_eq!(
        rig.sweeps(p.id),
        ["abandoned_with_debt=complete", "no_release=complete"]
    );
    assert_eq!(rig.count(p.id), 1, "paid on the settle's own sweeps");
}

/// Granted and enrolled, the scan is the debt reading. J7 reads no blob of an archived project, so
/// a copy archived before its first content scan pays nothing while it stays so.
fn granted_and_never_scanned_pays_nothing(rig: &Rig) {
    rig.grant(true);
    let p = unswept(rig, "unscanned");
    rig.enrol(&p);
    rig.archive(&p, true);
    rig.observe(&p, T0 + 300);
    assert_eq!(
        rig.count(p.id),
        0,
        "singleton sweeps are not the scan's reading"
    );
    rig.j7(&p, T0 + 310);
    rig.settle(&p, T0 + 311);
    assert_eq!(
        rig.count(p.id),
        0,
        "an archived copy's blobs are never read"
    );
}

/// Granted and enrolled, a scan complete at the copy's HEAD before the Seal is still the reading
/// after a J7 has run over the archived copy, and the receipt dates it by the scan's completion.
fn granted_and_scanned_before_the_seal_pays_once(rig: &Rig) {
    rig.grant(true);
    let p = unswept(rig, "scanned");
    rig.enrol(&p);
    rig.observe(&p, T0 + 400);
    rig.j7(&p, T0 + 410);
    assert_eq!(rig.sweeps(p.id).last().unwrap(), "todo_marker=complete");
    // An older sweep of a source nothing here sweeps again: not the scan's reading, so the
    // receipt is not dated by it.
    rig.sql(
        "INSERT INTO debt_sweep (project_id, source, outcome, item_count, observed_at)
         VALUES (?1, 'dependency_advisory', 'complete', 0, ?2)",
        rusqlite::params![p.id.0, T0],
    );
    rig.archive(&p, true);
    rig.j7(&p, T0 + 420);
    rig.observe(&p, T0 + 430);
    let paid = rig.paid(p.id);
    assert_eq!(paid.len(), 1, "paid at the settle after the Seal");
    assert_eq!(
        paid[0].observed_at,
        T0 + 410,
        "dated by the scan's completion"
    );
    rig.observe(&p, T0 + 440);
    assert_eq!(rig.count(p.id), 1);
}

/// Granted and enrolled, a scan of an older tree is not a reading of the current one: a commit
/// after the scan withholds the award while the archived copy's blobs go unread — though its
/// `todo_marker` sweep still says `complete` — and a scan of the new HEAD pays it.
fn granted_and_scanned_before_a_new_commit_withholds(rig: &Rig) {
    rig.grant(true);
    let p = unswept(rig, "moved");
    rig.enrol(&p);
    rig.observe(&p, T0 + 500);
    rig.j7(&p, T0 + 510);
    p.repo.write("later_test.rs", b"fn later() {}\n");
    p.repo.commit("later");
    p.repo.git(&["push", "-q", "origin", "main"]);
    rig.archive(&p, true);
    rig.observe(&p, T0 + 520);
    rig.j7(&p, T0 + 530);
    rig.settle(&p, T0 + 531);
    assert_eq!(rig.sweeps(p.id).last().unwrap(), "todo_marker=complete");
    assert_eq!(rig.count(p.id), 0, "the scan read a tree the HEAD has left");

    rig.archive(&p, false);
    rig.j7(&p, T0 + 540);
    rig.archive(&p, true);
    rig.observe(&p, T0 + 550);
    assert_eq!(rig.count(p.id), 1, "paid once the new HEAD is scanned");
}

/// No remote: nothing covers the local tips, so the read answers `0`.
fn a_repository_with_no_remote_pays_nothing(rig: &Rig) {
    let p = rig.project("private", false);
    rig.archive(&p, true);
    rig.observe(&p, T0 + 300);
    assert_eq!(rig.archive_reached(p.location), Some(0));
    assert_eq!(rig.count(p.id), 0);
}

#[test]
fn ac_p4_38_22_clean_archive() {
    let rig = Rig::new();
    pays_once_at_the_settle_after_its_j1(&rig);
    rearchiving_writes_one_row(&rig);
    a_local_only_branch_withholds(&rig);
    a_detached_head_on_a_local_only_commit_withholds(&rig);
    a_dirty_tree_withholds(&rig);
    an_untracked_file_withholds(&rig);
    a_stash_withholds(&rig);
    an_unverified_scored_item_withholds(&rig);
    ungranted_pays_on_the_settles_sweeps(&rig);
    unenrolled_pays_on_the_settles_sweeps(&rig);
    granted_and_never_scanned_pays_nothing(&rig);
    granted_and_scanned_before_the_seal_pays_once(&rig);
    granted_and_scanned_before_a_new_commit_withholds(&rig);
    a_repository_with_no_remote_pays_nothing(&rig);
}

/// A fake answering J1's two reads, and `archive_reach` with `reach`.
fn fake_git(reach: GitReply<bool>, at: i64) -> Arc<dyn GitBackend> {
    let clock = Arc::new(FakeClock::new(at));
    let git = FakeGitBackend::with_clock(Arc::clone(&clock));
    git.always_ref_state(GitReply::Ok(RefState {
        head_oid: Some("a".repeat(40)),
        branch: Some("main".to_owned()),
        upstream: None,
        ahead: None,
        behind: None,
        tag_count: 0,
        stash_count: Some(0),
        is_shallow: false,
        is_bare: false,
        interrupted_op: None,
        fetch_head_at: None,
        reflog_tail_at: None,
        basis: RefFingerprint::from_hex(&"0".repeat(64)).unwrap(),
        observed_at: at,
    }));
    git.always_divergence(GitReply::Ok(None));
    git.always_archive_reach(reach);
    Arc::new(git)
}

#[test]
fn archive_reached_is_null_unless_archived_and_read() {
    let rig = Rig::new();

    // A live project's copy reads NULL, and the read is never made for it.
    let live = rig.project("live", true);
    let clock = Arc::new(FakeClock::new(T0));
    let recording = Arc::new(RecordingGitBackend::new(Rig::system_git(&live, &clock)));
    rig.j1_with(&live, recording.clone(), T0);
    assert_eq!(rig.archive_reached(live.location), None);
    let ops: Vec<&str> = recording.calls().iter().map(|c| c.op).collect();
    assert!(!ops.contains(&"archive_reach"), "{ops:?}");

    // An archived copy whose read fails reads NULL — over a stored 1, never 1.
    let archived = rig.project("archived", true);
    rig.archive(&archived, true);
    rig.j1_with(&archived, fake_git(GitReply::Ok(true), T0 + 1), T0 + 1);
    assert_eq!(rig.archive_reached(archived.location), Some(1));
    rig.j1_with(
        &archived,
        fake_git(GitReply::Err(GitError::Missing), T0 + 2),
        T0 + 2,
    );
    assert_eq!(rig.archive_reached(archived.location), None);
}

/// J1's reach reads for `p`, run through native git behind a recorder.
fn reach_reads(rig: &Rig, p: &Project, at: i64) -> usize {
    let clock = Arc::new(FakeClock::new(at));
    let recording = Arc::new(RecordingGitBackend::new(Rig::system_git(p, &clock)));
    rig.j1_with(p, recording.clone(), at);
    let reads = recording
        .calls()
        .iter()
        .filter(|c| c.op == "archive_reach")
        .count();
    eprintln!("project {}: {reads} reach read(s) at {at}", p.id.0);
    reads
}

/// §38.8.5's award is paid once, ever, so J1 reads reachability only while it is unpaid: an
/// archived copy is read until its project holds the row, and never after.
#[test]
fn a_paid_clean_archive_spawns_no_reach_read() {
    let rig = Rig::new();
    let p = rig.project("paid", true);
    rig.archive(&p, true);
    assert_eq!(
        reach_reads(&rig, &p, T0 + 10),
        1,
        "an unpaid archive is read"
    );
    rig.settle(&p, T0 + 15);
    assert_eq!(rig.count(p.id), 1, "the fixture pays first");

    assert_eq!(
        reach_reads(&rig, &p, T0 + 20),
        0,
        "a paid archive spawned a reach read"
    );
    rig.settle(&p, T0 + 25);
    assert_eq!(rig.count(p.id), 1);
}
