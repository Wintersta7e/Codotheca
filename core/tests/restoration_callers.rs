//! §34.2 — **every writer of the open debt set is a `health_delta` caller, driven for real.**
//!
//! Three sites write the open debt set and each takes both snapshots inside its own transaction:
//! `JobRunner::settle` around §28's singleton evaluator, J7's item build (the TODO the user closes
//! is closed there, inside the job's own transaction), and `SyncRunner::settle` for a
//! `ProjectRemote` sync — the last is driven in `sync_runner.rs` beside its fixture. A producer
//! declared and tested against a fake caller compiles, passes, and writes nothing at assembly.
#![cfg(feature = "testkit")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use std::cell::RefCell;
use std::fmt::Write as _;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use codotheca_core::cancel::CancelToken;
use codotheca_core::git::{GitSlots, JobClass, JobContext, RepoHandle, SystemGit, TreeEntry};
use codotheca_core::health::switches::write_switches;
use codotheca_core::index::Index;
use codotheca_core::jobs::scheduler::JobRunner;
use codotheca_core::jobs::{j7_markers, Job, JobDeps, JobKind, JobOrigin, Priority};
use codotheca_core::mount::StoreClass;
use codotheca_core::proto::EventSink;
use codotheca_core::protocol::{
    DebtSource, DecayLayer, HealthCheckSwitch, HealthDetectedIn, LocationId, ProjectHealthDelta,
    ProjectId,
};
use codotheca_core::restoration::detected_in_for;
use codotheca_core::testing::{FakeClock, FakeGitBackend, GitReply};
use rusqlite::Connection;
use support::TestRepo;

/// The clock the J7 fixture writes at.
const NOW: i64 = 1_800_000_000;
/// Enrolment happened before any evidence below.
const ACK: i64 = NOW - 1_000;

fn rows(conn: &Connection) -> usize {
    let n: i64 = conn
        .query_row("SELECT count(*) FROM health_delta", [], |r| r.get(0))
        .unwrap();
    usize::try_from(n).unwrap()
}

fn only_row(conn: &Connection) -> (String, f64, f64, String) {
    conn.query_row(
        "SELECT layer, from_value, to_value, detected_in FROM health_delta",
        [],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
    )
    .unwrap()
}

/// §34.2: **the provenance of the job, not the user's attention** — and `JobOrigin`, not
/// `Priority`, because the page's own J7 is queued at `Standard`.
#[test]
fn detected_in_follows_the_chain_origin() {
    assert_eq!(
        detected_in_for(JobOrigin::Interactive),
        HealthDetectedIn::Foreground
    );
    assert_eq!(
        detected_in_for(JobOrigin::Walk),
        HealthDetectedIn::Background
    );
}

// ---------------------------------------------------------------------------------------------
// The production callers, driven for real.
// ---------------------------------------------------------------------------------------------

/// J7 writes the marker items inside its own transaction, so it is a caller in its own right: the
/// TODO the user closes is closed there, not in `settle`. Five markers become four, through the
/// real `run_j7` over a scripted tree, and the event is handed out for the runner to announce.
#[test]
fn ac_p3_34_2_closing_one_of_five_todos_through_j7_writes_one_row() {
    let rig = J7Rig::new();
    rig.tree("head-one", 5);
    let first = rig.run(HealthDetectedIn::Foreground);
    assert!(first.is_empty(), "a first observation was announced");
    assert_eq!(rig.rows(), 0);

    rig.tree("head-two", 4);
    let second = rig.run(HealthDetectedIn::Foreground);
    let written = rig.rows();
    eprintln!(
        "J7, five markers to four: {written} row(s), {} event(s)",
        second.len()
    );
    assert_eq!(written, 1);
    assert_eq!(second.len(), 1);
    let delta = &second[0];
    assert_eq!(delta.id, rig.project);
    assert_eq!(delta.detected_in, HealthDetectedIn::Foreground);
    assert_eq!(delta.layers.len(), 1);
    assert_eq!(delta.layers[0].layer, DecayLayer::Overgrowth);
    assert_eq!(delta.layers[0].from_value, Some(5.0));
    assert_eq!(delta.layers[0].to_value, Some(4.0));
}

struct J7Rig {
    _dir: tempfile::TempDir,
    index: Mutex<Index>,
    git: Arc<FakeGitBackend>,
    project: ProjectId,
    location: LocationId,
    repo: RepoHandle,
}

impl J7Rig {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let mut index = Index::open_at(dir.path(), NOW).unwrap();
        let (project, location) = index
            .with_tx(|tx| {
                tx.execute(
                    "INSERT INTO project (name, seed_basename, lineage_key, authored_by_user,
                                          is_reference, acknowledged_at, created_at, updated_at)
                     VALUES ('p', 'p', 'fixture', 1, 0, ?1, 0, 0)",
                    [ACK],
                )?;
                let project = ProjectId(tx.last_insert_rowid());
                tx.execute(
                    "INSERT INTO app_meta (k, v) VALUES ('content_scan_enabled', '1')
                     ON CONFLICT(k) DO UPDATE SET v = excluded.v",
                    [],
                )?;
                tx.execute(
                    "INSERT INTO location
                       (project_id, kind, path_bytes, path_key, path_display, store_key,
                        presence, repo_kind, head_oid)
                     VALUES (?1, 'linux', x'2f70', x'2f70', '/p', 'store-a', 'present',
                             'worktree', 'head-one')",
                    [project.0],
                )?;
                let location = LocationId(tx.last_insert_rowid());
                write_switches(
                    tx,
                    &[
                        HealthCheckSwitch {
                            check: DebtSource::MissingTests,
                            enabled: false,
                        },
                        HealthCheckSwitch {
                            check: DebtSource::UnpushedCommits,
                            enabled: false,
                        },
                    ],
                )?;
                Ok((project, location))
            })
            .unwrap();
        Self {
            _dir: dir,
            index: Mutex::new(index),
            git: Arc::new(FakeGitBackend::new()),
            project,
            location,
            repo: RepoHandle::bare(
                std::path::Path::new("/does/not/matter"),
                codotheca_core::git::StoreKey::new("test-store"),
                StoreClass::Local,
            ),
        }
    }

    /// A HEAD of one file carrying `markers` distinct TODO lines.
    fn tree(&self, head: &str, markers: usize) {
        let mut bytes = String::new();
        for i in 0..markers {
            let _ = writeln!(bytes, "// TODO: marker number {i}");
        }
        let oid = format!("{head:a>40}");
        self.git.script_blob(&oid, bytes.as_bytes());
        self.git.always_head_tree(GitReply::Ok(vec![TreeEntry {
            mode: "100644".to_owned(),
            kind: "blob".to_owned(),
            oid,
            path: b"src/a.rs".to_vec(),
        }]));
        let guard = self.index.lock().unwrap();
        guard
            .conn()
            .execute(
                "UPDATE location SET head_oid = ?2 WHERE id = ?1",
                rusqlite::params![self.location.0, head],
            )
            .unwrap();
        drop(guard);
    }

    fn run(&self, detected_in: HealthDetectedIn) -> Vec<ProjectHealthDelta> {
        let cancel = CancelToken::new();
        let ctx = JobContext::new(JobClass::Background, &cancel, None);
        let announce = RefCell::new(Vec::new());
        j7_markers::run_j7(
            &self.index,
            self.git.as_ref(),
            &self.repo,
            &ctx,
            j7_markers::ScanRun {
                project: self.project,
                location: self.location,
                cursor: None,
                now: NOW,
                tz_offset_min: 0,
                detected_in,
                announce: Some(&announce),
            },
        )
        .unwrap();
        announce.into_inner()
    }

    fn rows(&self) -> usize {
        let guard = self.index.lock().unwrap();
        let n = rows(guard.conn());
        drop(guard);
        n
    }
}

#[derive(Debug, Default)]
struct RecordingSink {
    events: Mutex<Vec<(String, String, serde_json::Value)>>,
}

impl EventSink for RecordingSink {
    fn emit(&self, topic: &str, event: &str, payload: serde_json::Value) {
        self.events
            .lock()
            .unwrap()
            .push((topic.to_owned(), event.to_owned(), payload));
    }
}

impl RecordingSink {
    fn named(&self, event: &str) -> Vec<serde_json::Value> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter(|(topic, name, _)| topic == "projects" && name == event)
            .map(|(_, _, payload)| payload.clone())
            .collect()
    }
}

/// What one real `JobRunner` produced across a `no_release` closure.
struct Settled {
    row: (String, f64, f64, String),
    duplicates: i64,
    deltas: Vec<ProjectHealthDelta>,
    upserted: usize,
    project: ProjectId,
}

/// The clock the runner fixture is frozen at.
const T0: i64 = 1_700_000_000;

/// A real repository, a real `SystemGit` and the real `JobRunner`. `dust` is fed by `no_release`
/// alone (`missing_readme` and `missing_license` are switched off), which J1's tag count answers:
/// no tag opens the item, and a tag closes it — a one-to-zero transition observed by a J1 settle.
struct ReleaseRig {
    repo: TestRepo,
    _dir: tempfile::TempDir,
    index: Arc<Mutex<Index>>,
    project: ProjectId,
    location: LocationId,
}

impl ReleaseRig {
    fn new() -> Self {
        let repo = TestRepo::init();
        // An entry point, so J3 classifies it `cli`. A text-only tree is `docs`, for which
        // `release` is N/A: once J3 ran, the next settle set the item aside instead of closing it.
        repo.write("main.py", b"print('one')\n");
        repo.commit("first");
        let dir = tempfile::tempdir().unwrap();
        let mut index = Index::open_at(dir.path(), T0).unwrap();
        let path = repo.path().to_string_lossy().into_owned();
        let (project, location) = index
            .with_tx(|tx| {
                tx.execute(
                    "INSERT INTO project (name, seed_basename, lineage_key, authored_by_user,
                                          is_reference, acknowledged_at, created_at, updated_at)
                     VALUES ('p', 'p', 'fixture', 1, 0, ?1, 0, 0)",
                    [T0 - 100],
                )?;
                let project = ProjectId(tx.last_insert_rowid());
                tx.execute(
                    "INSERT INTO location (project_id, kind, path_bytes, path_key, path_display,
                                           store_key, presence, repo_kind)
                     VALUES (?1, 'linux', ?2, ?2, ?3, 'store', 'present', 'worktree')",
                    rusqlite::params![project.0, path.as_bytes(), path],
                )?;
                let location = LocationId(tx.last_insert_rowid());
                write_switches(
                    tx,
                    &[
                        HealthCheckSwitch {
                            check: DebtSource::MissingReadme,
                            enabled: false,
                        },
                        HealthCheckSwitch {
                            check: DebtSource::MissingLicense,
                            enabled: false,
                        },
                    ],
                )?;
                Ok((project, location))
            })
            .unwrap();
        Self {
            repo,
            _dir: dir,
            index: Arc::new(Mutex::new(index)),
            project,
            location,
        }
    }

    /// One J1 on a fresh runner, spun until `until` holds — never a bare sleep — then stopped.
    fn run(&self, origin: JobOrigin, events: Arc<RecordingSink>, until: &dyn Fn(&Self) -> bool) {
        let clock = Arc::new(FakeClock::new(T0));
        let deps = JobDeps {
            git: Arc::new(SystemGit::new(
                Arc::new(self.repo.exec()),
                Arc::new(GitSlots::new(4)),
                clock.clone(),
            )),
            clock,
            cancel: CancelToken::new(),
            tz_offset_min: 0,
        };
        let runner = JobRunner::new(Arc::clone(&self.index), deps, events);
        assert!(runner.enqueue(Job {
            kind: JobKind::J1Refstate,
            project_id: self.project,
            location_id: self.location,
            store_key: "store".to_owned(),
            store_kind: StoreClass::Local,
            priority: Priority::Interactive,
            not_before: 0,
            origin,
        }));
        runner.start(1);
        let deadline = Instant::now() + Duration::from_secs(30);
        while Instant::now() < deadline && !until(self) {
            std::thread::sleep(Duration::from_millis(25));
        }
        runner.request_stop();
        runner.join();
    }

    fn count(&self, sql: &str) -> i64 {
        let guard = self.index.lock().unwrap();
        let n = guard
            .conn()
            .query_row(sql, [self.project.0], |r| r.get(0))
            .unwrap();
        drop(guard);
        n
    }

    /// Everything the delta's precondition reads, as text, for a wait that timed out. A timeout
    /// otherwise surfaces as a missing row, which names nothing about why the row is missing.
    fn dump(&self) -> String {
        let guard = self.index.lock().unwrap();
        let conn = guard.conn();
        let mut out = String::new();
        for (label, sql) in [
            (
                "project",
                "SELECT 'authored=' || ifnull(authored_by_user, 'NULL') || ' reference=' || \
                 ifnull(is_reference, 'NULL') || ' ack=' || ifnull(acknowledged_at, 'NULL') || \
                 ' archetype=' || ifnull(archetype, 'NULL') FROM project WHERE id = ?1",
            ),
            (
                "location",
                "SELECT 'presence=' || presence || ' tags=' || ifnull(tag_count, 'NULL') \
                 FROM location WHERE project_id = ?1",
            ),
            (
                "debt_item",
                "SELECT source || ' ' || state || ' ' || scoring || ' seen=' || last_seen_at \
                 FROM debt_item WHERE project_id = ?1",
            ),
            (
                "debt_sweep",
                "SELECT source || ' ' || outcome || ' at=' || observed_at \
                 FROM debt_sweep WHERE project_id = ?1",
            ),
            (
                "job_state",
                "SELECT job || ' ' || state || ' ' || ifnull(reason, '') \
                 FROM project_job_state WHERE project_id = ?1",
            ),
            (
                "health_delta",
                "SELECT layer || ' ' || from_value || '->' || to_value || ' ' || detected_in \
                 FROM health_delta WHERE project_id = ?1",
            ),
        ] {
            let mut statement = conn.prepare(sql).unwrap();
            let rows: Vec<String> = statement
                .query_map([self.project.0], |r| r.get::<_, String>(0))
                .unwrap()
                .map(Result::unwrap)
                .collect();
            let _ = writeln!(out, "  {label}: {rows:?}");
        }
        drop(guard);
        out
    }
}

fn settle_a_release(origin: JobOrigin) -> Settled {
    let rig = ReleaseRig::new();

    // Phase one: the item opens. A first observation writes nothing.
    rig.run(origin, Arc::new(RecordingSink::default()), &|this| {
        this.count("SELECT count(*) FROM debt_item WHERE project_id = ?1 AND source = 'no_release'")
            == 1
    });
    assert_eq!(
        rig.count("SELECT count(*) FROM debt_item WHERE project_id = ?1 AND source = 'no_release'"),
        1,
        "{origin:?}: the no_release item never opened within the wait\n{}",
        rig.dump()
    );
    assert_eq!(
        rig.count("SELECT count(*) FROM health_delta WHERE project_id = ?1"),
        0,
        "the first observation wrote a row"
    );
    // J1.5 may have classified the fixture since; the reading needs it authored and not
    // Reference, which is what it was before the chain ran.
    {
        let guard = rig.index.lock().unwrap();
        guard
            .conn()
            .execute(
                "UPDATE project SET authored_by_user = 1, is_reference = 0 WHERE id = ?1",
                [rig.project.0],
            )
            .unwrap();
        drop(guard);
    }

    // Phase two: a release. The next J1 settle closes the item.
    rig.repo.git(&["tag", "v1"]);
    let events = Arc::new(RecordingSink::default());
    let seen = Arc::clone(&events);
    rig.run(origin, Arc::clone(&events), &move |this| {
        this.count("SELECT count(*) FROM health_delta WHERE project_id = ?1") >= 1
            && !seen.named("health_delta").is_empty()
    });
    assert!(
        rig.count("SELECT count(*) FROM health_delta WHERE project_id = ?1") >= 1,
        "{origin:?}: the release settled no health_delta row within the wait\n{}",
        rig.dump()
    );

    let guard = rig.index.lock().unwrap();
    let row = only_row(guard.conn());
    let duplicates: i64 = guard
        .conn()
        .query_row(
            "SELECT count(*) FROM (SELECT 1 FROM health_delta
                                    GROUP BY project_id, ts, layer HAVING count(*) > 1)",
            [],
            |r| r.get(0),
        )
        .unwrap();
    drop(guard);
    let deltas = events
        .named("health_delta")
        .into_iter()
        .map(|payload| serde_json::from_value::<ProjectHealthDelta>(payload).unwrap())
        .collect();
    let upserted = events.named("upserted").len();
    Settled {
        row,
        duplicates,
        deltas,
        upserted,
        project: rig.project,
    }
}

/// **`AC-P3-34-8` — `detected_in` records provenance, on the job's origin.** The same closure,
/// observed by the real `JobRunner`, is `foreground` on a chain a user started and `background` on
/// a walk's. The row is read out of `health_delta`, not off a return value, and one settle writes
/// one row per layer.
#[test]
fn ac_p3_34_8_the_job_settle_writes_the_origin_of_the_chain() {
    let mut executed = 0usize;
    for (origin, expected) in [
        (JobOrigin::Interactive, "foreground"),
        (JobOrigin::Walk, "background"),
    ] {
        let settled = settle_a_release(origin);
        eprintln!("{origin:?}: {:?}", settled.row);
        assert_eq!(
            settled.row,
            ("dust".to_owned(), 1.0, 0.0, expected.to_owned())
        );
        assert_eq!(
            settled.duplicates, 0,
            "two rows for one layer in one settle"
        );
        executed += 1;
    }
    eprintln!("AC-P3-34-8 origins executed: {executed}");
    assert!(executed > 0);
}

/// The event is announced after the commit, decoded through the generated type, and it is not the
/// state's transport: an interactive chain ALSO publishes `projects.upserted` (R121), and a walk's
/// publishes the delta without it — neither is derived from the other.
#[test]
fn the_settle_announces_the_delta_beside_upserted_and_never_instead_of_it() {
    let interactive = settle_a_release(JobOrigin::Interactive);
    eprintln!(
        "interactive: {} health_delta, {} upserted",
        interactive.deltas.len(),
        interactive.upserted
    );
    assert_eq!(interactive.deltas.len(), 1);
    let delta = &interactive.deltas[0];
    assert_eq!(delta.id, interactive.project);
    assert_eq!(delta.detected_in, HealthDetectedIn::Foreground);
    assert_eq!(delta.layers.len(), 1);
    assert_eq!(delta.layers[0].layer, DecayLayer::Dust);
    assert_eq!(
        (delta.layers[0].from_value, delta.layers[0].to_value),
        (Some(1.0), Some(0.0))
    );
    assert!(
        interactive.upserted > 0,
        "the state's transport was not published"
    );

    let walk = settle_a_release(JobOrigin::Walk);
    eprintln!(
        "walk: {} health_delta, {} upserted",
        walk.deltas.len(),
        walk.upserted
    );
    assert_eq!(walk.deltas.len(), 1);
    assert_eq!(walk.deltas[0].detected_in, HealthDetectedIn::Background);
    assert_eq!(walk.upserted, 0, "a walk chain published a row change");
}
