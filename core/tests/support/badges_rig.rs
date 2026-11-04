//! The library the badge tests drive through their production callers: real repositories whose
//! commits fall on chosen days, each one project on one present copy, settled by a real J4.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use codotheca_core::cancel::CancelToken;
use codotheca_core::clock::local_date;
use codotheca_core::git::{GitSlots, SystemGit};
use codotheca_core::index::Index;
use codotheca_core::jobs::scheduler::JobRunner;
use codotheca_core::jobs::{Job, JobDeps, JobKind, JobOrigin, Priority};
use codotheca_core::mount::StoreClass;
use codotheca_core::protocol::{LocationId, ProjectId, SessionId, TargetId};
use codotheca_core::session::store::{self, SessionOpen};
use codotheca_core::testing::events::ValidatingSink;
use codotheca_core::testing::zone::FixedZone;
use codotheca_core::testing::{FakeClock, TempIndex};
use rusqlite::types::Value;

use super::TestRepo;

/// The runner's frozen clock, after every fixture date.
pub(crate) const T0: i64 = 1_800_000_000;

/// The address every fixture commit is made under, confirmed as the user's.
pub(crate) const ME: &str = "fixture@example.invalid";

/// A project's flags, as the scan would have written them.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Flags {
    pub(crate) shallow: bool,
    pub(crate) reference: bool,
}

struct Copy {
    repo: TestRepo,
    /// The bare remote the copy pushed to, kept alive for J1's reachability read.
    _remote: Option<TestRepo>,
    project: ProjectId,
    location: LocationId,
}

/// One index, its projects, and every event the runners it built emitted.
pub(crate) struct BadgesRig {
    _dir: tempfile::TempDir,
    pub(crate) index: Arc<Mutex<Index>>,
    copies: Vec<Copy>,
    pub(crate) events: Arc<ValidatingSink>,
}

impl BadgesRig {
    pub(crate) fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let index = Index::open_at(dir.path(), T0).unwrap();
        index
            .conn()
            .execute(
                "INSERT INTO identity (email, is_user, source, confirmed_at)
                 VALUES (?1, 1, 'manual', ?2)",
                rusqlite::params![ME, T0 - 100],
            )
            .unwrap();
        Self {
            _dir: dir,
            index: Arc::new(Mutex::new(index)),
            copies: Vec::new(),
            events: Arc::new(ValidatingSink::default()),
        }
    }

    /// A rig over the index already in `dir`, opened through the real path — which runs the
    /// rest of the migration chain — beside the repositories that index's rows already name, each
    /// `(repository, project, its one copy)`.
    pub(crate) fn reopened(
        dir: tempfile::TempDir,
        planted: Vec<(TestRepo, ProjectId, LocationId)>,
    ) -> Self {
        let index = Index::open_at(dir.path(), T0).unwrap();
        Self {
            _dir: dir,
            index: Arc::new(Mutex::new(index)),
            copies: planted
                .into_iter()
                .map(|(repo, project, location)| Copy {
                    repo,
                    _remote: None,
                    project,
                    location,
                })
                .collect(),
            events: Arc::new(ValidatingSink::default()),
        }
    }

    /// A project named `name` whose repository holds one of the user's commits at noon UTC on
    /// each local day number in `days`, on one present copy.
    pub(crate) fn project(&mut self, name: &str, days: &[i64], flags: Flags) -> ProjectId {
        let repo = TestRepo::init();
        for &day in days {
            let date = local_date(day);
            repo.commit_at(&date, &format!("{date}T12:00:00+00:00"));
        }
        self.plant(name, repo, None, flags)
    }

    /// A project §38.8.5 can pay once it is archived: one commit, tagged and pushed to a bare
    /// remote, a clean tree, and one `complete` debt sweep at `T0`. The tag is load-bearing: a
    /// settle reading no tag opens a scored `no_release` item, which withholds the award alone.
    pub(crate) fn archivable(&mut self, name: &str) -> ProjectId {
        let repo = TestRepo::init();
        repo.write("a.txt", b"one\n");
        repo.commit("first");
        repo.git(&["tag", "v1.0.0"]);
        let remote = TestRepo::init_bare();
        let url = remote.path().to_string_lossy().into_owned();
        repo.git(&["remote", "add", "origin", &url]);
        repo.git(&["push", "-q", "origin", "main", "v1.0.0"]);
        let project = self.plant(name, repo, Some(remote), Flags::default());
        let guard = self.index.lock().unwrap();
        guard
            .conn()
            .execute(
                "INSERT INTO debt_sweep (project_id, source, outcome, item_count, observed_at)
                 VALUES (?1, 'todo_marker', 'complete', 0, ?2)",
                rusqlite::params![project.0, T0],
            )
            .unwrap();
        drop(guard);
        project
    }

    /// `projects.setFlags` with `isArchived`, as the product's archive control sends it.
    pub(crate) fn set_archived(&self, project: ProjectId, archived: bool) {
        let guard = self.index.lock().unwrap();
        let sink = codotheca_core::art::testsupport::CollectingSink::default();
        let mounts = codotheca_core::testing::FakeMountResolver::default();
        let ctx = codotheca_core::projects::ProjectsCtx {
            index: &guard,
            events: &sink,
            jobs: &codotheca_core::jobs::NullJobSink,
            mounts: &mounts,
            sync: &codotheca_core::sync::runner::NullSyncSink,
            now: T0,
            tz_offset_min: 0,
        };
        codotheca_core::projects::dispatch_projects_command(
            &ctx,
            "projects.setFlags",
            serde_json::json!({ "id": project.0, "isArchived": archived }),
        )
        .expect("owned")
        .expect("ok");
        drop(guard);
    }

    /// J1's arm at `at` — run alone, as the scheduler runs it, so it chains nothing that would
    /// rewrite the fixture — writing `archive_reached` from the real reachability read; then a
    /// J2's settle at `at + 1`, in a zone `offset_min` east of UTC, which runs §38's writer.
    pub(crate) fn observe_archive(&self, project: ProjectId, at: i64, offset_min: i32) {
        let copy = self.copy(project);
        let clock = Arc::new(FakeClock::new(at));
        let deps = JobDeps {
            git: Arc::new(SystemGit::new(
                Arc::new(copy.repo.exec()),
                Arc::new(GitSlots::new(4)),
                clock.clone(),
            )),
            clock,
            cancel: CancelToken::new(),
            zone: Arc::new(FixedZone(offset_min)),
        };
        codotheca_core::jobs::run_one(
            &self.index,
            &deps,
            &Self::job(copy, JobKind::J1Refstate),
            &std::cell::RefCell::new(Vec::new()),
        )
        .unwrap();
        self.settle_at(project, JobKind::J2Status, at + 1, offset_min);
    }

    /// `project`'s `clean_archive` rows as `(ts, tz_offset_min)`, by `ts` then id.
    pub(crate) fn clean_archive_rows(&self, project: ProjectId) -> Vec<(i64, i64)> {
        let guard = self.index.lock().unwrap();
        let mut st = guard
            .conn()
            .prepare(
                "SELECT ts, tz_offset_min FROM xp_events
                  WHERE project_id = ?1 AND kind = 'clean_archive' ORDER BY ts, id",
            )
            .unwrap();
        let rows = st
            .query_map([project.0], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        drop(st);
        drop(guard);
        rows
    }

    /// Insert `name` on one present copy at `repo`, keeping `remote` alive beside it.
    fn plant(
        &mut self,
        name: &str,
        repo: TestRepo,
        remote: Option<TestRepo>,
        flags: Flags,
    ) -> ProjectId {
        let path = repo.path().to_string_lossy().into_owned();
        let mut guard = self.index.lock().unwrap();
        let (project, location) = guard
            .with_tx(|tx| {
                tx.execute(
                    "INSERT INTO project (name, seed_basename, lineage_key, authored_by_user,
                                          is_shallow, is_reference, created_at, updated_at)
                     VALUES (?1, ?1, ?1, 1, ?2, ?3, 0, 0)",
                    rusqlite::params![name, i64::from(flags.shallow), i64::from(flags.reference)],
                )?;
                let project = ProjectId(tx.last_insert_rowid());
                tx.execute(
                    "INSERT INTO location (project_id, kind, path_bytes, path_key, path_display,
                                           store_key, presence, repo_kind)
                     VALUES (?1, 'linux', ?2, ?2, ?3, 'store', 'present', 'worktree')",
                    rusqlite::params![project.0, path.as_bytes(), path],
                )?;
                let location = LocationId(tx.last_insert_rowid());
                tx.execute(
                    "INSERT INTO project_committer (project_id, email, commits)
                     VALUES (?1, ?2, 1)",
                    rusqlite::params![project.0, ME],
                )?;
                Ok((project, location))
            })
            .unwrap();
        drop(guard);
        self.copies.push(Copy {
            repo,
            _remote: remote,
            project,
            location,
        });
        project
    }

    /// One J4 for `project` on a fresh runner over real git, spun until the runner is idle.
    pub(crate) fn settle_j4(&self, project: ProjectId) {
        self.settle(project, JobKind::J4History);
    }

    /// One J3 for `project`, as [`Self::settle_j4`]: it writes `primary_language` from the
    /// files at HEAD, and the jobs it chains run to idle with it.
    pub(crate) fn settle_j3(&self, project: ProjectId) {
        self.settle(project, JobKind::J3Inventory);
    }

    /// One J2 for `project`, as [`Self::settle_j4`]: the settle that runs §28's singletons and
    /// §31's completion evaluator over the copy's worktree.
    pub(crate) fn settle_j2(&self, project: ProjectId) {
        self.settle(project, JobKind::J2Status);
    }

    /// The facts §31's ten checks read for `project`, as the jobs that own them leave them: a
    /// remote, a tag, a persisted refstate, and a finished content scan whose CI config answer
    /// is `ci` (`present` or `absent`).
    pub(crate) fn completion_facts(&self, project: ProjectId, ci: &str) {
        let guard = self.index.lock().unwrap();
        let conn = guard.conn();
        conn.execute(
            "UPDATE project SET remote_key = 'forge/o/' || name WHERE id = ?1",
            [project.0],
        )
        .unwrap();
        conn.execute(
            "UPDATE location SET tag_count = 1, refstate_observed_at = 50 WHERE project_id = ?1",
            [project.0],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO project_content_scan
                (project_id, head_oid, complete_head_oid, blobs_total, blobs_pending,
                 predicate_version, has_readme, has_license, has_tests, has_ci,
                 presence_observed_at, enumerated_at, completed_at)
             VALUES (?1, 'head0', 'head0', 1, 0, 1, 'present', 'present', 'present', ?2, 1, 1, 1)
             ON CONFLICT(project_id) DO UPDATE SET has_ci = excluded.has_ci",
            rusqlite::params![project.0, ci],
        )
        .unwrap();
        drop(guard);
    }

    /// Each `(project, evaluable)` given the projection a settle that lit every evaluable check
    /// leaves: `completion_lit = completion_applicable = evaluable`.
    pub(crate) fn at_top_rung(&self, projects: &[(ProjectId, i64)]) {
        let guard = self.index.lock().unwrap();
        for &(project, evaluable) in projects {
            guard
                .conn()
                .execute(
                    "UPDATE project SET completion_lit = ?2, completion_applicable = ?2
                      WHERE id = ?1",
                    rusqlite::params![project.0, evaluable],
                )
                .unwrap();
        }
        drop(guard);
    }

    /// `project`'s `(completion_lit, completion_applicable)`.
    pub(crate) fn projection(&self, project: ProjectId) -> (Option<i64>, Option<i64>) {
        let guard = self.index.lock().unwrap();
        let projection = guard
            .conn()
            .query_row(
                "SELECT completion_lit, completion_applicable FROM project WHERE id = ?1",
                [project.0],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        drop(guard);
        projection
    }

    /// Write `rel` into `project`'s repository and commit it at noon UTC on local day `day`, so a
    /// day the project already holds gains no commit-day.
    pub(crate) fn commit_file(&self, project: ProjectId, rel: &str, bytes: &[u8], day: i64) {
        let copy = self
            .copies
            .iter()
            .find(|c| c.project == project)
            .expect("a project this rig planted");
        copy.repo.write(rel, bytes);
        let date = local_date(day);
        copy.repo.commit_at(rel, &format!("{date}T12:00:00+00:00"));
    }

    fn copy(&self, project: ProjectId) -> &Copy {
        self.copies
            .iter()
            .find(|c| c.project == project)
            .expect("a project this rig planted")
    }

    fn job(copy: &Copy, kind: JobKind) -> Job {
        Job {
            kind,
            project_id: copy.project,
            location_id: copy.location,
            store_key: "store".to_owned(),
            store_kind: StoreClass::Local,
            priority: Priority::Interactive,
            not_before: 0,
            origin: JobOrigin::Walk,
        }
    }

    fn settle(&self, project: ProjectId, kind: JobKind) {
        self.settle_at(project, kind, T0, 0);
    }

    /// One `kind` for `project` on a fresh runner whose clock reads `at`, in a zone `offset_min`
    /// east of UTC, spun until the runner is idle.
    fn settle_at(&self, project: ProjectId, kind: JobKind, at: i64, offset_min: i32) {
        let copy = self.copy(project);
        let clock = Arc::new(FakeClock::new(at));
        let git = SystemGit::new(
            Arc::new(copy.repo.exec()),
            Arc::new(GitSlots::new(4)),
            clock.clone(),
        );
        let deps = JobDeps {
            git: Arc::new(git),
            clock,
            cancel: CancelToken::new(),
            zone: Arc::new(FixedZone(offset_min)),
        };
        let runner = JobRunner::new(Arc::clone(&self.index), deps, self.events.clone());
        assert!(runner.enqueue(Self::job(copy, kind)));
        runner.start(1);
        let deadline = Instant::now() + Duration::from_secs(60);
        while Instant::now() < deadline && !runner.is_idle() {
            std::thread::sleep(Duration::from_millis(25));
        }
        let idle = runner.is_idle();
        runner.request_stop();
        runner.join();
        assert!(idle, "{} did not settle for {project:?}", kind.slug());
        let guard = self.index.lock().unwrap();
        let state: String = guard
            .conn()
            .query_row(
                "SELECT state FROM project_job_state WHERE project_id = ?1 AND job = ?2",
                rusqlite::params![project.0, kind.slug()],
                |r| r.get(0),
            )
            .unwrap();
        drop(guard);
        assert_eq!(
            state,
            "ok",
            "{} did not finish for {project:?}",
            kind.slug()
        );
    }

    /// `historyComplete` as the reveal reads it now.
    pub(crate) fn history_complete(&self) -> bool {
        let guard = self.index.lock().unwrap();
        let complete = codotheca_core::stats::reveal::population(guard.conn())
            .unwrap()
            .history_complete;
        drop(guard);
        complete
    }

    /// Run `sql` on the index.
    pub(crate) fn sql(&self, sql: &str) {
        let guard = self.index.lock().unwrap();
        guard.conn().execute_batch(sql).unwrap();
        drop(guard);
    }

    /// Every `badge_award` row, every column, by kind.
    pub(crate) fn awards(&self) -> Vec<Vec<Value>> {
        let guard = self.index.lock().unwrap();
        let mut st = guard
            .conn()
            .prepare("SELECT * FROM badge_award ORDER BY kind")
            .unwrap();
        let width = st.column_count();
        let rows = st
            .query_map([], |r| (0..width).map(|i| r.get::<_, Value>(i)).collect())
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        drop(st);
        drop(guard);
        rows
    }

    /// `kind`'s award as `(earned_on, project_id, evidence)`, when it is held.
    pub(crate) fn award(&self, kind: &str) -> Option<(String, Option<i64>, serde_json::Value)> {
        let guard = self.index.lock().unwrap();
        let row = guard
            .conn()
            .query_row(
                "SELECT earned_on, project_id, evidence FROM badge_award WHERE kind = ?1",
                [kind],
                |r| Ok((r.get(0)?, r.get(1)?, r.get::<_, String>(2)?)),
            )
            .ok();
        drop(guard);
        row.map(|(on, project, evidence)| (on, project, serde_json::from_str(&evidence).unwrap()))
    }

    /// `SELECT COUNT(*) FROM xp_events`.
    pub(crate) fn xp_event_count(&self) -> i64 {
        let guard = self.index.lock().unwrap();
        let n = guard
            .conn()
            .query_row("SELECT COUNT(*) FROM xp_events", [], |r| r.get(0))
            .unwrap();
        drop(guard);
        n
    }

    /// `SELECT COUNT(*) FROM xp_events` of one `kind`.
    pub(crate) fn xp_event_count_of(&self, kind: &str) -> i64 {
        let guard = self.index.lock().unwrap();
        let n = guard
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM xp_events WHERE kind = ?1",
                [kind],
                |r| r.get(0),
            )
            .unwrap();
        drop(guard);
        n
    }
}

/// One `badge_award` row, its columns by name.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct AwardRow {
    pub(crate) earned_at: i64,
    pub(crate) tz_offset_min: i64,
    pub(crate) earned_on: String,
    pub(crate) project_id: Option<i64>,
    pub(crate) subject_key: Option<String>,
    pub(crate) evidence: serde_json::Value,
    pub(crate) awarded_at: i64,
    pub(crate) awarded_on: String,
}

/// `kind`'s row on `conn`, when it is held.
pub(crate) fn award_row(conn: &rusqlite::Connection, kind: &str) -> Option<AwardRow> {
    conn.query_row(
        "SELECT earned_at, tz_offset_min, earned_on, project_id, subject_key, evidence,
                awarded_at, awarded_on
           FROM badge_award WHERE kind = ?1",
        [kind],
        |r| {
            Ok(AwardRow {
                earned_at: r.get(0)?,
                tz_offset_min: r.get(1)?,
                earned_on: r.get(2)?,
                project_id: r.get(3)?,
                subject_key: r.get(4)?,
                evidence: serde_json::from_str(&r.get::<_, String>(5)?).unwrap(),
                awarded_at: r.get(6)?,
                awarded_on: r.get(7)?,
            })
        },
    )
    .ok()
}

/// A session index: one project of **S** named `marathon`, with a lineage, on one present copy
/// with one launch target.
pub(crate) struct SessionLib {
    pub(crate) index: TempIndex,
    pub(crate) project: ProjectId,
    pub(crate) location: LocationId,
    pub(crate) target: TargetId,
}

/// See [`SessionLib`].
pub(crate) fn session_index() -> SessionLib {
    let index = TempIndex::new();
    let project = index.insert_project();
    index
        .index()
        .conn()
        .execute(
            "UPDATE project SET name = 'marathon', lineage_key = 'marathon' WHERE id = ?1",
            [project.0],
        )
        .unwrap();
    let location = index.insert_location(project, "/w/marathon");
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
    SessionLib {
        index,
        project,
        location,
        target,
    }
}

/// `lib`'s project opening a session at `started_at`, on its copy and target.
pub(crate) const fn opening(lib: &SessionLib, started_at: i64) -> SessionOpen {
    SessionOpen {
        project_id: lib.project,
        location_id: Some(lib.location),
        target_id: Some(lib.target),
        started_at,
    }
}

/// A session a previous process left open at `started_at`, its one segment's watermark
/// `credited` seconds in — what a crash leaves behind.
pub(crate) fn orphaned_session(lib: &mut SessionLib, started_at: i64, credited: i64) -> SessionId {
    let open = opening(lib, started_at);
    let tx = lib.index.index_mut().conn_mut().transaction().unwrap();
    let session = store::open_session(&tx, &open).unwrap();
    let segment = store::open_segment(&tx, session, started_at, false).unwrap();
    store::mark_activity(&tx, segment, started_at + credited).unwrap();
    tx.commit().unwrap();
    session
}

/// `session`'s stored `(ended_at, credited_seconds)`.
pub(crate) fn session_row(index: &TempIndex, session: SessionId) -> (Option<i64>, i64) {
    index
        .index()
        .conn()
        .query_row(
            "SELECT ended_at, credited_seconds FROM session WHERE id = ?1",
            [session.0],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap()
}

/// A project of **S** named `name` on one present copy, its ledger keyed by its lineage, with no
/// commit-day of its own; `acknowledged` decides whether it is enrolled.
pub(crate) fn plain_project(index: &mut Index, name: &str, acknowledged: bool) -> ProjectId {
    index
        .with_tx(|tx| {
            tx.execute(
                "INSERT INTO project (name, seed_basename, lineage_key, authored_by_user,
                                      acknowledged_at, created_at, updated_at)
                 VALUES (?1, ?1, ?1, 1, ?2, 0, 0)",
                rusqlite::params![name, acknowledged.then_some(1_i64)],
            )?;
            let project = ProjectId(tx.last_insert_rowid());
            tx.execute(
                "INSERT INTO location (project_id, kind, path_bytes, path_key, path_display,
                                       store_key, presence, repo_kind)
                 VALUES (?1, 'linux', ?2, ?2, CAST(?2 AS TEXT), 'store', 'present',
                         'worktree')",
                rusqlite::params![project.0, format!("/w/{name}").into_bytes()],
            )?;
            Ok(project)
        })
        .unwrap()
}

/// `project`'s CI as §28's `ci_red` arm reads it: the project names its forge repository, its
/// copy is on `main`, and it is enrolled, so a repair pays.
pub(crate) fn connect_ci(index: &Index, project: ProjectId) {
    let conn = index.conn();
    conn.execute(
        "UPDATE project SET provider = 'github', provider_repo_id = name,
                            acknowledged_at = COALESCE(acknowledged_at, 1)
          WHERE id = ?1",
        [project.0],
    )
    .unwrap();
    conn.execute(
        "UPDATE location SET branch = 'main' WHERE project_id = ?1",
        [project.0],
    )
    .unwrap();
}

/// A commit-day row for `project` on local day `day`, as J4 writes one: the day's midnight,
/// offset 0.
pub(crate) fn plant_commit_day(index: &Index, project: ProjectId, day: i64) {
    index
        .conn()
        .execute(
            "INSERT INTO xp_events (ts, tz_offset_min, project_id, subject_key, kind, dedupe_key,
                                    track, meta)
             SELECT ?2 * 86400, 0, id, 'lineage:' || name || '|remote:', 'commit_day',
                    'commit_day:' || name || '::' || ?2, 'git', '{}'
               FROM project WHERE id = ?1",
            rusqlite::params![project.0, day],
        )
        .unwrap();
}

/// A project holding one qualifying Firefighter pair on each local day of `days`: that day's
/// `debt_day` row listing `ci_red` among its same-day repairs, and a commit-day the same date.
pub(crate) fn plant_repairs(index: &mut Index, name: &str, days: &[i64]) -> ProjectId {
    let project = plain_project(index, name, true);
    for &day in days {
        index
            .conn()
            .execute(
                "INSERT INTO xp_events (ts, tz_offset_min, project_id, subject_key, kind,
                                        dedupe_key, track, meta)
                 VALUES (?2 * 86400 + 36000, 0, ?1, ?3, 'debt_day', ?4, 'session',
                         '{\"sources\":[\"ci_red\"],\"closed\":1,\"same_day\":[\"ci_red\"]}')",
                rusqlite::params![
                    project.0,
                    day,
                    format!("lineage:{name}|remote:"),
                    format!("debt_day:lineage:{name}|remote::{}", local_date(day))
                ],
            )
            .unwrap();
        plant_commit_day(index, project, day);
    }
    project
}

/// One concluded run of `project`'s CI on `main`, started at `started_at`.
fn ci_run(index: &Index, project: ProjectId, conclusion: &str, started_at: i64) {
    index
        .conn()
        .execute(
            "INSERT INTO remote_ci_run
                (provider, provider_repo_id, run_id, workflow_name, conclusion, branch,
                 run_number, started_at)
             SELECT 'github', provider_repo_id,
                    (SELECT COALESCE(MAX(run_id), 0) + 1 FROM remote_ci_run), 'build', ?2,
                    'main', 1, ?3
               FROM project WHERE id = ?1",
            rusqlite::params![project.0, conclusion, started_at],
        )
        .unwrap();
}

/// §28's singleton settle for `project` at `unix`, through `settle_singletons` — the payout's
/// caller both runners reach — in a transaction of its own, under UTC.
pub(crate) fn settle_singletons_at(index: &mut Index, project: ProjectId, unix: i64) {
    let zone = FixedZone(0);
    index
        .with_tx(|tx| {
            codotheca_core::debt::singletons::settle_singletons(
                tx,
                project,
                codotheca_core::clock::LocalZone::at(&zone, unix),
                &zone,
                &codotheca_core::debt::store::SqliteDebtStore,
            )
            .unwrap();
            Ok(())
        })
        .unwrap();
}

/// A red build at `red_at` turned green at `green_at`, each observed by a singleton settle a
/// minute after its run started: the `ci_red` item opens at the first and closes at the second.
pub(crate) fn red_then_green(index: &mut Index, project: ProjectId, red_at: i64, green_at: i64) {
    ci_run(index, project, "failure", red_at);
    settle_singletons_at(index, project, red_at + 60);
    ci_run(index, project, "success", green_at);
    settle_singletons_at(index, project, green_at + 60);
}

/// The qualifying Firefighter pairs the library holds, `(project, local date)`, read the way a
/// reader of §40.3 would: a `debt_day` listing `ci_red` as a same-day repair, joined to a
/// commit-day of that project on that date.
pub(crate) fn repair_pairs(index: &Index) -> Vec<(i64, i64)> {
    let mut st = index
        .conn()
        .prepare(
            "SELECT DISTINCT d.project_id, (d.ts + d.tz_offset_min * 60) / 86400 AS day
               FROM xp_events d
              WHERE d.kind = 'debt_day'
                AND EXISTS (SELECT 1 FROM json_each(d.meta, '$.same_day') j
                             WHERE j.value = 'ci_red')
                AND EXISTS (SELECT 1 FROM xp_events c
                             WHERE c.kind = 'commit_day' AND c.project_id = d.project_id
                               AND c.ts / 86400 = (d.ts + d.tz_offset_min * 60) / 86400)
              ORDER BY day, d.project_id",
        )
        .unwrap();
    st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}
