//! §38.5–§38.6: the one level reading, its three states, and the floor only a confirmed, computed
//! level raises.
//!
//! `AC-P4-38-7`: unknown is never rendered as zero — `notComputed` carries no number at all, an
//! upgraded index reads `partial` until its eligible histories settle, and a routine rescan never
//! makes a computed library partial again. `AC-P4-38-9`: the floor's gate — it rises only when
//! the reading is `computed` and the identity card has been answered.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use codotheca_core::cancel::CancelToken;
use codotheca_core::firstrun::{dispatch, FirstRunEnv};
use codotheca_core::git::{GitSlots, SystemGit};
use codotheca_core::index::migrate::{apply_all, MIGRATIONS};
use codotheca_core::index::{open_connection, Index};
use codotheca_core::jobs::j15_authorship::{persist, AuthorshipFacts};
use codotheca_core::jobs::scheduler::JobRunner;
use codotheca_core::jobs::{Job, JobDeps, JobKind, JobOrigin, Priority};
use codotheca_core::ledger::curve::{level_of, threshold};
use codotheca_core::ledger::floor::{identity_answered, read_floor};
use codotheca_core::ledger::level::{pending_projects, read_level};
use codotheca_core::ledger::xp::xp_by_term;
use codotheca_core::mount::StoreClass;
use codotheca_core::proto::EventSink;
use codotheca_core::protocol::{LevelReading, LevelState, LocationId, ProjectId};
use codotheca_core::testing::FakeClock;
use rusqlite::Connection;
use serde_json::{json, Value};
use support::TestRepo;

const T0: i64 = 1_700_000_000;
const DAY: i64 = 86_400;

/// The committer `TestRepo` writes as, so J1.5 keeps a fixture repository on the user's side.
const FIXTURE: &str = "fixture@example.invalid";
const A: &str = "a@example.invalid";
const B: &str = "b@example.invalid";

#[derive(Debug, Default)]
struct NullSink;

impl EventSink for NullSink {
    fn emit(&self, topic: &str, event: &str, payload: Value) {
        codotheca_core::testing::events::validated(topic, event, &payload);
    }
}

/// One reading through the production producer, in its own transaction.
fn read(index: &mut Index, now: i64) -> LevelReading {
    index.with_tx(|tx| read_level(tx, now)).unwrap()
}

/// The floor's one row: `(level, raised_at)`.
fn floor_row(conn: &Connection) -> (i64, Option<i64>) {
    conn.query_row(
        "SELECT level, raised_at FROM level_floor WHERE id = 1",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .unwrap()
}

fn project(conn: &Connection, name: &str) -> i64 {
    conn.execute(
        "INSERT INTO project (name, seed_basename, lineage_key, created_at, updated_at)
         VALUES (?1, ?1, ?1, 0, 0)",
        [name],
    )
    .unwrap();
    conn.last_insert_rowid()
}

/// One copy of `project` at `path`, with its own `presence`.
fn location(conn: &Connection, project: i64, path: &str, presence: &str) -> i64 {
    conn.execute(
        "INSERT INTO location (project_id, kind, path_bytes, path_key, path_display, store_key,
                               presence, repo_kind)
         VALUES (?1, 'linux', ?2, ?2, ?3, 'store', ?4, 'worktree')",
        rusqlite::params![project, path.as_bytes(), path, presence],
    )
    .unwrap();
    conn.last_insert_rowid()
}

/// Writes `project`'s `job` row in `state`, whatever it held.
fn job(conn: &Connection, project: i64, job: &str, state: &str) {
    conn.execute(
        "INSERT INTO project_job_state (project_id, job, state, at) VALUES (?1, ?2, ?3, ?4)
           ON CONFLICT (project_id, job) DO UPDATE SET state = excluded.state",
        rusqlite::params![project, job, state, T0],
    )
    .unwrap();
}

/// `n` commit-days on `n` consecutive days ending at `last_day`, so no date cap or revival
/// gap bites.
fn commit_days(conn: &Connection, project: i64, last_day: i64, n: i64) {
    for i in 0..n {
        conn.execute(
            "INSERT INTO xp_events (ts, tz_offset_min, project_id, subject_key, kind, dedupe_key,
                                    track)
             VALUES (?1, 0, ?2, 's', 'commit_day', ?3, 'git')",
            rusqlite::params![
                (last_day - i) * DAY,
                project,
                format!("commit_day:{project}:{i}")
            ],
        )
        .unwrap();
    }
}

/// `n` completion latches on `project`, the retained kind no cap touches.
fn completion_checks(conn: &Connection, project: i64, n: i64) {
    for i in 0..n {
        conn.execute(
            "INSERT INTO xp_events (ts, tz_offset_min, project_id, subject_key, kind, dedupe_key,
                                    track)
             VALUES (?1, 0, ?2, 's', 'completion_check', ?3, 'session')",
            rusqlite::params![T0, project, format!("completion_check:{project}:x{i}")],
        )
        .unwrap();
    }
}

fn identity(conn: &Connection, email: &str) {
    conn.execute(
        "INSERT INTO identity (email, is_user, source) VALUES (?1, 1, 'gitconfig')",
        [email],
    )
    .unwrap();
}

/// What the XP function and the curve say right now, with no floor.
fn earned(conn: &Connection) -> (i64, u32) {
    let xp = xp_by_term(conn).unwrap().total();
    (xp, level_of(xp))
}

/// `xp < levelAt` happens only when floored, and the thresholds are the curve's.
fn assert_consistent(reading: &LevelReading) {
    let (Some(level), Some(xp), Some(at), Some(next)) = (
        reading.level,
        reading.xp,
        reading.level_at,
        reading.next_level_at,
    ) else {
        panic!("a computed or partial reading carries every number: {reading:?}");
    };
    assert_eq!(at, threshold(level), "levelAt is T(level)");
    assert_eq!(next, threshold(level + 1), "nextLevelAt is T(level + 1)");
    assert_eq!(
        xp < at,
        reading.floored,
        "xp sits below levelAt exactly when the floor holds the level: {reading:?}"
    );
}

// ---------------------------------------------------------------------------------------------
// AC-P4-38-7 — three states, never a zero.
// ---------------------------------------------------------------------------------------------

/// A database at exactly `version` files applied.
fn at_version(dir: &Path, version: usize) -> Connection {
    let mut conn = open_connection(&Index::db_path(dir)).unwrap();
    apply_all(&mut conn, MIGRATIONS.get(..version).unwrap()).unwrap();
    conn
}

/// J1 for every `(project, location)` on a fresh real `JobRunner` over real git, spun until
/// `until` holds — never a bare sleep — then stopped. `observe` sees the index on every spin.
fn settle(
    index: &Arc<Mutex<Index>>,
    git: &TestRepo,
    now: i64,
    jobs: &[(i64, i64)],
    until: &dyn Fn(&Connection) -> bool,
    observe: &mut dyn FnMut(&mut Index),
) {
    let clock = Arc::new(FakeClock::new(now));
    let deps = JobDeps {
        git: Arc::new(SystemGit::new(
            Arc::new(git.exec()),
            Arc::new(GitSlots::new(4)),
            clock.clone(),
        )),
        clock,
        cancel: CancelToken::new(),
        zone: Arc::new(codotheca_core::testing::zone::FixedZone(0)),
    };
    let runner = JobRunner::new(Arc::clone(index), deps, Arc::new(NullSink));
    for &(project, location) in jobs {
        assert!(runner.enqueue(Job {
            kind: JobKind::J1Refstate,
            project_id: ProjectId(project),
            location_id: LocationId(location),
            store_key: "store".to_owned(),
            store_kind: StoreClass::Local,
            priority: Priority::Interactive,
            not_before: 0,
            origin: JobOrigin::Walk,
        }));
    }
    runner.start(1);
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut done = false;
    while !done && Instant::now() < deadline {
        let mut guard = index.lock().unwrap();
        observe(&mut guard);
        done = until(guard.conn());
        drop(guard);
        if !done {
            std::thread::sleep(Duration::from_millis(25));
        }
    }
    runner.request_stop();
    runner.join();
    assert!(done, "the histories did not settle within the deadline");
}

/// Every `j4` row of `projects` reads `ok`, and was written at `at`.
fn j4_settled_at(conn: &Connection, projects: &[i64], at: i64) -> bool {
    projects.iter().all(|p| {
        conn.query_row(
            "SELECT count(*) FROM project_job_state
              WHERE project_id = ?1 AND job = 'j4' AND state = 'ok' AND at = ?2",
            rusqlite::params![p, at],
            |r| r.get::<_, i64>(0),
        )
        .unwrap()
            == 1
    })
}

fn repo() -> TestRepo {
    let repo = TestRepo::init();
    repo.write("main.py", b"print('one')\n");
    repo.commit("first");
    repo
}

/// A new install: one project found, its history not yet walked, and an empty ledger.
fn a_new_install_reads_no_number() {
    let fresh = tempfile::tempdir().unwrap();
    let mut index = Index::open_at(fresh.path(), T0).unwrap();
    let p = project(index.conn(), "found");
    location(index.conn(), p, "/found", "present");
    let reading = read(&mut index, T0);
    eprintln!("new install: {reading:?}");
    assert_eq!(reading.state, LevelState::NotComputed);
    assert_eq!(reading.pending_projects, 1);
    assert!(
        !reading.floored,
        "nothing is floored before anything is computed"
    );
    let wire = serde_json::to_value(&reading).unwrap();
    for field in ["level", "xp", "levelAt", "nextLevelAt"] {
        assert_eq!(wire[field], Value::Null, "{field} is absent, never 0 or 1");
    }
    assert_eq!(wire["state"], json!("notComputed"));
    assert_eq!(floor_row(index.conn()), (1, None), "no reading raised it");
}

/// An upgraded index: seeded at 18 with finished histories and a ledger, then migrated, which
/// re-queues every finished J4. One project per repository is eligible; an offline one and a
/// Reference one are queued too and are not. Returns the eligible `(project, location)`s.
fn upgraded_library(dir: &Path, repos: &[TestRepo]) -> (Index, Vec<(i64, i64)>) {
    let (eligible, offline, reference) = {
        let conn = at_version(dir, 18);
        identity(&conn, FIXTURE);
        let mut eligible = Vec::new();
        for (i, r) in repos.iter().enumerate() {
            let p = project(&conn, &format!("mine-{i}"));
            let l = location(&conn, p, &r.path().to_string_lossy(), "present");
            eligible.push((p, l));
        }
        let offline = project(&conn, "on-a-drive");
        location(&conn, offline, "/unplugged", "offline");
        let reference = project(&conn, "theirs");
        location(&conn, reference, "/theirs", "present");
        conn.execute(
            "UPDATE project SET is_reference = 1 WHERE id = ?1",
            [reference],
        )
        .unwrap();
        for p in eligible.iter().map(|&(p, _)| p).chain([offline, reference]) {
            for j in ["j1", "j1_5", "j4"] {
                job(&conn, p, j, "ok");
            }
            commit_days(&conn, p, T0 / DAY - 10 * p, 5);
        }
        (eligible, offline, reference)
    };
    let mut index = Index::open_at(dir, T0).unwrap();
    let queued: i64 = index
        .conn()
        .query_row(
            "SELECT count(*) FROM project_job_state WHERE job = 'j4' AND state = 'queued'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(queued, 4, "the upgrade re-queues every finished history");
    let reading = read(&mut index, T0);
    eprintln!("upgraded: {reading:?} (offline project {offline}, Reference {reference})");
    assert_eq!(reading.state, LevelState::Partial);
    assert_eq!(
        reading.pending_projects,
        u32::try_from(eligible.len()).unwrap(),
        "only the eligible queued projects are pending — not the offline one, not Reference"
    );
    assert_eq!(
        pending_projects(index.conn()).unwrap(),
        reading.pending_projects
    );
    assert_consistent(&reading);
    (index, eligible)
}

#[test]
fn ac_p4_38_7_three_states_never_a_zero() {
    a_new_install_reads_no_number();
    let repos = [repo(), repo()];
    let upgraded = tempfile::tempdir().unwrap();
    let (index, eligible) = upgraded_library(upgraded.path(), &repos);

    // J4 settles through the real scheduler: computed.
    let index = Arc::new(Mutex::new(index));
    let ids: Vec<i64> = eligible.iter().map(|&(p, _)| p).collect();
    settle(
        &index,
        &repos[0],
        T0,
        &eligible,
        &|conn| j4_settled_at(conn, &ids, T0),
        &mut |_| {},
    );
    let mut guard = index.lock().unwrap();
    let reading = read(&mut guard, T0);
    eprintln!("settled: {reading:?}");
    assert_eq!(reading.state, LevelState::Computed);
    assert_eq!(reading.pending_projects, 0);
    assert_consistent(&reading);

    // A concluded failure is concluded: a failed J4, or a failure earlier on its chain, does
    // not hold the reading partial.
    job(guard.conn(), ids[0], "j4", "queued");
    assert_eq!(read(&mut guard, T0).state, LevelState::Partial);
    job(guard.conn(), ids[0], "j4", "failed");
    assert_eq!(read(&mut guard, T0).state, LevelState::Computed);
    job(guard.conn(), ids[0], "j4", "queued");
    job(guard.conn(), ids[0], "j1_5", "deferred_slow");
    assert_eq!(read(&mut guard, T0).state, LevelState::Computed);
    job(guard.conn(), ids[0], "j1_5", "ok");
    job(guard.conn(), ids[0], "j4", "ok");
    assert_eq!(read(&mut guard, T0).state, LevelState::Computed);
    drop(guard);

    // A routine rescan walks J1 → J1.5 → J4 again and never makes the library partial.
    let later = T0 + 3_600;
    let mut seen = Vec::new();
    settle(
        &index,
        &repos[0],
        later,
        &eligible,
        &|conn| j4_settled_at(conn, &ids, later),
        &mut |during| seen.push(read(during, later).state),
    );
    eprintln!("states read during the rescan: {}", seen.len());
    assert!(!seen.is_empty(), "the rescan was never observed");
    assert_eq!(
        seen.iter()
            .filter(|s| **s != LevelState::Computed)
            .collect::<Vec<_>>(),
        Vec::<&LevelState>::new(),
        "a routine rescan flipped a computed library"
    );
    let mut after = index.lock().unwrap();
    assert_eq!(read(&mut after, later).state, LevelState::Computed);
    drop(after);
}

// ---------------------------------------------------------------------------------------------
// AC-P4-38-9 — a raise only from a confirmed, computed level.
// ---------------------------------------------------------------------------------------------

fn env(home: &Path) -> FirstRunEnv {
    FirstRunEnv {
        sources: codotheca_core::firstrun::sources::SourceEnv {
            home: home.to_path_buf(),
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
    }
}

/// The census J1.5 would write, through its own writer.
fn committers(conn: &mut Connection, project: i64, emails: &[&str]) {
    let tx = conn.transaction().unwrap();
    persist(
        &tx,
        ProjectId(project),
        &AuthorshipFacts {
            committers: emails.iter().map(|e| ((*e).to_owned(), 3)).collect(),
            authored_by_user: Some(true),
        },
    )
    .unwrap();
    tx.commit().unwrap();
}

/// A library whose every history has concluded: one project per address, each with a readable
/// copy and commit-days, and the seeded identity set not yet confirmed.
fn two_address_library(index: &mut Index) -> (i64, i64) {
    identity(index.conn(), A);
    identity(index.conn(), B);
    let a = project(index.conn(), "by-a");
    location(index.conn(), a, "/by-a", "present");
    let b = project(index.conn(), "by-b");
    location(index.conn(), b, "/by-b", "present");
    committers(index.conn_mut(), a, &[A]);
    committers(index.conn_mut(), b, &[B]);
    for p in [a, b] {
        for j in ["j1", "j1_5", "j4"] {
            job(index.conn(), p, j, "ok");
        }
    }
    commit_days(index.conn(), a, T0 / DAY, 20);
    commit_days(index.conn(), b, T0 / DAY - 100, 60);
    (a, b)
}

#[test]
fn ac_p4_38_9_pa22s_gate() {
    let dir = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let mut index = Index::open_at(dir.path(), T0).unwrap();
    let (a, _b) = two_address_library(&mut index);

    // Computed, but the card is unanswered: the read raises nothing.
    assert!(!identity_answered(index.conn()).unwrap());
    let before = read(&mut index, T0);
    let (_, both) = earned(index.conn());
    eprintln!("before the answer: {before:?}");
    assert_eq!(before.state, LevelState::Computed);
    assert_eq!(before.level, Some(both));
    assert!(
        both > 2,
        "the fixture's level must be worth protecting, got {both}"
    );
    assert_eq!(
        floor_row(index.conn()),
        (1, None),
        "an unanswered read raised the floor"
    );

    // The answer, through the production handler, with B unticked: B's project moves to
    // Reference and its history goes, and the displayed level falls with it — the pre-answer
    // level, which counted an address the user then unticked, was never locked in.
    let answered = dispatch(
        index.conn_mut(),
        &env(home.path()),
        "identity.confirm",
        &json!({ "emails": [A], "apply": true }),
        T0 + 10,
    )
    .expect("identity.confirm is first run's")
    .expect("identity.confirm answered");
    assert_eq!(answered["applied"], json!(true));
    assert!(identity_answered(index.conn()).unwrap());
    let after = read(&mut index, T0 + 20);
    let (_, mine) = earned(index.conn());
    eprintln!("after the answer: {after:?}");
    assert!(mine < both, "unticking B removed nothing: {mine} vs {both}");
    assert_eq!(after.state, LevelState::Computed);
    assert_eq!(
        after.level,
        Some(mine),
        "the card's own answer lowers the level"
    );
    assert!(!after.floored);

    // After the answer, a computed read raises the floor and dates the raise.
    assert_eq!(
        floor_row(index.conn()),
        (i64::from(mine), Some(T0 + 20)),
        "a computed read after the answer raises the floor"
    );
    assert_eq!(read_floor(index.conn()).unwrap(), mine);

    // A partial reading raises nothing, answered or not.
    completion_checks(index.conn(), a, 20);
    let (_, more) = earned(index.conn());
    assert!(more > mine, "the fixture must earn a level above the floor");
    job(index.conn(), a, "j4", "queued");
    let partial = read(&mut index, T0 + 30);
    eprintln!("partial: {partial:?}");
    assert_eq!(partial.state, LevelState::Partial);
    assert_eq!(
        partial.level,
        Some(more),
        "the displayed level is the computed one"
    );
    assert_eq!(
        floor_row(index.conn()),
        (i64::from(mine), Some(T0 + 20)),
        "a partial read raised the floor"
    );

    // Concluded again: the next read raises it, and only now moves `raised_at`.
    job(index.conn(), a, "j4", "ok");
    let computed = read(&mut index, T0 + 40);
    assert_eq!(computed.state, LevelState::Computed);
    assert_eq!(floor_row(index.conn()), (i64::from(more), Some(T0 + 40)));
    let again = read(&mut index, T0 + 50);
    assert_eq!(again.level, Some(more));
    assert_eq!(
        floor_row(index.conn()),
        (i64::from(more), Some(T0 + 40)),
        "a read that raises nothing leaves raised_at alone"
    );
}

/// The card answered by dismissal alone: `LEAVE IT AS IT IS` keeps the seeded set in force.
#[test]
fn a_dismissed_card_counts_as_answered() {
    let dir = tempfile::tempdir().unwrap();
    let mut index = Index::open_at(dir.path(), T0).unwrap();
    two_address_library(&mut index);
    read(&mut index, T0);
    assert_eq!(floor_row(index.conn()), (1, None));
    index
        .conn()
        .execute(
            "INSERT INTO view_state (k, v) VALUES ('notice.dismissed.identity', '1')",
            [],
        )
        .unwrap();
    assert!(identity_answered(index.conn()).unwrap());
    let reading = read(&mut index, T0 + 5);
    let (_, level) = earned(index.conn());
    assert_eq!(reading.level, Some(level));
    assert_eq!(floor_row(index.conn()), (i64::from(level), Some(T0 + 5)));
}

#[test]
fn xp_below_level_only_when_floored() {
    let dir = tempfile::tempdir().unwrap();
    let mut index = Index::open_at(dir.path(), T0).unwrap();
    identity(index.conn(), A);
    index
        .conn()
        .execute("UPDATE identity SET confirmed_at = ?1", [T0])
        .unwrap();
    let p = project(index.conn(), "mine");
    location(index.conn(), p, "/mine", "present");
    job(index.conn(), p, "j4", "ok");
    commit_days(index.conn(), p, T0 / DAY, 60);

    let earned_reading = read(&mut index, T0);
    eprintln!("earned: {earned_reading:?}");
    assert_consistent(&earned_reading);
    assert!(!earned_reading.floored);
    let level = earned_reading.level.unwrap();
    assert!(level > 1);

    // The git rows go (a history rewrite, say): the XP falls below the level's threshold and
    // the floor holds the level.
    index
        .conn()
        .execute("DELETE FROM xp_events WHERE track = 'git'", [])
        .unwrap();
    let held = read(&mut index, T0 + 1);
    eprintln!("held: {held:?}");
    assert_eq!(held.level, Some(level));
    assert_eq!(held.xp, Some(0));
    assert!(held.floored);
    assert_consistent(&held);
}
