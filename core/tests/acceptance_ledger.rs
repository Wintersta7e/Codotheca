//! §38.6: the displayed level never falls. `AC-P4-38-8` walks one library through every production
//! path that can lower the level its XP earns — an identity narrowing after the answer, a J4
//! reconcile after a history rewrite, a price-table change, a curve change, and a rebuild restoring
//! the last export — reading the level through `read_level` after each. Every step must lower the
//! earned level, or the floor holding the displayed one proves nothing.
//!
//! The overrides are process-global, so this binary holds one test.

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
use codotheca_core::git::{GitSlots, SystemGit};
use codotheca_core::identity::confirm;
use codotheca_core::index::rebuild::{rebuild_in_place, RebuildOutcome};
use codotheca_core::index::Index;
use codotheca_core::jobs::scheduler::JobRunner;
use codotheca_core::jobs::{Job, JobDeps, JobKind, JobOrigin, Priority};
use codotheca_core::ledger::curve::{level_of, LEVEL_THRESHOLDS};
use codotheca_core::ledger::floor::read_floor;
use codotheca_core::ledger::level::read_level;
use codotheca_core::ledger::price::testkit::{override_curve, override_prices};
use codotheca_core::ledger::price::{PriceTable, PRICES};
use codotheca_core::mount::StoreClass;
use codotheca_core::proto::EventSink;
use codotheca_core::protocol::{LevelState, LocationId, ProjectId};
use codotheca_core::testing::zone::FixedZone;
use codotheca_core::testing::FakeClock;
use serde_json::Value;
use support::TestRepo;

const T0: i64 = 1_800_000_000;
const DAY: i64 = 86_400;
/// 2024-01-01, the first day of the fixture's history.
const START: i64 = 19_723;

/// The address `TestRepo`'s own commits carry, so the rewrite's squash stays the user's.
const ME: &str = "fixture@example.invalid";
const WORK: &str = "work@example.invalid";

// One commit-day per date and no gap, so no cap or revival moves a figure. 190 days earn level 6;
// without WORK's 60, level 5; with 50 squashed away, 80 earn level 4; at half price their 400 XP
// earn level 3, and on a doubled curve level 2; the rebuilt ledger holds none of them: level 1.
const WORK_DAYS: i64 = 60;
const MY_DAYS: i64 = 130;
const SQUASHED: &str = "HEAD~50";

/// What a J4 settle announces is not under test here, beyond matching the schema.
struct Quiet;

impl EventSink for Quiet {
    fn emit(&self, topic: &str, event: &str, payload: Value) {
        codotheca_core::testing::events::validated(topic, event, &payload);
    }
}

/// A linear history of empty commits on `main`, one per `(committer, day)` at noon UTC, written as
/// raw objects because every commit `TestRepo` makes carries the one fixture address.
fn history(repo: &TestRepo, commits: &[(&str, i64)]) {
    let tree = repo.git(&["write-tree"]).trim().to_owned();
    let body = repo.scratch().join("commit");
    let mut parent: Option<String> = None;
    for &(email, day) in commits {
        let at = day * DAY + DAY / 2;
        let parent_line = parent.map_or_else(String::new, |p| format!("parent {p}\n"));
        std::fs::write(
            &body,
            format!(
                "tree {tree}\n{parent_line}author Someone <{email}> {at} +0000\n\
                 committer Someone <{email}> {at} +0000\n\nday {day}\n"
            ),
        )
        .unwrap();
        let commit = repo.git(&["hash-object", "-t", "commit", "-w", &body.to_string_lossy()]);
        parent = Some(commit.trim().to_owned());
    }
    repo.git(&["update-ref", "refs/heads/main", &parent.unwrap()]);
}

/// One J4 over `repo` on a fresh real `JobRunner`, spun until idle — never a bare sleep.
fn settle(index: &Arc<Mutex<Index>>, repo: &TestRepo, project: ProjectId, location: LocationId) {
    let clock = Arc::new(FakeClock::new(T0));
    let deps = JobDeps {
        git: Arc::new(SystemGit::new(
            Arc::new(repo.exec()),
            Arc::new(GitSlots::new(4)),
            clock.clone(),
        )),
        clock,
        cancel: CancelToken::new(),
        zone: Arc::new(FixedZone(0)),
    };
    let runner = JobRunner::new(Arc::clone(index), deps, Arc::new(Quiet));
    assert!(runner.enqueue(Job {
        kind: JobKind::J4History,
        project_id: project,
        location_id: location,
        store_key: "store".to_owned(),
        store_kind: StoreClass::Local,
        priority: Priority::Interactive,
        not_before: 0,
        origin: JobOrigin::Walk,
    }));
    runner.start(1);
    let deadline = Instant::now() + Duration::from_secs(60);
    while Instant::now() < deadline && !runner.is_idle() {
        std::thread::sleep(Duration::from_millis(25));
    }
    let idle = runner.is_idle();
    runner.request_stop();
    runner.join();
    assert!(idle, "J4 did not settle");
}

/// The identity card's answer, as its handler applies it, keeping exactly `emails`.
fn answer(index: &Arc<Mutex<Index>>, emails: &[&str], now: i64) {
    let mut guard = index.lock().unwrap();
    let emails: Vec<String> = emails.iter().map(|e| (*e).to_owned()).collect();
    confirm::apply(guard.conn_mut(), &emails, now).unwrap();
    drop(guard);
}

/// One reading: the displayed level, the XP, and the level that XP earns on the curve in force.
struct Reading {
    level: u32,
    xp: i64,
    earned: u32,
}

/// The level after `step`, read through `read_level` and printed; `floored` must hold exactly
/// when the floor exceeds the earned level.
fn read(index: &mut Index, now: i64, step: &str) -> Reading {
    let reading = index.with_tx(|tx| read_level(tx, now)).unwrap();
    let floor = read_floor(index.conn()).unwrap();
    let (Some(level), Some(xp)) = (reading.level, reading.xp) else {
        panic!("{step}: a computed reading carries its numbers: {reading:?}");
    };
    let earned = level_of(xp);
    eprintln!(
        "{step}: level {level}, xp {xp}, earned {earned}, floor {floor}, floored {}",
        reading.floored
    );
    assert_eq!(reading.state, LevelState::Computed, "{step}");
    assert_eq!(
        reading.floored,
        floor > earned,
        "{step}: floored is the floor exceeding the earned level"
    );
    Reading { level, xp, earned }
}

/// `after`, read once `step` ran, against the reading before it. The curve's step moves no XP;
/// every other step must.
fn holds(before: &Reading, after: &Reading, step: &str, xp_moves: bool) {
    assert!(
        after.level >= before.level,
        "{step} lowered the displayed level: {} to {}",
        before.level,
        after.level
    );
    assert!(
        after.earned < before.earned,
        "{step} did not lower the earned level ({} to {}), so it proves nothing",
        before.earned,
        after.earned
    );
    if xp_moves {
        assert!(after.xp < before.xp, "{step} moved no XP");
    } else {
        assert_eq!(after.xp, before.xp, "{step} moved XP");
    }
}

#[test]
fn ac_p4_38_8_the_displayed_level_never_falls() {
    let repo = TestRepo::init();
    let work = (START..START + WORK_DAYS).map(|day| (WORK, day));
    let mine = (START + WORK_DAYS..START + WORK_DAYS + MY_DAYS).map(|day| (ME, day));
    history(&repo, &work.chain(mine).collect::<Vec<_>>());

    // The starting library: one present copy, both addresses seeded and not yet answered, and the
    // census J1.5 would write.
    let dir = tempfile::tempdir().unwrap();
    let mut index = Index::open_at(dir.path(), T0).unwrap();
    let path = support::canonical(repo.path())
        .to_string_lossy()
        .into_owned();
    let (project, location) = index
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
                rusqlite::params![project.0, path.as_bytes(), path],
            )?;
            let location = LocationId(tx.last_insert_rowid());
            for (email, commits) in [(ME, MY_DAYS), (WORK, WORK_DAYS)] {
                tx.execute(
                    "INSERT INTO identity (email, is_user, source) VALUES (?1, 1, 'gitconfig')",
                    [email],
                )?;
                tx.execute(
                    "INSERT INTO project_committer (project_id, email, commits)
                     VALUES (?1, ?2, ?3)",
                    rusqlite::params![project.0, email, commits],
                )?;
            }
            Ok((project, location))
        })
        .unwrap();
    let index = Arc::new(Mutex::new(index));
    settle(&index, &repo, project, location);

    // The answer, keeping both addresses: the first computed read after it raises the floor.
    answer(&index, &[ME, WORK], T0 + 10);
    let answered = read(&mut index.lock().unwrap(), T0 + 20, "answered");
    // The last export, older than every step below: the floor it carries is all the rebuild keeps.
    index.lock().unwrap().export_sidecar(T0 + 25).unwrap();

    // An identity narrowing after the answer: the next J4 drops the unticked address's days.
    answer(&index, &[ME], T0 + 30);
    settle(&index, &repo, project, location);
    let narrowed = read(&mut index.lock().unwrap(), T0 + 40, "narrowed");
    holds(&answered, &narrowed, "the narrowing", true);

    // A history rewrite — the newest fifty commits squashed into the one beneath them, which keeps
    // its author date — reconciled by the next J4.
    repo.git(&["reset", "-q", "--soft", SQUASHED]);
    repo.git(&["commit", "-q", "--amend", "--allow-empty", "--no-edit"]);
    settle(&index, &repo, project, location);
    let rewritten = read(&mut index.lock().unwrap(), T0 + 50, "rewritten");
    holds(&narrowed, &rewritten, "the rewrite", true);

    let _prices = override_prices(PriceTable {
        commit_day: PRICES.commit_day / 2,
        ..PRICES
    });
    let repriced = read(&mut index.lock().unwrap(), T0 + 60, "repriced");
    holds(&rewritten, &repriced, "the price change", true);

    let _curve = override_curve(LEVEL_THRESHOLDS.map(|t| t * 2));
    let recurved = read(&mut index.lock().unwrap(), T0 + 70, "recurved");
    holds(&repriced, &recurved, "the curve change", false);

    // The Reference verdict's step is §44's, driven through the one verdict writer it rules.

    // A rebuild restoring the last export, through the production entry.
    drop(Arc::try_unwrap(index).unwrap().into_inner().unwrap());
    std::fs::write(Index::db_path(dir.path()), b"this is not a database").unwrap();
    match rebuild_in_place(dir.path(), T0 + 90) {
        Ok(RebuildOutcome::Rebuilt(_)) => {}
        other => panic!("expected a rebuild, got {other:?}"),
    }
    let mut restored = Index::open_at(dir.path(), T0 + 100).unwrap();
    let rebuilt = read(&mut restored, T0 + 110, "rebuilt");
    holds(&recurved, &rebuilt, "the rebuild", true);
}
