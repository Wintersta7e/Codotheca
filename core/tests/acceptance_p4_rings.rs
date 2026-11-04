//! §39 through §43's one read: the criteria the status band's `motivation.get` and its pushed
//! `motivation/changed` make passable. The handler runs over a real index on a fake clock in UTC;
//! every ring input arrives through its production writer — the session manager, J1, J4 and J7
//! over real repositories, the command handlers — and no reading is built by hand.

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

use codotheca_core::art::testsupport::CollectingSink;
use codotheca_core::cancel::CancelToken;
use codotheca_core::clock::{local_date, Clock as _};
use codotheca_core::detail::{dispatch_detail_command, DetailCtx};
use codotheca_core::git::{GitSlots, RepoHandle, StoreKey, SystemGit};
use codotheca_core::health::switches::write_switches;
use codotheca_core::index::Index;
use codotheca_core::jobs::{run_one, Job, JobDeps, JobKind, JobOrigin, NullJobSink, Priority};
use codotheca_core::mount::StoreClass;
use codotheca_core::projects::{dispatch_projects_command, ProjectsCtx};
use codotheca_core::proto::dispatch::CommandHandler as _;
use codotheca_core::proto::pubsub::Queued;
use codotheca_core::protocol::{
    DebtSource, HealthCheckSwitch, LocationId, ProjectId, SessionId, TargetId, Topic,
};
use codotheca_core::scan::ScanLauncher;
use codotheca_core::session::activity::FakeIgnoreCheck;
use codotheca_core::session::manager::{LaunchedSession, SessionManager};
use codotheca_core::session::watch::{ActivityBatch, ActivitySource, FakeActivitySource};
use codotheca_core::session::{SessionError, DEFAULT_TICK_SECS};
use codotheca_core::surfaces::settings::KEY_CONTENT_SCAN_ENABLED;
use codotheca_core::sync::runner::NullSyncSink;
use codotheca_core::testing::events::ValidatingSink;
use codotheca_core::testing::zone::FixedZone;
use codotheca_core::testing::{
    core_handler, CoreHandlerParts, FakeMountResolver, HeldScanLauncher,
};
use serde_json::{json, Value};
use support::TestRepo;

const DAY: i64 = 86_400;
const HOUR: i64 = 3_600;
/// Wednesday 2026-09-16: day one in every fixture that has one.
const D: i64 = 20_712;
/// The Monday of `D`'s week.
const MONDAY: i64 = D - 2;
/// Noon on day one.
const NOW: i64 = D * DAY + 12 * HOUR;
/// `run_loop`'s longest sleep: the pump runs at least this often.
const PARENT_POLL: i64 = 2;
/// The one identity every fixture commit carries.
const EMAIL: &str = "fixture@example.invalid";

/// The handler over a fresh index, the renderer subscribed to `motivation`.
struct Band {
    _dir: tempfile::TempDir,
    index: Arc<Mutex<Index>>,
    parts: CoreHandlerParts,
    /// How many `motivation/changed` deltas earlier pumps already returned.
    seen: usize,
}

impl Band {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let index = Arc::new(Mutex::new(Index::open_at(dir.path(), NOW).unwrap()));
        let launcher: Arc<dyn ScanLauncher> = Arc::new(HeldScanLauncher::new(Arc::clone(&index)));
        let parts = core_handler(dir.path(), &index, NOW, launcher);
        parts.events.with(|p| p.subscribe(Topic::Motivation));
        Self {
            _dir: dir,
            index,
            parts,
            seen: 0,
        }
    }

    fn now(&self) -> i64 {
        self.parts.clock.now_unix()
    }

    fn handle(&mut self, command: &str, args: Value) -> Value {
        self.parts
            .handler
            .handle(command, args)
            .unwrap_or_else(|e| panic!("{command}: {e:?}"))
    }

    fn get(&mut self) -> Value {
        self.handle("motivation.get", json!({}))
    }

    /// Day one, stamped through §43's own writer: the band was shown at the clock's instant.
    fn band_shown(&mut self) {
        self.handle(
            "motivation.acknowledge",
            json!({ "event": "bandShown", "level": null }),
        );
    }

    /// One loop iteration; the `motivation/changed` payloads it pushed.
    fn pump(&mut self) -> Vec<Value> {
        self.parts.handler.pump();
        let all: Vec<Value> = self
            .parts
            .events
            .with(|p| p.queued(Topic::Motivation))
            .into_iter()
            .filter_map(|queued| match queued {
                Queued::Delta { event, data, .. } if event == "changed" => Some(data),
                _ => None,
            })
            .collect();
        let new = all[self.seen..].to_vec();
        self.seen = all.len();
        new
    }

    /// Pumps every `PARENT_POLL` for `secs` of fake time; each push with the instant it came at.
    fn run_for(&mut self, secs: i64) -> Vec<(i64, Value)> {
        let mut pushed = Vec::new();
        for _ in 0..secs / PARENT_POLL {
            self.parts.clock.advance(PARENT_POLL);
            let at = self.now();
            pushed.extend(self.pump().into_iter().map(|p| (at, p)));
        }
        pushed
    }

    fn sql<T: rusqlite::types::FromSql>(&self, sql: &str) -> T {
        let guard = self.index.lock().unwrap();
        let value = guard.conn().query_row(sql, [], |r| r.get(0)).unwrap();
        drop(guard);
        value
    }

    fn exec(&self, sql: &str, params: impl rusqlite::Params) -> i64 {
        let guard = self.index.lock().unwrap();
        guard.conn().execute(sql, params).unwrap();
        let id = guard.conn().last_insert_rowid();
        drop(guard);
        id
    }

    /// Waits, in real time, until no write has landed for a while: the handler's job pump runs
    /// on its own thread.
    fn quiesce(&self) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let mut quiet = 0;
        let mut seen = self.total_changes();
        while quiet < 5 {
            assert!(
                std::time::Instant::now() < deadline,
                "the index never went quiet"
            );
            std::thread::sleep(std::time::Duration::from_millis(40));
            let now = self.total_changes();
            quiet = if now == seen { quiet + 1 } else { 0 };
            seen = now;
        }
    }

    fn total_changes(&self) -> u64 {
        let guard = self.index.lock().unwrap();
        let n = guard.conn().total_changes();
        drop(guard);
        n
    }
}

/// `today`'s cell of the band's rhythm week.
fn today_cell(state: &Value) -> &str {
    let today = state["rings"]["today"].as_str().unwrap();
    let monday = state["rings"]["weekStart"].as_str().unwrap();
    let offset = (1..7)
        .find(|n| local_date(MONDAY + n) == today)
        .map_or(0, |n| usize::try_from(n).unwrap());
    assert_eq!(monday, local_date(MONDAY), "the fixture's week moved");
    state["rings"]["rhythm"]["days"][offset].as_str().unwrap()
}

// ---------------------------------------------------------------------------------------------
// AC-P4-39-7-read

/// `(app_meta rows, rhythm_week rows, change counter)`.
fn footprint(band: &Band) -> (i64, i64, u64) {
    (
        band.sql("SELECT count(*) FROM app_meta"),
        band.sql("SELECT count(*) FROM rhythm_week"),
        band.total_changes(),
    )
}

/// A band whose index has everything a settlement would write from: a finished scan, the
/// first-run stamp, and commit-days across the two weeks before `D`'s.
fn settleable(day_one: Option<i64>) -> Band {
    let mut band = Band::new();
    {
        let guard = band.index.lock().unwrap();
        support::rings_rig::scan_run(guard.conn(), false);
        codotheca_core::firstrun::stamp_first_run_completed(guard.conn(), NOW - 20 * DAY).unwrap();
        drop(guard);
    }
    let project = band.exec(
        "INSERT INTO project (name, seed_basename, lineage_key, created_at, updated_at)
         VALUES ('p', 'p', 'p', 1, 1)",
        [],
    );
    for day in MONDAY - 14..MONDAY {
        band.exec(
            "INSERT INTO xp_events (ts, project_id, subject_key, kind, dedupe_key, track, meta)
             VALUES (?1, ?2, 'subject', 'commit_day', ?3, 'git', '{}')",
            rusqlite::params![day * DAY, project, format!("commit_day:{project}:{day}")],
        );
    }
    if let Some(day) = day_one {
        band.parts.clock.set_unix(day * DAY + 9 * HOUR);
        band.band_shown();
        band.parts.clock.set_unix(NOW);
    }
    band
}

/// **AC-P4-39-7-read.** Computing the rings through `motivation.get`'s production path writes no
/// `app_meta` row and no `rhythm_week` row, and moves the connection's change counter not at all —
/// with day one NULL, today and yesterday. A settlement was due each time: the pump after the
/// reads writes the weeks the reads did not. With the identity answered and the earned level above
/// the floor, the read's one change is §38's ratchet of `level_floor`, and a repeat read none.
#[test]
fn ac_p4_39_7_reading_the_band_writes_nothing() {
    for (name, day_one) in [
        ("NULL", None),
        ("today", Some(D)),
        ("yesterday", Some(D - 1)),
    ] {
        let mut band = settleable(day_one);
        let before = footprint(&band);
        let read = band.get();
        let again = band.get();
        let after = footprint(&band);
        eprintln!(
            "day one {name}: waived {}, before {before:?}, after {after:?}",
            read["rings"]["waived"]
        );
        assert_eq!(read, again, "day one {name}: two reads disagree");
        assert_eq!(
            read["rings"]["waived"],
            day_one != Some(D - 1),
            "day one {name}"
        );
        assert_eq!(before, after, "day one {name}: the read wrote");

        band.pump();
        let weeks: i64 = band.sql("SELECT count(*) FROM rhythm_week");
        eprintln!("day one {name}: the pump settled {weeks} weeks");
        assert!(
            weeks > before.1,
            "day one {name}: nothing was due, so the read had nothing to write"
        );
    }

    // §38's ratchet: with the identity answered and a computed reading above the floor, the read
    // raises `level_floor` — the one write `motivation.get` may make, and only once.
    let mut band = settleable(None);
    band.exec(
        "INSERT INTO identity (email, is_user, source, confirmed_at) VALUES (?1, 1, 'manual', 1)",
        [EMAIL],
    );
    let contents = |of: &Band| -> (String, String) {
        (
            of.sql(
                "SELECT coalesce(group_concat(k || '=' || v, ';'), '')
                   FROM (SELECT k, v FROM app_meta ORDER BY k)",
            ),
            of.sql(
                "SELECT coalesce(group_concat(week_start || ':' || verdict || ':' || settled_at, ';'), '')
                   FROM (SELECT * FROM rhythm_week ORDER BY week_start)",
            ),
        )
    };
    let floor = |of: &Band| -> (i64, Option<i64>) {
        let guard = of.index.lock().unwrap();
        let row = guard
            .conn()
            .query_row(
                "SELECT level, raised_at FROM level_floor WHERE id = 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        drop(guard);
        row
    };
    let (rows_before, contents_before, floor_before) =
        (footprint(&band), contents(&band), floor(&band));
    let read = band.get();
    let earned = read["level"]["figures"]["level"].as_i64().unwrap();
    let (rows_after, contents_after, floor_after) =
        (footprint(&band), contents(&band), floor(&band));
    eprintln!(
        "ratchet: state {}, earned {earned}, floor {floor_before:?} -> {floor_after:?}, \
         changes {} -> {}",
        read["level"]["state"], rows_before.2, rows_after.2
    );
    assert_eq!(read["level"]["state"], "computed");
    assert!(
        earned > floor_before.0,
        "the fixture's earned level must exceed the floor"
    );
    assert_eq!((rows_before.0, rows_before.1), (rows_after.0, rows_after.1));
    assert_eq!(
        contents_before, contents_after,
        "the read touched app_meta or rhythm_week"
    );
    assert_eq!(
        floor_after,
        (earned, Some(band.now())),
        "the floor is raised to the earned level"
    );
    assert_eq!(
        rows_after.2 - rows_before.2,
        1,
        "the floor row is the only change the read makes"
    );
    let again = band.get();
    assert_eq!(read, again, "a repeat read disagrees");
    assert_eq!(
        footprint(&band).2,
        rows_after.2,
        "a repeat read changed something"
    );
}

// ---------------------------------------------------------------------------------------------
// AC-P4-39-10-dirty

/// **AC-P4-39-10-dirty.** With no session and no write, local midnight is the only input: exactly
/// one `motivation/changed` arrives within one `DEFAULT_TICK_SECS` of it, dated the new day.
#[test]
fn ac_p4_39_10_the_rollover_publishes_within_one_pump_period() {
    let mut band = Band::new();
    band.band_shown();
    assert_eq!(band.pump().len(), 1, "the first computation is pushed");
    let midnight = (D + 1) * DAY;
    band.parts.clock.set_unix(midnight - 10);
    assert!(band.pump().is_empty(), "a push before midnight");

    let tick = i64::try_from(DEFAULT_TICK_SECS).unwrap();
    let pushed = band.run_for(10 + tick);
    let offsets: Vec<i64> = pushed.iter().map(|(at, _)| at - midnight).collect();
    eprintln!(
        "across midnight: {} pushed, at {offsets:?} s past it",
        pushed.len()
    );
    assert_eq!(pushed.len(), 1, "{pushed:?}");
    let (at, payload) = &pushed[0];
    assert!(
        (midnight..=midnight + tick).contains(at),
        "pushed {} s from midnight",
        at - midnight
    );
    // §43.3 recomputes when the local date moved, not only when the index did: the push is the
    // first loop iteration past midnight, before the day close's own writes could mark it stale.
    assert!(
        at - midnight < PARENT_POLL,
        "pushed {} s past midnight: it waited for a write",
        at - midnight
    );
    assert_eq!(payload["rings"]["today"], local_date(D + 1));
}

// ---------------------------------------------------------------------------------------------
// The world AC-P4-39-11 and AC-P4-39-13 drive.

/// The manager takes `Box<dyn ActivitySource>`; the test keeps a handle to stand in for the OS.
#[derive(Debug, Clone, Default)]
struct SharedActivity(Arc<Mutex<FakeActivitySource>>);

impl ActivitySource for SharedActivity {
    fn watch(&mut self, session: SessionId, root: &std::path::Path) -> Result<(), SessionError> {
        self.0
            .lock()
            .map_err(|_| SessionError::Watch("the shared activity lock is poisoned".to_owned()))?
            .watch(session, root)
    }
    fn unwatch(&mut self, session: SessionId) {
        self.0.lock().unwrap().unwatch(session);
    }
    fn drain(&mut self) -> Vec<ActivityBatch> {
        self.0.lock().unwrap().drain()
    }
}

/// One copy the jobs run over: its repository and its rows.
struct Copy {
    repo: TestRepo,
    project: ProjectId,
    location: LocationId,
}

/// The band plus every production writer a ring reads: a session manager on the band's clock,
/// and three real repositories — `tended` (enrolled by `projects.get`, its TODOs swept by J7,
/// its history read by J4), `shipped` (pushed to a bare remote, read by J1) and a launch target.
struct World {
    band: Band,
    manager: SessionManager,
    activity: SharedActivity,
    focus: (ProjectId, LocationId, TargetId),
    tended: Copy,
    shipped: Copy,
    _remote: TestRepo,
}

impl World {
    /// Day one stamped at noon on `D`; the clock then at 09:00 on `D + 1`, every input still to
    /// come.
    fn new() -> Self {
        let mut band = Band::new();
        band.band_shown();
        band.parts.clock.set_unix((D + 1) * DAY + 9 * HOUR);
        let focus = focus_copy(&band);
        let tended = tended_copy(&band);
        let remote = TestRepo::init_bare();
        let shipped = shipped_copy(&band, &remote);
        let activity = SharedActivity::default();
        let manager = SessionManager::new(
            band.parts.clock.clone(),
            Arc::new(ValidatingSink::default()),
            Box::new(activity.clone()),
            Arc::new(FakeIgnoreCheck::new(&[])),
            Arc::new(FixedZone(0)),
        );
        let world = Self {
            band,
            manager,
            activity,
            focus,
            tended,
            shipped,
            _remote: remote,
        };
        world.job(&world.shipped, JobKind::J1Refstate);
        world
    }

    /// A launch, a worktree change ten minutes in, then idle until the segment closes: 30
    /// credited minutes, all through the production manager.
    fn session(&mut self) {
        let (project, location, target) = self.focus;
        let mut guard = self.band.index.lock().unwrap();
        let session = self
            .manager
            .launch(
                &mut guard,
                LaunchedSession {
                    project_id: project,
                    location_id: Some(location),
                    target_id: Some(target),
                    repo: RepoHandle::bare(
                        std::path::Path::new("/w/focus"),
                        StoreKey::new("store"),
                        StoreClass::Local,
                    ),
                    waiter: None,
                },
            )
            .unwrap();
        drop(guard);
        self.ticks(39);
        self.activity.0.lock().unwrap().push(session, "src/main.rs");
        self.ticks(201);
    }

    fn ticks(&mut self, count: usize) {
        let tick = i64::try_from(DEFAULT_TICK_SECS).unwrap();
        for _ in 0..count {
            self.band.parts.clock.advance(tick);
            let mut guard = self.band.index.lock().unwrap();
            self.manager.tick(&mut guard).unwrap();
            drop(guard);
        }
    }

    /// One job through `run_one`, the dispatch the runner itself calls, over real git on the
    /// band's clock.
    fn job(&self, copy: &Copy, kind: JobKind) {
        let clock = self.band.parts.clock.clone();
        let deps = JobDeps {
            git: Arc::new(SystemGit::new(
                Arc::new(copy.repo.exec()),
                Arc::new(GitSlots::new(4)),
                clock.clone(),
            )),
            clock,
            cancel: CancelToken::new(),
            zone: Arc::new(FixedZone(0)),
        };
        let job = Job {
            kind,
            project_id: copy.project,
            location_id: copy.location,
            store_key: "store".to_owned(),
            store_kind: StoreClass::Local,
            priority: Priority::Interactive,
            not_before: 0,
            origin: JobOrigin::Interactive,
        };
        run_one(&self.band.index, &deps, &job, &RefCell::new(Vec::new()))
            .unwrap_or_else(|e| panic!("{kind:?}: {e:?}"));
    }

    /// J1 then J7 over `tended`'s HEAD, the way a returning walk queues them.
    fn content_scan(&self) {
        self.job(&self.tended, JobKind::J1Refstate);
        self.job(&self.tended, JobKind::J7Markers);
    }

    /// `tended`'s HEAD now holds `n` TODO lines, committed at the band's instant.
    fn markers(&self, n: usize) {
        let body = (0..n).fold(String::new(), |mut body, i| {
            writeln!(body, "// TODO: marker number {i}").unwrap();
            body
        });
        self.tended.repo.write("src/a.rs", body.as_bytes());
        self.tended
            .repo
            .commit_at(&format!("{n} markers"), &iso(self.band.now()));
    }

    /// `projects.get` on `tended`, answered by the command's own handler: it enrols the project.
    fn open_page(&self) {
        let guard = self.band.index.lock().unwrap();
        let git = SystemGit::new(
            Arc::new(self.tended.repo.exec()),
            Arc::new(GitSlots::new(4)),
            self.band.parts.clock.clone(),
        );
        let ctx = DetailCtx {
            index: &guard,
            git: &git,
            mount: &FakeMountResolver::default(),
            events: &CollectingSink::default(),
            jobs: &NullJobSink,
            sync: &NullSyncSink,
            now: self.band.now(),
            zone: &FixedZone(0),
        };
        dispatch_detail_command(&ctx, "projects.get", json!({ "id": self.tended.project.0 }))
            .expect("owned")
            .expect("ok");
        drop(guard);
    }

    /// `projects.setFlags`, answered by the command's own handler.
    fn archive(&self, archived: bool) {
        let guard = self.band.index.lock().unwrap();
        let ctx = ProjectsCtx {
            index: &guard,
            events: &CollectingSink::default(),
            jobs: &NullJobSink,
            mounts: &FakeMountResolver::default(),
            sync: &NullSyncSink,
            now: self.band.now(),
            tz_offset_min: 0,
        };
        dispatch_projects_command(
            &ctx,
            "projects.setFlags",
            json!({ "id": self.tended.project.0, "isArchived": archived }),
        )
        .expect("owned")
        .expect("ok");
        drop(guard);
    }

    /// `shipped` pushes now, the reflog line dated at the band's instant; then J1 reads it.
    fn push(&self) {
        let date = format!("{} +0000", self.band.now());
        self.shipped.repo.write("b.txt", b"b");
        self.shipped.repo.git(&["add", "b.txt"]);
        dated(&self.shipped.repo, &date, &["commit", "-q", "-m", "b"]);
        dated(&self.shipped.repo, &date, &["push", "-q", "origin", "main"]);
        self.job(&self.shipped, JobKind::J1Refstate);
    }

    /// A J7 closure: five TODOs read, then one removed an hour later and read again.
    fn closure(&mut self) -> Vec<Value> {
        self.markers(5);
        self.content_scan();
        let opened = self.band.pump();
        self.band.parts.clock.advance(HOUR);
        self.markers(4);
        let before: i64 = self
            .band
            .sql("SELECT count(*) FROM xp_events WHERE kind = 'debt_day'");
        self.content_scan();
        let after: i64 = self
            .band
            .sql("SELECT count(*) FROM xp_events WHERE kind = 'debt_day'");
        eprintln!(
            "five TODOs pushed {}; the closure paid {} debt_day",
            opened.len(),
            after - before
        );
        assert_eq!(after - before, 1, "the closure paid no debt_day");
        opened
    }
}

fn iso(unix: i64) -> String {
    chrono::DateTime::from_timestamp(unix, 0)
        .unwrap()
        .to_rfc3339()
}

/// [`TestRepo::git`] with the committer date fixed, which a push's reflog line carries.
fn dated(repo: &TestRepo, date: &str, args: &[&str]) {
    let home = repo.scratch().join("home");
    let out = std::process::Command::new("git")
        .current_dir(repo.path())
        .env("HOME", &home)
        .env("XDG_CONFIG_HOME", &home)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("LC_ALL", "C")
        .env("GIT_AUTHOR_NAME", "Fixture")
        .env("GIT_AUTHOR_EMAIL", EMAIL)
        .env("GIT_COMMITTER_NAME", "Fixture")
        .env("GIT_COMMITTER_EMAIL", EMAIL)
        .env("GIT_AUTHOR_DATE", date)
        .env("GIT_COMMITTER_DATE", date)
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

/// A project with no copy the jobs read, and a launch target on it.
fn focus_copy(band: &Band) -> (ProjectId, LocationId, TargetId) {
    let project = band.exec(
        "INSERT INTO project (name, seed_basename, lineage_key, created_at, updated_at)
         VALUES ('focus', 'focus', 'focus', 1, 1)",
        [],
    );
    let location = band.exec(
        "INSERT INTO location (project_id, kind, path_bytes, path_key, path_display, store_key,
                               presence, repo_kind, push_record)
         VALUES (?1, 'linux', CAST('/w/focus' AS BLOB), CAST('/w/focus' AS BLOB), '/w/focus', 'store',
                 'present', 'worktree', 'no_remote')",
        [project],
    );
    let target = band.exec(
        "INSERT INTO launch_target (project_id, location_id, kind, name, exec_bytes, sort_index)
         VALUES (?1, ?2, 'editor', 'an editor', X'2F65', 0)",
        [project, location],
    );
    (ProjectId(project), LocationId(location), TargetId(target))
}

/// An authored project on a real copy, the user's identity confirmed, its first commit dated
/// Monday of `D`'s week with no TODO in it. Every health check but `todo_marker` is off and the
/// content scan granted, so J7's sweep alone decides whether Tend has looked.
fn tended_copy(band: &Band) -> Copy {
    let repo = TestRepo::init();
    repo.write("src/a.rs", b"fn main() {}\n");
    repo.commit_at("first", &iso(MONDAY * DAY + 10 * HOUR));
    let copy = located(band, repo, "tended", true);
    band.exec(
        "INSERT INTO identity (email, is_user, source, confirmed_at) VALUES (?1, 1, 'manual', 1)",
        [EMAIL],
    );
    band.exec(
        "INSERT INTO project_committer (project_id, email, commits) VALUES (?1, ?2, 1)",
        rusqlite::params![copy.project.0, EMAIL],
    );
    band.exec(
        "INSERT INTO app_meta (k, v) VALUES (?1, '1')",
        [KEY_CONTENT_SCAN_ENABLED],
    );
    let off: Vec<HealthCheckSwitch> = DebtSource::ALL
        .into_iter()
        .filter(|&check| check != DebtSource::TodoMarker)
        .map(|check| HealthCheckSwitch {
            check,
            enabled: false,
        })
        .collect();
    band.index
        .lock()
        .unwrap()
        .with_tx(|tx| write_switches(tx, &off))
        .unwrap();
    copy
}

/// A copy with one commit and `origin` at `remote`, never pushed.
fn shipped_copy(band: &Band, remote: &TestRepo) -> Copy {
    let repo = TestRepo::init();
    repo.write("a.txt", b"a");
    repo.commit("a");
    repo.git(&["remote", "add", "origin", &remote.path().to_string_lossy()]);
    located(band, repo, "shipped", false)
}

fn located(band: &Band, repo: TestRepo, name: &str, authored: bool) -> Copy {
    let project = band.exec(
        "INSERT INTO project (name, seed_basename, lineage_key, authored_by_user, is_reference,
                              created_at, updated_at)
         VALUES (?1, ?1, ?1, ?2, 0, 1, 1)",
        rusqlite::params![name, authored],
    );
    let path = repo.path().to_string_lossy().into_owned();
    let location = band.exec(
        "INSERT INTO location (project_id, kind, path_bytes, path_key, path_display, store_key,
                               presence, repo_kind)
         VALUES (?1, 'linux', ?2, ?2, ?3, 'store', 'present', 'worktree')",
        rusqlite::params![project, path.as_bytes(), path],
    );
    Copy {
        repo,
        project: ProjectId(project),
        location: LocationId(location),
    }
}

// ---------------------------------------------------------------------------------------------
// AC-P4-39-11

/// What one input class produced: its name and the pushes the one pump after it carried.
struct Class {
    name: &'static str,
    pushed: Vec<Value>,
}

/// Drives each class through its production path, then pumps once. Returns every class, the
/// unchanged recompute and the idle hour included, in order.
fn drive_every_class(world: &mut World) -> Vec<Class> {
    let mut classes = Vec::new();
    let mut class = |w: &mut World, name: &'static str, drive: &dyn Fn(&mut World)| {
        drive(w);
        classes.push(Class {
            name,
            pushed: w.band.pump(),
        });
    };
    class(world, "session activity", &|w| w.session());
    class(world, "J4 settle (commit_day this week)", &|w| {
        w.job(&w.tended, JobKind::J4History);
    });
    class(world, "settings.set focusTargetMinutes", &|w| {
        w.band.handle(
            "settings.set",
            json!({ "patch": { "focusTargetMinutes": 60 } }),
        );
    });
    class(world, "E-membership: projects.get acknowledges", &|w| {
        w.open_page();
    });
    class(world, "sweep outcome change (J7)", &|w| w.content_scan());
    class(world, "E-membership: archived", &|w| w.archive(true));
    let unarchived = {
        world.archive(false);
        world.band.pump()
    };
    assert_eq!(unarchived.len(), 1, "unarchiving: {unarchived:?}");
    class(world, "debt_day (J7 closure)", &|w| {
        w.closure();
    });
    class(world, "J1 push-record change", &|w| w.push());
    class(world, "unchanged recompute (view.set)", &|w| {
        let before = w.band.total_changes();
        w.band
            .handle("view.set", json!({ "patch": { "density": 3 } }));
        assert_ne!(before, w.band.total_changes(), "view.set wrote nothing");
    });
    let idle = world.band.run_for(HOUR);
    classes.push(Class {
        name: "one idle hour",
        pushed: idle.into_iter().map(|(_, p)| p).collect(),
    });
    let midnight = (D + 2) * DAY;
    let now = world.band.now();
    world.band.parts.clock.advance(midnight - 10 - now);
    let crossed = world
        .band
        .run_for(10 + i64::try_from(DEFAULT_TICK_SECS).unwrap());
    classes.push(Class {
        name: "rollover",
        pushed: crossed.into_iter().map(|(_, p)| p).collect(),
    });
    classes
}

/// **AC-P4-39-11.** Each input class, through its production path, drives exactly one
/// `motivation/changed` whose rings differ from the last push; a write the reading does not read
/// and an idle hour drive none.
#[test]
fn ac_p4_39_11_each_input_class_publishes_once() {
    let mut world = World::new();
    let mut first = world.band.pump();
    assert_eq!(first.len(), 1, "the first computation is pushed");
    // The first tick's day close queued a daily pass on the handler's own job pump; its writes
    // land on another thread, so they are let settle before any class is measured.
    world.band.quiesce();
    first.extend(world.band.pump());
    let mut last = first.last().unwrap()["rings"].clone();
    assert_eq!(last["waived"], false, "day one is not yesterday");

    let classes = drive_every_class(&mut world);
    let mut problems = Vec::new();
    for Class { name, pushed } in &classes {
        let silent = matches!(*name, "unchanged recompute (view.set)" | "one idle hour");
        // A push whose rings equal the last ones is a level change alone — the rollover's day
        // close queues a daily pass whose job states move the pending count — and not this
        // criterion's event; a silent class may push nothing at all.
        let mut differing = 0;
        for push in pushed {
            if push["rings"] != last {
                differing += 1;
                last = push["rings"].clone();
            }
        }
        eprintln!(
            "{name}: {differing} with rings changed, {} pushed",
            pushed.len()
        );
        if silent && !pushed.is_empty() {
            problems.push(format!("{name} pushed {}", pushed.len()));
        } else if !silent && differing != 1 {
            problems.push(format!("{name}: {differing} pushes changed the rings"));
        }
    }
    assert!(problems.is_empty(), "{problems:#?}");
    assert_eq!(classes.len(), 11, "a class went missing");
    assert_eq!(last["today"], local_date(D + 2));
}

// ---------------------------------------------------------------------------------------------
// AC-P4-39-13

/// **AC-P4-39-13.** Day one stamped at `D` by §43's writer, the clock at `D + 1`; a session
/// launched and touched through the manager, a J7 closure paying a `debt_day`, and J1 reading a
/// push dated `D + 1`. `motivation.get` then carries Focus's union, Tend closed, Ship closed and
/// today's cell active — every ring from production, none built by hand.
#[test]
fn ac_p4_39_13_motivation_get_carries_every_ring_from_production() {
    let mut world = World::new();
    world.session();
    world.open_page();
    world.closure();
    world.push();
    let state = world.band.get();
    let rings = &state["rings"];
    eprintln!("motivation.get: {rings}");
    assert_eq!(rings["today"], local_date(D + 1));
    assert_eq!(rings["waived"], false);
    assert_eq!(rings["focus"]["creditedSeconds"], 1_800, "Focus's union");
    assert_eq!(rings["tend"]["status"], "closed");
    assert_eq!(rings["ship"]["status"], "closed");
    assert_eq!(rings["ship"]["pushedOn"], local_date(D + 1));
    assert_eq!(today_cell(&state), "active");
}
