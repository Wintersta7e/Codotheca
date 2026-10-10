//! §39.2 rules 1–5, `AC-P4-39-10` — **the zone is a function of the instant.** A local day is the
//! set of instants whose local date is that day, 23 or 25 hours across a transition, and every
//! `debt_day` payout site dates its row in the offset in force at its own closing instant — not
//! the offset the process happened to read when it started.
//!
//! Each payout site runs ONE runner across a transition (a `TransitionZone` and a `FakeClock`
//! moved past it), so a site that captured the offset once would date the closure in the zone the
//! runner started in. Rust integration tests cannot import one another, so the rig shapes are
//! copied from `debt_day_gates.rs`, never shared.
#![cfg(feature = "testkit")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use codotheca_core::advisories::lockfiles::read_lockfiles;
use codotheca_core::assembly::jobs::JobPump;
use codotheca_core::cancel::CancelToken;
use codotheca_core::clock::{day_bounds, LocalZone, SystemZone};
use codotheca_core::git::{GitSlots, SystemGit, TreeEntry};
use codotheca_core::health::switches::write_switches;
use codotheca_core::http::HttpResponse;
use codotheca_core::index::Index;
use codotheca_core::jobs::{run_one, Job, JobDeps, JobKind, JobOrigin, Priority};
use codotheca_core::mount::StoreClass;
use codotheca_core::proto::EventSink;
use codotheca_core::protocol::{
    DebtSource, HealthCheckSwitch, LocationId, ProjectId, SyncTaskKind, SyncTaskState,
};
use codotheca_core::provider::GitHubProvider;
use codotheca_core::sync::http::ObservingTransport;
use codotheca_core::sync::runner::SyncRunner;
use codotheca_core::sync::state::SyncTaskStateRow;
use codotheca_core::sync::store::{load, put};
use codotheca_core::sync::task::SyncTask;
use codotheca_core::sync::SyncDeps;
use codotheca_core::testing::zone::{FixedZone, TransitionZone};
use codotheca_core::testing::{FakeClock, FakeGitBackend, FakeTokenStore, FakeTransport, GitReply};
use rusqlite::Connection;
use support::TestRepo;

const DAY: i64 = 86_400;

/// The local day `D0` every transition below happens on, and the instant of 00:00 UTC on it.
const D0: i64 = 20_000;

// ---------------------------------------------------------------------------------------------
// `day_bounds` against the zone it is computed from
// ---------------------------------------------------------------------------------------------

/// A switch at 02:00 local time on `D0`, read in the offset before it.
fn transition(before_min: i32, after_min: i32) -> TransitionZone {
    TransitionZone {
        at: D0 * DAY + 2 * 3_600 - i64::from(before_min) * 60,
        before_min,
        after_min,
    }
}

/// Rules 1 and 2: for every instant sampled at 60 s over the three days around `D0`, its local
/// day is `d` exactly when it lies in `day_bounds(d)`. Returns the number of samples checked.
fn check_bounds(name: &str, zone: &dyn LocalZone, d0_length: i64) -> usize {
    let (start, end) = day_bounds(zone, D0);
    assert_eq!(
        end - start,
        d0_length,
        "{name}: day {D0} is {} s long",
        end - start
    );

    let mut checked = 0;
    let mut t = (D0 - 1) * DAY;
    while t < (D0 + 2) * DAY {
        let local = zone.at(t).day();
        for d in D0 - 2..=D0 + 2 {
            let (lo, hi) = day_bounds(zone, d);
            assert_eq!(
                local == d,
                (lo..hi).contains(&t),
                "{name}: instant {t} has local day {local}, but day_bounds({d}) = [{lo}, {hi})"
            );
        }
        checked += 1;
        t += 60;
    }
    checked
}

#[test]
fn day_bounds_is_the_set_of_instants_whose_local_day_it_is() {
    let cases: [(&str, Box<dyn LocalZone>, i64); 4] = [
        ("utc", Box::new(FixedZone(0)), 86_400),
        ("+05:30", Box::new(FixedZone(330)), 86_400),
        (
            "spring forward 0 → +60",
            Box::new(transition(0, 60)),
            82_800,
        ),
        ("fall back +60 → 0", Box::new(transition(60, 0)), 90_000),
    ];
    let mut samples = 0;
    for (name, zone, length) in &cases {
        samples += check_bounds(name, zone.as_ref(), *length);
    }
    eprintln!("day_bounds samples checked: {samples}");
    assert!(samples > 0, "no instant was checked");
}

/// The production zone reads the OS's zone at each instant, so it must agree with the local wall
/// clock chrono derives for that instant — whatever zone the host is in.
#[test]
fn system_zone_agrees_with_chrono() {
    let now = chrono::Utc::now().timestamp();
    for t in [now - 180 * DAY, now, now + 180 * DAY] {
        let utc = chrono::DateTime::from_timestamp(t, 0).unwrap();
        let wall = utc.with_timezone(&chrono::Local).naive_local();
        let expected = (wall - utc.naive_utc()).num_minutes();
        let reported = SystemZone.at(t);
        assert_eq!(reported.unix, t);
        assert_eq!(
            i64::from(reported.offset_min),
            expected,
            "SystemZone at {t} reported {} min, chrono's wall clock differs by {expected} min",
            reported.offset_min
        );
    }
}

// ---------------------------------------------------------------------------------------------
// The three payout sites across a transition
// ---------------------------------------------------------------------------------------------

/// The offset a runner starts in, and the one in force after the switch.
const BEFORE_MIN: i32 = 0;
const AFTER_MIN: i32 = 60;

/// A runner that starts at `start` crosses a switch an hour later, and closes at 23:30 UTC on
/// the next day but one: a local date of `D` before the switch and `D + 1` after it.
struct Crossing {
    zone: Arc<TransitionZone>,
    closure: i64,
}

impl Crossing {
    fn from(start: i64) -> Self {
        Self {
            zone: Arc::new(TransitionZone {
                at: start + 3_600,
                before_min: BEFORE_MIN,
                after_min: AFTER_MIN,
            }),
            closure: (start.div_euclid(DAY) + 2) * DAY - 1_800,
        }
    }

    /// The local date of the closure in `offset_min`, derived by chrono rather than by the helper
    /// under test.
    fn date_in(&self, offset_min: i32) -> String {
        chrono::DateTime::from_timestamp(self.closure + i64::from(offset_min) * 60, 0)
            .unwrap()
            .date_naive()
            .to_string()
    }
}

/// Every `debt_day` row: `(dedupe_key, tz_offset_min)`.
fn debt_days(conn: &Connection) -> BTreeSet<(String, Option<i64>)> {
    let mut stmt = conn
        .prepare("SELECT dedupe_key, tz_offset_min FROM xp_events WHERE kind = 'debt_day'")
        .unwrap();
    let rows = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    drop(stmt);
    rows
}

fn ledger(index: &Mutex<Index>) -> BTreeSet<(String, Option<i64>)> {
    let guard = index.lock().unwrap();
    let rows = debt_days(guard.conn());
    drop(guard);
    rows
}

#[derive(Debug, Default)]
struct NullSink;

impl EventSink for NullSink {
    fn emit(&self, topic: &str, event: &str, payload: serde_json::Value) {
        codotheca_core::testing::events::validated(topic, event, &payload);
    }
}

/// What one site paid after the switch, judged against the post-switch date.
struct Paid {
    site: &'static str,
    rows: Vec<(String, Option<i64>)>,
    post: String,
    pre: String,
}

impl Paid {
    fn problem(&self) -> Option<String> {
        assert_ne!(
            self.pre, self.post,
            "{}: the fixture does not cross a date",
            self.site
        );
        match self.rows.as_slice() {
            [(key, offset)]
                if key.starts_with("debt_day:")
                    && key.ends_with(&format!(":{}", self.post))
                    && *offset == Some(i64::from(AFTER_MIN)) =>
            {
                None
            }
            rows => Some(format!(
                "{} paid {rows:?}; expected one debt_day dated {} at offset {AFTER_MIN} \
                 (the pre-switch date is {})",
                self.site, self.post, self.pre
            )),
        }
    }
}

fn paid_since(
    site: &'static str,
    crossing: &Crossing,
    before: &BTreeSet<(String, Option<i64>)>,
    after: &BTreeSet<(String, Option<i64>)>,
) -> Paid {
    Paid {
        site,
        rows: after.difference(before).cloned().collect(),
        post: crossing.date_in(AFTER_MIN),
        pre: crossing.date_in(BEFORE_MIN),
    }
}

fn wait_until(what: &str, until: &dyn Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !until() {
        assert!(
            Instant::now() < deadline,
            "{what} did not happen within the wait"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
}

/// Site 1 — the scheduler's settle, through the production `JobPump::start` and one pool. A
/// `no_release` item opens before the switch and closes on `git tag v1` after it.
fn scheduler_site() -> Paid {
    const T0: i64 = 1_700_000_000;
    const OPEN_NO_RELEASE: &str =
        "SELECT count(*) FROM debt_item WHERE project_id = ?1 AND source = 'no_release'";
    const J15_SETTLED: &str = "SELECT count(*) FROM project_job_state
                                WHERE project_id = ?1 AND job = 'j1_5' AND state = 'ok'";

    let repo = TestRepo::init();
    // An entry point, so J3 classifies it `cli`, for which `release` applies.
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
    let index = Arc::new(Mutex::new(index));
    let count = |sql: &str| -> i64 {
        let guard = index.lock().unwrap();
        let n = guard
            .conn()
            .query_row(sql, [project.0], |r| r.get(0))
            .unwrap();
        drop(guard);
        n
    };

    let crossing = Crossing::from(T0);
    let clock = Arc::new(FakeClock::new(T0));
    let pump = JobPump::start(
        Arc::clone(&index),
        Arc::new(SystemGit::new(
            Arc::new(repo.exec()),
            Arc::new(GitSlots::new(4)),
            clock.clone(),
        )),
        clock.clone(),
        Arc::new(NullSink),
        crossing.zone.clone(),
    );
    pump.sink()
        .on_location_indexed(project, location, "store", StoreClass::Local);
    wait_until("the no_release item opening and J1.5 settling", &|| {
        count(OPEN_NO_RELEASE) == 1 && count(J15_SETTLED) == 1
    });
    // J1.5 has classified the fixture; set both columns back so the closure is paid.
    {
        let guard = index.lock().unwrap();
        guard
            .conn()
            .execute(
                "UPDATE project SET authored_by_user = 1, is_reference = 0 WHERE id = ?1",
                [project.0],
            )
            .unwrap();
        drop(guard);
    }

    let before = ledger(&index);
    clock.set_unix(crossing.closure);
    repo.git(&["tag", "v1"]);
    pump.sink()
        .on_location_indexed(project, location, "store", StoreClass::Local);
    wait_until("the release closing no_release", &|| {
        count(OPEN_NO_RELEASE) == 0
    });
    pump.stop();
    paid_since("scheduler", &crossing, &before, &ledger(&index))
}

/// Site 2 — J7's item build, through `run_one` with one `JobDeps` across both runs.
fn j7_site() -> Paid {
    const NOW: i64 = 1_800_000_000;
    let dir = tempfile::tempdir().unwrap();
    let mut index = Index::open_at(dir.path(), NOW).unwrap();
    let (project, location) = index
        .with_tx(|tx| {
            tx.execute(
                "INSERT INTO project (name, seed_basename, lineage_key, authored_by_user,
                                      is_reference, acknowledged_at, created_at, updated_at)
                 VALUES ('p', 'p', 'fixture', 1, 0, ?1, 0, 0)",
                [NOW - 1_000],
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
            Ok((project, LocationId(tx.last_insert_rowid())))
        })
        .unwrap();
    let index = Mutex::new(index);
    let git = Arc::new(FakeGitBackend::new());
    let tree = |head: &str, markers: usize| {
        let mut bytes = String::new();
        for i in 0..markers {
            let _ = writeln!(bytes, "// TODO: marker number {i}");
        }
        let oid = format!("{head:a>40}");
        git.script_blob(&oid, bytes.as_bytes());
        git.always_head_tree(GitReply::Ok(vec![TreeEntry {
            mode: "100644".to_owned(),
            kind: "blob".to_owned(),
            oid,
            path: b"src/a.rs".to_vec(),
        }]));
        let guard = index.lock().unwrap();
        guard
            .conn()
            .execute(
                "UPDATE location SET head_oid = ?2 WHERE id = ?1",
                rusqlite::params![location.0, head],
            )
            .unwrap();
        drop(guard);
    };

    let crossing = Crossing::from(NOW);
    let clock = Arc::new(FakeClock::new(NOW));
    let deps = JobDeps {
        git: git.clone(),
        clock: clock.clone(),
        cancel: CancelToken::new(),
        zone: crossing.zone.clone(),
    };
    let job = Job {
        kind: JobKind::J7Markers,
        project_id: project,
        location_id: location,
        store_key: "store-a".to_owned(),
        store_kind: StoreClass::Local,
        priority: Priority::Standard,
        not_before: 0,
        origin: JobOrigin::Walk,
    };
    let announce = RefCell::new(Vec::new());

    tree("head-one", 5);
    run_one(&index, &deps, &job, &announce).unwrap();
    let before = ledger(&index);
    clock.set_unix(crossing.closure);
    tree("head-two", 4);
    run_one(&index, &deps, &job, &announce).unwrap();
    paid_since("j7", &crossing, &before, &ledger(&index))
}

/// An enrolled, authored project with a lineage and one location row at `work`.
fn advisory_project(index: &Mutex<Index>, work: &std::path::Path, now: i64) -> ProjectId {
    let mut guard = index.lock().unwrap();
    let project = guard
        .with_tx(|tx| {
            tx.execute(
                "INSERT INTO project (name, seed_basename, lineage_key, acknowledged_at,
                                      authored_by_user, is_reference, created_at, updated_at)
                 VALUES ('alpha', 'alpha', 'alpha', ?1, 1, 0, 1, 1)",
                [now],
            )?;
            let project = ProjectId(tx.last_insert_rowid());
            let path = work.to_string_lossy().into_owned();
            tx.execute(
                "INSERT INTO location (project_id, kind, path_bytes, path_key,
                                       path_display, store_key, presence, repo_kind)
                 VALUES (?1, 'linux', ?2, ?2, ?3, 'store', 'present', 'worktree')",
                rusqlite::params![project.0, path.as_bytes(), path],
            )?;
            Ok(project)
        })
        .unwrap();
    drop(guard);
    project
}

/// Site 3 — the advisory settle, through one `SyncRunner` with scripted forge answers.
fn advisory_site() -> Paid {
    const NOW: i64 = 1_800_000_000;
    const HOST: &str = "forge.example.invalid";
    let dir = tempfile::tempdir().unwrap();
    let index = Arc::new(Mutex::new(Index::open_at(dir.path(), NOW).unwrap()));
    let work = tempfile::tempdir().unwrap();
    let project = advisory_project(&index, work.path(), NOW);
    let lock = |version: &str, at: i64| {
        std::fs::write(
            work.path().join("package-lock.json"),
            format!(
                r#"{{"lockfileVersion":3,"packages":{{"":{{}},"node_modules/left":{{"version":"{version}"}}}}}}"#
            ),
        )
        .unwrap();
        let mut guard = index.lock().unwrap();
        guard
            .with_tx(|tx| {
                read_lockfiles(tx, project, work.path(), at).unwrap();
                Ok(())
            })
            .unwrap();
        drop(guard);
    };
    let transport = Arc::new(FakeTransport::new());
    let answer = |body: &str| {
        transport.push(HttpResponse {
            status: 200,
            headers: codotheca_core::http::normalise_headers([("x-ratelimit-resource", "core")]),
            body: body.as_bytes().to_vec(),
        });
    };
    let open_items = || -> i64 {
        let guard = index.lock().unwrap();
        let n = guard
            .conn()
            .query_row(
                "SELECT count(*) FROM debt_item WHERE source = 'dependency_advisory'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        drop(guard);
        n
    };

    let crossing = Crossing::from(NOW);
    let clock = Arc::new(FakeClock::new(NOW));
    let observing = Arc::new(ObservingTransport::new(transport.clone(), clock.clone()));
    let runner = SyncRunner::new(
        Arc::clone(&index),
        SyncDeps {
            provider: Arc::new(GitHubProvider::new(observing.clone(), HOST.to_owned())),
            transport: observing,
            tokens: Arc::new(FakeTokenStore::available()),
            clock: clock.clone(),
            cancel: CancelToken::new(),
            zone: crossing.zone.clone(),
        },
        Arc::new(NullSink),
    );

    // One critical advisory against `left`, vulnerable below 2.0.0, fixed in 2.0.0.
    lock("1.0.0", NOW);
    {
        let mut guard = index.lock().unwrap();
        guard
            .with_tx(|tx| {
                put(
                    tx,
                    &SyncTaskStateRow::queued(SyncTaskKind::Advisories, None, NOW),
                )
            })
            .unwrap();
        drop(guard);
    }
    answer(
        r#"[{"ghsa_id":"GHSA-aaaa","cve_id":"CVE-2026-0001","summary":"s","html_url":"u",
             "severity":"critical","withdrawn_at":null,
             "identifiers":[{"value":"GHSA-aaaa","type":"GHSA"},{"value":"CVE-2026-0001","type":"CVE"}],
             "vulnerabilities":[{"package":{"ecosystem":"npm","name":"left"},
               "vulnerable_version_range":"< 2.0.0","first_patched_version":"2.0.0"}]}]"#,
    );
    runner.start();
    wait_until("the first advisory sweep settling", &|| {
        let guard = index.lock().unwrap();
        let state = load(guard.conn(), SyncTaskKind::Advisories, None)
            .unwrap()
            .map(|r| r.state);
        drop(guard);
        state == Some(SyncTaskState::Ok) && open_items() == 1
    });

    // The user upgrades past the range after the switch; the forge has nothing more to say.
    let before = ledger(&index);
    clock.set_unix(crossing.closure);
    lock("2.0.0", crossing.closure);
    answer("[]");
    runner.enqueue(SyncTask::Advisories);
    wait_until("the upgrade closing the advisory", &|| {
        open_items() == 0 && ledger(&index).len() > before.len()
    });
    runner.request_stop();
    runner.join();
    paid_since("advisory", &crossing, &before, &ledger(&index))
}

/// **`AC-P4-39-10`.** Through each production payout site, a closure after a DST switch — at an
/// instant whose local date differs between the offsets either side of it — writes
/// `debt_day:<subject>:<post-switch date>` with `tz_offset_min` the post-switch offset.
#[test]
fn ac_p4_39_10_debt_day_dates_per_instant() {
    let sites: [fn() -> Paid; 3] = [scheduler_site, j7_site, advisory_site];
    let mut failed = Vec::new();
    let mut executed = 0;
    for site in sites {
        let paid = site();
        eprintln!(
            "{}: paid {:?} (post-switch date {}, pre-switch {})",
            paid.site, paid.rows, paid.post, paid.pre
        );
        failed.extend(paid.problem());
        executed += 1;
    }
    eprintln!("AC-P4-39-10 payout sites executed: {executed}");
    assert_eq!(executed, 3, "a payout site did not run");
    assert_eq!(
        failed,
        Vec::<String>::new(),
        "a site dated a closure in the wrong zone"
    );
}
