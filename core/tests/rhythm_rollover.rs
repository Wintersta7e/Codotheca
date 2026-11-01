//! §39.7's two production callers of the settlement: every due pump pass, and core start. A
//! Monday settles the finished week at the first due pass after **local** midnight, a late
//! `commit_day` raises its settled week at the next due pass, and an idle pass writes nothing.

#![cfg(feature = "testkit")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::cell::RefCell;
use std::path::Path;
use std::sync::Arc;

use codotheca_core::assembly::startup::run_startup;
use codotheca_core::assembly::{CoreDeps, CoreHandler};
use codotheca_core::cancel::CancelToken;
use codotheca_core::clock::{local_date, Clock as _, LocalZone};
use codotheca_core::git::{Authorship, CommitterTally, RootCommit};
use codotheca_core::index::rebuild::{rebuild_in_place, RebuildOutcome};
use codotheca_core::index::Index;
use codotheca_core::jobs::{run_one, Job, JobDeps, JobKind, JobOrigin, Priority};
use codotheca_core::mount::StoreClass;
use codotheca_core::proto::dispatch::CommandHandler as _;
use codotheca_core::proto::pubsub::{Publisher, PublisherSink};
use codotheca_core::protocol::{LocationId, ProjectId, RhythmTargetBasis, Topic};
use codotheca_core::rings::rhythm::SettleReport;
use codotheca_core::rings::targets::{rhythm_target, KEY_RHYTHM_TARGET, KEY_RHYTHM_TARGET_BASIS};
use codotheca_core::session::DEFAULT_TICK_SECS;
use codotheca_core::testing::zone::{FixedZone, TransitionZone};
use codotheca_core::testing::{FakeClock, FakeGitBackend, GitReply};

const DAY: i64 = 86_400;
/// Monday 2026-10-12: the week the rollover finishes. Its Sunday is `M + 6`.
const M: i64 = 20_738;
const ME: &str = "me@example.invalid";

fn tick() -> i64 {
    i64::try_from(DEFAULT_TICK_SECS).unwrap()
}

/// One settled week with no active day, `missed` against `target`.
fn seed(index: &Index, monday: i64, target: u32) {
    index
        .conn()
        .execute(
            "INSERT INTO rhythm_week
                (week_start, active_days, target, verdict, git_coverage, settled_at)
             VALUES (?1, 0, ?2, 'missed', 'complete', 1)",
            rusqlite::params![local_date(monday), target],
        )
        .unwrap();
}

/// The weeks up to two weeks before the Monday after `M`, settled under a derived `target`.
fn seeded(index: &Index, target: u32) {
    seed(index, M - 14, target);
    seed(index, M - 7, target);
    for (k, v) in [
        (KEY_RHYTHM_TARGET, target.to_string()),
        (KEY_RHYTHM_TARGET_BASIS, "derived".to_owned()),
    ] {
        index
            .conn()
            .execute("INSERT INTO app_meta (k, v) VALUES (?1, ?2)", [k, &v])
            .unwrap();
    }
}

/// A project with no copy: the library is not empty, and no history is pending.
fn copyless(index: &Index) {
    index
        .conn()
        .execute(
            "INSERT INTO project (name, seed_basename, lineage_key, authored_by_user,
                                  is_reference, created_at, updated_at)
             VALUES ('q', 'q', 'lin-q', 1, 0, 0, 0)",
            [],
        )
        .unwrap();
}

/// A `CoreHandler` over `index` in `zone`, with fakes for everything the settlement does not read;
/// the sink and the clock come back so a test can watch `core/error` and move time.
fn handler(
    dir: &Path,
    index: Index,
    now: i64,
    zone: Arc<dyn LocalZone>,
) -> (CoreHandler, Arc<PublisherSink>, Arc<FakeClock>) {
    let index = Arc::new(std::sync::Mutex::new(index));
    let clock = Arc::new(FakeClock::new(now));
    let events = Arc::new(PublisherSink::new(Publisher::detached()));
    let http: Arc<dyn codotheca_core::http::HttpTransport> =
        Arc::new(codotheca_core::testing::FakeTransport::new());
    let sync_observing = Arc::new(codotheca_core::sync::http::ObservingTransport::new(
        Arc::clone(&http),
        clock.clone(),
    ));
    let home = dir.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let h = CoreHandler::new(CoreDeps {
        index: Arc::clone(&index),
        provider: Arc::new(codotheca_core::provider::GitHubProvider::new(
            Arc::clone(&http),
            codotheca_core::provider::listing::GITHUB_CANONICAL_HOST.to_owned(),
        )),
        tokens: Arc::new(codotheca_core::testing::FakeTokenStore::unavailable()),
        http,
        client_id: "test-client-id".to_owned(),
        clock: clock.clone(),
        git: Arc::new(FakeGitBackend::new()),
        write_git: Arc::new(codotheca_core::testing::FakeMutatingGit::new(
            codotheca_core::testing::CloneBehaviour::Succeed,
        )),
        trash: Arc::new(codotheca_core::testing::CountingTrash::new()),
        mount: Arc::new(codotheca_core::testing::FakeMountResolver::default()),
        spawner: Box::new(codotheca_core::launch::spawn::RecordingSpawner::new()),
        sessions: codotheca_core::session::manager::SessionManager::new(
            clock.clone(),
            events.clone(),
            Box::new(codotheca_core::session::watch::FakeActivitySource::new()),
            Arc::new(codotheca_core::session::activity::FakeIgnoreCheck::new(&[])),
            Arc::clone(&zone),
        ),
        scans: codotheca_core::scan::ScanSupervisor::new(Arc::new(
            codotheca_core::testing::ScanLauncherFake::new(),
        )),
        scan_store: Arc::new(codotheca_core::testing::MemScanStore::new()),
        firstrun: codotheca_core::firstrun::FirstRunEnv {
            sources: codotheca_core::firstrun::sources::SourceEnv {
                home,
                app_data: None,
                xdg_config: None,
            },
            classifier: Arc::new(codotheca_core::firstrun::classify::FixedClassifier::new(
                vec![],
            )),
            distros: Arc::new(codotheca_core::firstrun::classify::NoDistros),
            platform: codotheca_core::index::path::PathPlatform::Unix,
            skip: codotheca_core::scan::skiplist::SkipList::default(),
            cache: codotheca_core::firstrun::roots::SuggestionCache::new(),
        },
        jobs: codotheca_core::assembly::jobs::JobPump::start(
            Arc::clone(&index),
            Arc::new(FakeGitBackend::new()),
            clock.clone(),
            events.clone(),
            Arc::clone(&zone),
        ),
        sync: codotheca_core::assembly::sync::SyncPump::start(
            Arc::clone(&index),
            codotheca_core::sync::SyncDeps {
                provider: Arc::new(codotheca_core::provider::GitHubProvider::new(
                    sync_observing.clone(),
                    codotheca_core::provider::listing::GITHUB_CANONICAL_HOST.to_owned(),
                )),
                transport: sync_observing,
                tokens: Arc::new(codotheca_core::testing::FakeTokenStore::unavailable()),
                clock: clock.clone(),
                cancel: CancelToken::new(),
                zone: Arc::clone(&zone),
            },
            events.clone(),
        ),
        events: Arc::clone(&events),
        tz_offset_min: 0,
        zone,
    });
    events.with(|p| p.subscribe(Topic::Core));
    (h, events, clock)
}

/// Every settled week as `(week_start, active_days, verdict, settled_at)`, oldest first.
fn weeks(h: &CoreHandler) -> Vec<(String, i64, String, i64)> {
    let guard = h.index().lock().unwrap();
    let rows = guard
        .conn()
        .prepare(
            "SELECT week_start, active_days, verdict, settled_at FROM rhythm_week
              ORDER BY week_start",
        )
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    drop(guard);
    rows
}

fn total_changes(h: &CoreHandler) -> u64 {
    h.index().lock().unwrap().conn().total_changes()
}

/// The sequence the `core` topic has reached; it moves on every `core` event, `core/error`
/// included.
fn core_seq(events: &PublisherSink) -> Option<u64> {
    events.with(|p| p.resync(Topic::Core))
}

/// One rollover over `zone`: Sunday `M + 6` began at local midnight `previous` and ends at local
/// midnight `midnight`.
fn rollover(name: &str, zone: Arc<dyn LocalZone>, previous: i64, midnight: i64) {
    let dir = tempfile::tempdir().unwrap();
    let start = midnight - 10;
    let index = Index::open_at(dir.path(), start).unwrap();
    seeded(&index, 3);
    copyless(&index);
    let (mut h, events, clock) = handler(dir.path(), index, start, zone);

    // Sunday 23:59:50 local: the week ending tonight is not complete yet.
    h.pump();
    assert_eq!(weeks(&h).len(), 2, "{name}: a week settled before it ended");

    // One-second steps, as finely as `run_loop` could ever wake, until the week settles.
    let mut steps = 0;
    while weeks(&h).len() == 2 {
        clock.advance(1);
        h.pump();
        steps += 1;
        assert!(
            steps <= 2 * tick(),
            "{name}: no settlement within two pump periods"
        );
    }
    let at = clock.now_unix();
    let settled = weeks(&h);
    let row = &settled[2];
    let a_day_later = previous + DAY;
    eprintln!(
        "{name}: inserted {} at {at}, {}s after local midnight {midnight} (24 h after the \
         previous midnight is {a_day_later})",
        row.0,
        at - midnight,
    );
    assert_eq!(row.0, local_date(M), "{name}: the finished week settled");
    assert_eq!(row.3, at, "{name}: settled at the pass that inserted it");
    assert!(
        (midnight..=midnight + tick()).contains(&at),
        "{name}: inserted at {at}, not within one pump period of local midnight {midnight}"
    );

    // Ten further due passes: nothing to do, nothing written, nothing reported.
    let (changes, seq, ticks) = (total_changes(&h), core_seq(&events), h.ticks());
    for _ in 0..10 {
        clock.advance(tick());
        h.pump();
    }
    assert_eq!(h.ticks(), ticks + 10, "{name}: ten due passes ran");
    assert_eq!(weeks(&h), settled, "{name}: an idle pass inserted a row");
    assert_eq!(total_changes(&h), changes, "{name}: an idle pass wrote");
    assert_eq!(
        core_seq(&events),
        seq,
        "{name}: an idle pass emitted on core"
    );
}

#[test]
fn ac_p4_39_10_the_monday_rollover_settles_once() {
    let sunday = (M + 6) * DAY;
    rollover("utc", Arc::new(FixedZone(0)), sunday, sunday + DAY);
    // The clocks spring forward at 02:00 local on that Sunday, UTC+1 to UTC+2: a 23-hour day,
    // whose local midnight is 22:00 UTC, an hour before 24 h after the previous one.
    rollover(
        "spring forward",
        Arc::new(TransitionZone {
            at: sunday + 3_600,
            before_min: 60,
            after_min: 120,
        }),
        sunday - 3_600,
        sunday + DAY - 2 * 3_600,
    );
}

/// `rhythm_week` and its target as the index holds them.
fn stored(index: &Index) -> (Vec<String>, Option<(u32, RhythmTargetBasis)>) {
    let rows = index
        .conn()
        .prepare(
            "SELECT week_start || '|' || active_days || '|' || target || '|' || verdict || '|'
                    || git_coverage || '|' || settled_at
               FROM rhythm_week ORDER BY week_start",
        )
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    (rows, rhythm_target(index.conn()).unwrap())
}

#[test]
fn startup_settles_after_the_restore() {
    // Wednesday noon of week `M`: the seeded weeks run to the last complete one.
    let now = (M + 2) * DAY + 12 * 3_600;
    let dir = tempfile::tempdir().unwrap();
    let index = Index::open_at(dir.path(), now - 100).unwrap();
    // A derived target no ledger here implies: a re-derivation would answer the fallback.
    seeded(&index, 4);
    let exported = stored(&index);
    index.export_sidecar(now - 50).unwrap();
    drop(index);

    std::fs::write(Index::db_path(dir.path()), b"this is not a database").unwrap();
    match rebuild_in_place(dir.path(), now - 40) {
        Ok(RebuildOutcome::Rebuilt(report)) => {
            eprintln!("rebuilt: restored {:?}", report.restored);
            assert_eq!(report.restored.get("rhythm_week").copied(), Some(2));
        }
        other => panic!("expected a rebuild, got {other:?}"),
    }
    let rebuilt = Index::open_at(dir.path(), now - 30).unwrap();
    let restored = stored(&rebuilt);
    assert_eq!(
        restored, exported,
        "the restore brought the weeks and the target back"
    );
    // The scan has indexed a project, so the start-up pass runs rather than idling on none.
    copyless(&rebuilt);

    let (mut h, _events, _clock) = handler(dir.path(), rebuilt, now, Arc::new(FixedZone(0)));
    let summary = run_startup(&mut h);
    eprintln!("startup: {:?}", summary.rhythm);
    assert_eq!(summary.rhythm, Some(SettleReport::default()));
    let after = stored(&h.index().lock().unwrap());
    assert_eq!(
        after, restored,
        "the start re-derived or rewrote a restored week"
    );
    assert_eq!(after.1, Some((4, RhythmTargetBasis::Derived)));
}

/// One project of the user's on one present copy, the user's address confirmed.
fn authored(index: &mut Index) -> (ProjectId, LocationId) {
    index
        .with_tx(|tx| {
            tx.execute(
                "INSERT INTO project (name, seed_basename, lineage_key, authored_by_user,
                                      is_reference, created_at, updated_at)
                 VALUES ('p', 'p', 'lin', 1, 0, 0, 0)",
                [],
            )?;
            let project = ProjectId(tx.last_insert_rowid());
            tx.execute(
                "INSERT INTO location (project_id, kind, path_bytes, path_key, path_display,
                                       store_key, presence, repo_kind)
                 VALUES (?1, 'linux', ?2, ?2, ?3, 'store', 'present', 'worktree')",
                rusqlite::params![project.0, b"/rig/late".as_slice(), "/rig/late"],
            )?;
            let location = LocationId(tx.last_insert_rowid());
            tx.execute(
                "INSERT INTO identity (email, is_user, source, confirmed_at)
                 VALUES (?1, 1, 'manual', 1)",
                [ME],
            )?;
            Ok((project, location))
        })
        .unwrap()
}

/// J4 over a fake history in which the user committed on `days`, through the production `run_one`.
fn j4(h: &CoreHandler, clock: &Arc<FakeClock>, place: (ProjectId, LocationId), days: &[i64]) {
    let last = days.iter().max().unwrap() * DAY + DAY / 2;
    let git = FakeGitBackend::new();
    git.always_root_commits(GitReply::Ok(vec![RootCommit {
        oid: "a".repeat(40),
        committed_at: last,
        tz_offset_min: 0,
    }]))
    .always_authorship(GitReply::Ok(Authorship {
        committers: vec![CommitterTally {
            email: ME.to_owned(),
            commits: u32::try_from(days.len()).unwrap(),
            days: days.iter().copied().collect(),
            last_commit_at: last,
        }],
    }))
    .always_commit_subjects(GitReply::Ok(Vec::new()))
    .always_release_tags(GitReply::Ok(Vec::new()));
    let deps = JobDeps {
        git: Arc::new(git),
        clock: clock.clone(),
        cancel: CancelToken::new(),
        zone: Arc::new(FixedZone(0)),
    };
    let job = Job {
        kind: JobKind::J4History,
        project_id: place.0,
        location_id: place.1,
        store_key: "store".to_owned(),
        store_kind: StoreClass::Local,
        priority: Priority::Interactive,
        not_before: 0,
        origin: JobOrigin::Walk,
    };
    run_one(h.index(), &deps, &job, &RefCell::new(Vec::new())).unwrap();
}

#[test]
fn a_rebuilt_index_settles_no_week_before_a_project_is_indexed() {
    // Wednesday noon two weeks after week `M`: weeks `M` and `M + 7` are complete and unsettled.
    let now = (M + 16) * DAY + 12 * 3_600;
    let dir = tempfile::tempdir().unwrap();
    let index = Index::open_at(dir.path(), now - 100).unwrap();
    seeded(&index, 2);
    index.export_sidecar(now - 50).unwrap();
    drop(index);

    std::fs::write(Index::db_path(dir.path()), b"this is not a database").unwrap();
    assert!(matches!(
        rebuild_in_place(dir.path(), now - 40),
        Ok(RebuildOutcome::Rebuilt(_))
    ));
    let rebuilt = Index::open_at(dir.path(), now - 30).unwrap();
    let projects: i64 = rebuilt
        .conn()
        .query_row("SELECT count(*) FROM project", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        projects, 0,
        "the rebuilt index holds no project until a scan"
    );
    let restored = stored(&rebuilt);

    // Core start runs before any scan: with no project, no history has been read, so a week
    // settled now would claim a complete, empty history.
    let (mut h, _events, clock) = handler(dir.path(), rebuilt, now, Arc::new(FixedZone(0)));
    let summary = run_startup(&mut h);
    eprintln!("startup over no project: {:?}", summary.rhythm);
    assert_eq!(
        stored(&h.index().lock().unwrap()),
        restored,
        "a week settled over an index that holds no project"
    );
    assert_eq!(summary.rhythm, Some(SettleReport::default()));

    // The scan indexes the user's project and J4 reads its history; the next due pass settles
    // the missing weeks with their real active days.
    let place = authored(&mut h.index().lock().unwrap());
    j4(&h, &clock, place, &[M, M + 1, M + 7]);
    // The runner's settle row for the job: the project's history is no longer pending.
    h.index()
        .lock()
        .unwrap()
        .conn()
        .execute(
            "INSERT INTO project_job_state (project_id, job, state, at) VALUES (?1, 'j4', 'ok', 1)",
            [place.0 .0],
        )
        .unwrap();
    clock.advance(tick());
    h.pump();
    let settled_at = now + tick();
    let after = stored(&h.index().lock().unwrap());
    eprintln!("after the scan's history: {:?}", after.0);
    assert_eq!(
        after.0,
        [
            restored.0[0].clone(),
            restored.0[1].clone(),
            format!("{}|2|2|counted|complete|{settled_at}", local_date(M)),
            format!("{}|1|2|missed|complete|{settled_at}", local_date(M + 7)),
        ],
        "the first pass after the history was read settles the missing weeks from it"
    );
}

#[test]
fn a_late_commit_day_is_raised_within_one_pump_period() {
    let now = (M + 2) * DAY + 12 * 3_600;
    let dir = tempfile::tempdir().unwrap();
    let mut index = Index::open_at(dir.path(), now).unwrap();
    let place = authored(&mut index);
    seeded(&index, 3);
    let (mut h, _events, clock) = handler(dir.path(), index, now, Arc::new(FixedZone(0)));

    h.pump();
    let before = weeks(&h);
    assert_eq!((before[1].1, before[1].2.as_str()), (0, "missed"));
    let mark = h.rhythm_raise_mark();

    // Three days of the settled week `M - 7`, written by J4 after it settled.
    j4(&h, &clock, place, &[M - 7, M - 6, M - 5]);
    let written: i64 = h
        .index()
        .lock()
        .unwrap()
        .conn()
        .query_row(
            "SELECT count(*) FROM xp_events WHERE kind = 'commit_day'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    eprintln!("J4 wrote {written} commit days; raise mark {mark}");
    assert_eq!(written, 3, "J4 wrote the late commit days");

    clock.advance(tick());
    h.pump();
    let after = weeks(&h);
    eprintln!("after the next due pass: {:?}", after[1]);
    assert_eq!(
        (after[1].0.as_str(), after[1].1, after[1].2.as_str()),
        (local_date(M - 7).as_str(), 3, "counted"),
        "the late commit days raised the settled week at the next due pass"
    );
    assert_eq!(after[0], before[0], "a week with no late day moved");
    assert!(
        h.rhythm_raise_mark() > mark,
        "the pass read past the new rows"
    );
}
