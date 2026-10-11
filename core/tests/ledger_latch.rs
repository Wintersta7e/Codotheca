//! §38.8.2's completion latch, driven through `JobRunner`'s settle and never by calling the latch:
//! a check that passes while the gate is open pays once per project, whatever its row does next.
//!
//! Every project here passes or fails exactly one check, `remote`, by toggling
//! `project.remote_key` under an observed refstate; its other nine read `fail` or `unknown`.

#![cfg(feature = "testkit")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use codotheca_core::cancel::CancelToken;
use codotheca_core::clock::LocalZone as _;
use codotheca_core::git::{GitSlots, SystemGit};
use codotheca_core::identity::merge::merge_projects;
use codotheca_core::identity::AssociationKind;
use codotheca_core::index::subject::subject_for_project;
use codotheca_core::index::Index;
use codotheca_core::jobs::scheduler::JobRunner;
use codotheca_core::jobs::{Job, JobDeps, JobKind, JobOrigin, Priority};
use codotheca_core::mount::StoreClass;
use codotheca_core::proto::EventSink;
use codotheca_core::protocol::{CompletionCheck, LocationId, ProjectId};
use codotheca_core::testing::zone::FixedZone;
use codotheca_core::testing::FakeClock;
use serde_json::{json, Value};
use support::TestRepo;

const T0: i64 = 1_800_000_000;

struct Quiet;

impl EventSink for Quiet {
    fn emit(&self, topic: &str, event: &str, payload: Value) {
        codotheca_core::testing::events::validated(topic, event, &payload);
    }
}

/// One project on one present copy of its own repository.
struct Project {
    repo: TestRepo,
    id: ProjectId,
    location: LocationId,
}

/// A latch row as stored: what it pays for, its receipt date, and the subject it was keyed on.
#[derive(Debug, PartialEq, Eq)]
struct Latch {
    what: String,
    when: String,
    subject_key: String,
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

    /// Authored by the user, not Reference, refstate observed, `commit_days` gate rows, and the
    /// `remote` check passing when `remote` is set.
    fn project(&self, name: &str, commit_days: i64, remote: bool) -> Project {
        let repo = TestRepo::init();
        repo.write("a.txt", b"one\n");
        repo.commit("first");
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
                                       store_key, presence, repo_kind, refstate_observed_at)
                 VALUES (?1, 'linux', ?2, ?2, ?3, 'store', 'present', 'worktree', ?4)",
                rusqlite::params![id.0, path.as_bytes(), path, T0],
            )
            .unwrap();
            let location = LocationId(conn.last_insert_rowid());
            drop(guard);
            (id, location)
        };
        let p = Project { repo, id, location };
        for day in 0..commit_days {
            self.commit_day(&p, day);
        }
        self.remote(&p, remote);
        p
    }

    fn commit_day(&self, p: &Project, day: i64) {
        self.sql(
            "INSERT INTO xp_events (ts, tz_offset_min, project_id, subject_key, kind, dedupe_key,
                                    track)
             VALUES (?1, 0, ?2, 'gate', 'commit_day', ?3, 'git')",
            rusqlite::params![
                T0 - 86_400 * (day + 1),
                p.id.0,
                format!("cd:{}:{day}", p.id.0)
            ],
        );
    }

    fn remote(&self, p: &Project, on: bool) {
        let key = on.then(|| format!("remote-{}", p.id.0));
        self.sql(
            "UPDATE project SET remote_key = ?2 WHERE id = ?1",
            rusqlite::params![p.id.0, key],
        );
    }

    /// One J2 — a leaf, so nothing chains a J1.5 or a J4 that would rewrite the fixture — on a
    /// fresh runner whose clock reads `at`, spun until idle.
    fn settle(&self, p: &Project, at: i64) {
        let clock = Arc::new(FakeClock::new(at));
        let deps = JobDeps {
            git: Arc::new(SystemGit::new(
                Arc::new(p.repo.exec()),
                Arc::new(GitSlots::new(4)),
                clock.clone(),
            )),
            clock,
            cancel: CancelToken::new(),
            zone: Arc::new(FixedZone(0)),
        };
        let runner = JobRunner::new(Arc::clone(&self.index), deps, Arc::new(Quiet));
        assert!(runner.enqueue(Job {
            kind: JobKind::J2Status,
            project_id: p.id,
            location_id: p.location,
            store_key: "store".to_owned(),
            store_kind: StoreClass::Local,
            priority: Priority::Interactive,
            not_before: 0,
            origin: JobOrigin::Interactive,
        }));
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

    fn set_na(&self, p: &Project, na: Option<bool>, at: i64) {
        let mut guard = self.index.lock().unwrap();
        guard
            .with_tx(|tx| {
                codotheca_core::completion::set_check_na(
                    tx,
                    p.id,
                    CompletionCheck::Remote,
                    na,
                    FixedZone(0).at(at),
                )
            })
            .unwrap();
    }

    /// Every row `sql` answers for `project`, under a lock held for the read alone.
    fn select<T>(
        &self,
        sql: &str,
        project: ProjectId,
        row: impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
    ) -> Vec<T> {
        let guard = self.index.lock().unwrap();
        let mut st = guard.conn().prepare(sql).unwrap();
        let out = st
            .query_map([project.0], row)
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        drop(st);
        drop(guard);
        out
    }

    fn latches(&self, project: ProjectId) -> Vec<Latch> {
        self.select(
            "SELECT json_extract(meta, '$.what'), json_extract(meta, '$.when'), subject_key
               FROM xp_events WHERE project_id = ?1 AND kind = 'completion_check'
              ORDER BY id",
            project,
            |r| {
                Ok(Latch {
                    what: r.get(0)?,
                    when: r.get(1)?,
                    subject_key: r.get(2)?,
                })
            },
        )
    }

    fn whats(&self, project: ProjectId) -> Vec<String> {
        self.latches(project).into_iter().map(|l| l.what).collect()
    }

    /// `(check_key, state, observed_at)` for each stored row, in key order.
    fn rows(&self, project: ProjectId) -> Vec<(String, String, i64)> {
        self.select(
            "SELECT check_key, state, observed_at FROM project_check
              WHERE project_id = ?1 ORDER BY check_key",
            project,
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
    }

    fn state_of(&self, project: ProjectId, check: &str) -> String {
        self.rows(project)
            .into_iter()
            .find(|(key, ..)| key == check)
            .map_or_else(
                || panic!("no {check} row for project {}", project.0),
                |(_, state, _)| state,
            )
    }
}

/// fail → pass → fail → pass writes one row, and only a `pass` row latches: the `fail` and
/// `unknown` rows beside it write nothing.
fn a_flapping_check_pays_once(rig: &Rig) {
    let p = rig.project("flap", 3, false);
    rig.settle(&p, T0);
    assert_eq!(rig.state_of(p.id, "remote"), "fail");
    assert_eq!(
        rig.whats(p.id),
        Vec::<String>::new(),
        "a failing check latched"
    );
    for (step, on) in [true, false, true].into_iter().enumerate() {
        rig.remote(&p, on);
        rig.settle(&p, T0 + 10 * (i64::try_from(step).unwrap() + 1));
    }
    assert_eq!(rig.state_of(p.id, "remote"), "pass");
    assert_eq!(rig.whats(p.id), ["remote"], "fail → pass → fail → pass");
    assert_eq!(
        rig.latches(p.id)[0].when,
        "2027-01-15",
        "the receipt's date is the latching settle's"
    );

    let rows = rig.rows(p.id);
    let passing: Vec<&str> = rows
        .iter()
        .filter(|(_, state, _)| state == "pass")
        .map(|(key, ..)| key.as_str())
        .collect();
    let unknown = rows
        .iter()
        .filter(|(_, state, _)| state == "unknown")
        .count();
    eprintln!("ledger_latch: flap rows pass={passing:?} unknown={unknown}");
    assert_eq!(passing, ["remote"]);
    assert!(unknown > 0, "the fixture holds no unknown row: {rows:?}");
}

/// A third commit-day opens the gate under a check already passing: the recompute it runs
/// rewrites nothing, and the latch is written all the same.
fn a_third_commit_day_latches_on_an_unchanged_recompute(rig: &Rig) {
    let p = rig.project("third", 2, true);
    rig.settle(&p, T0 + 100);
    assert_eq!(rig.state_of(p.id, "remote"), "pass");
    assert_eq!(
        rig.whats(p.id),
        Vec::<String>::new(),
        "two commit-days latched"
    );
    let before = rig.rows(p.id);

    rig.commit_day(&p, 2);
    rig.settle(&p, T0 + 200);
    assert_eq!(
        rig.rows(p.id),
        before,
        "the second recompute rewrote its rows, so it did not exercise the unchanged return"
    );
    assert_eq!(
        rig.whats(p.id),
        ["remote"],
        "the unchanged recompute latched nothing"
    );
}

/// A row the user ruled N/A latches nothing; clearing the ruling recomputes and latches in
/// `set_check_na`'s own call, with no settle after it.
fn a_ruling_latches_through_its_own_call(rig: &Rig) {
    let p = rig.project("ruled", 3, false);
    rig.settle(&p, T0 + 300);
    rig.set_na(&p, Some(true), T0 + 310);
    rig.remote(&p, true);
    rig.settle(&p, T0 + 320);
    assert_eq!(rig.state_of(p.id, "remote"), "na");
    assert_eq!(rig.whats(p.id), Vec::<String>::new(), "an N/A row latched");

    rig.set_na(&p, None, T0 + 330);
    assert_eq!(rig.whats(p.id), ["remote"], "set_check_na did not latch");
}

/// A frozen reading is not recomputed and latches nothing, though its stored inputs now pass.
fn a_frozen_reading_latches_nothing(rig: &Rig) {
    let p = rig.project("frozen", 3, false);
    rig.settle(&p, T0 + 400);
    rig.remote(&p, true);
    rig.sql(
        "UPDATE location SET presence = 'offline' WHERE id = ?1",
        [p.location.0],
    );
    rig.settle(&p, T0 + 410);
    assert_eq!(
        rig.state_of(p.id, "remote"),
        "fail",
        "the frozen rows moved"
    );
    assert_eq!(
        rig.whats(p.id),
        Vec::<String>::new(),
        "a frozen reading latched"
    );

    rig.sql(
        "UPDATE location SET presence = 'present' WHERE id = ?1",
        [p.location.0],
    );
    rig.settle(&p, T0 + 420);
    assert_eq!(
        rig.whats(p.id),
        ["remote"],
        "the thawed reading did not latch"
    );
}

/// Two projects each latched on `remote`, merged: the survivor holds both rows and its next
/// recompute writes no third.
fn a_merge_of_two_latched_projects_writes_no_third(rig: &Rig) {
    let a = rig.project("merge-a", 3, true);
    let b = rig.project("merge-b", 3, true);
    rig.settle(&a, T0 + 500);
    rig.settle(&b, T0 + 510);
    assert_eq!(rig.whats(a.id), ["remote"]);
    assert_eq!(rig.whats(b.id), ["remote"]);

    let survivor = {
        let mut guard = rig.index.lock().unwrap();
        guard
            .with_tx(|tx| {
                Ok(merge_projects(
                    tx,
                    a.id.0,
                    b.id.0,
                    AssociationKind::Manual,
                    &json!({ "rule": "manual" }),
                    T0 + 520,
                )
                .unwrap()
                .survivor)
            })
            .unwrap()
    };
    let kept = if survivor == a.id.0 { &a } else { &b };
    assert_eq!(
        rig.whats(kept.id),
        ["remote", "remote"],
        "the merge did not reparent"
    );

    rig.settle(kept, T0 + 530);
    assert_eq!(rig.state_of(kept.id, "remote"), "pass");
    assert_eq!(
        rig.whats(kept.id),
        ["remote", "remote"],
        "the merged project paid a third time"
    );
}

/// A first commit moves a project's subject from `path:` to `lineage:`, and its key with it:
/// the latch is found by project and check, so nothing is paid again.
fn a_subject_move_repays_nothing(rig: &Rig) {
    let p = rig.project("moved", 3, true);
    rig.settle(&p, T0 + 600);
    let first = rig.latches(p.id);
    assert_eq!(first.len(), 1);

    rig.sql(
        "UPDATE project SET lineage_key = 'moved-lineage' WHERE id = ?1",
        [p.id.0],
    );
    let now_key = {
        let guard = rig.index.lock().unwrap();
        subject_for_project(guard.conn(), p.id)
            .unwrap()
            .unwrap()
            .to_key()
    };
    assert_ne!(first[0].subject_key, now_key, "the subject did not move");

    rig.settle(&p, T0 + 610);
    assert_eq!(rig.latches(p.id), first, "the moved subject paid again");
}

#[test]
fn ac_p4_38_16_the_completion_latch() {
    let rig = Rig::new();
    a_flapping_check_pays_once(&rig);
    a_third_commit_day_latches_on_an_unchanged_recompute(&rig);
    a_ruling_latches_through_its_own_call(&rig);
    a_frozen_reading_latches_nothing(&rig);
    a_merge_of_two_latched_projects_writes_no_third(&rig);
    a_subject_move_repays_nothing(&rig);
}
