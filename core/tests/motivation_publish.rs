//! §43.3's one emitter: `motivation/changed` is pushed once per burst of committed writes and
//! at local midnight, never polled for; §43.11's quiet mode strips the payload at its source, and
//! leaving it returns a rise that waited as a level alone.

#![cfg(feature = "testkit")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use codotheca_core::clock::{day_bounds, local_date, Clock as _, LocalZone as _};
use codotheca_core::index::Index;
use codotheca_core::motivation::acknowledge::acknowledge;
use codotheca_core::motivation::keys::{day_one, KEY_LEVEL_LINE_RETURN};
use codotheca_core::motivation::publish::MotivationPublisher;
use codotheca_core::motivation::state::motivation_state;
use codotheca_core::proto::dispatch::CommandHandler as _;
use codotheca_core::proto::pubsub::{EventSink, Queued};
use codotheca_core::protocol::{MotivationAck, Topic};
use codotheca_core::scan::ScanLauncher;
use codotheca_core::testing::zone::FixedZone;
use codotheca_core::testing::{core_handler, CoreHandlerParts, HeldScanLauncher, TempIndex};
use serde_json::{json, Value};
use support::badges_rig::{
    award_row, connect_ci, plain_project, plant_commit_day, plant_repairs, red_then_green,
};

const DAY: i64 = 86_400;
const HOUR: i64 = 3_600;
/// Wednesday 2026-09-16.
const TODAY: i64 = 20_712;
const NOW: i64 = TODAY * DAY + 12 * HOUR;
/// `run_loop`'s longest sleep: the pump runs at least this often.
const PARENT_POLL: i64 = 2;

struct Rig {
    _dir: tempfile::TempDir,
    index: Arc<Mutex<Index>>,
    parts: CoreHandlerParts,
    /// How many `motivation/changed` deltas earlier pumps already returned.
    seen: usize,
}

/// A handler over a fresh index, the renderer subscribed to `motivation`.
fn rig() -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let index = Arc::new(Mutex::new(Index::open_at(dir.path(), NOW).unwrap()));
    let launcher: Arc<dyn ScanLauncher> = Arc::new(HeldScanLauncher::new(Arc::clone(&index)));
    let parts = core_handler(dir.path(), &index, NOW, launcher);
    parts.events.with(|p| p.subscribe(Topic::Motivation));
    Rig {
        _dir: dir,
        index,
        parts,
        seen: 0,
    }
}

impl Rig {
    fn handle(&mut self, command: &str, args: Value) -> Value {
        self.parts
            .handler
            .handle(command, args)
            .unwrap_or_else(|e| panic!("{command}: {e:?}"))
    }

    fn get(&mut self) -> Value {
        self.handle("motivation.get", json!({}))
    }

    fn ack(&mut self, event: &str, level: Option<u32>) {
        self.handle(
            "motivation.acknowledge",
            json!({ "event": event, "level": level }),
        );
    }

    fn quiet(&mut self, quiet: bool) {
        self.handle(
            "settings.set",
            json!({ "patch": { "motivationShown": !quiet } }),
        );
    }

    /// One loop iteration; the `motivation/changed` payloads it pushed. A job the tick queued
    /// finishes on a worker thread and its write moves the payload, so the iteration waits for
    /// it: otherwise that write lands in whichever later pump runs after it, at random.
    fn pump(&mut self) -> Vec<Value> {
        self.parts.handler.pump();
        let start = Instant::now();
        while !self.parts.handler.jobs_idle() {
            assert!(
                start.elapsed() < Duration::from_secs(60),
                "the tick's jobs never settled"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
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

    /// Pumps every `PARENT_POLL` for `secs` of fake time; every payload pushed meanwhile.
    fn run_for(&mut self, secs: i64) -> Vec<Value> {
        let mut pushed = Vec::new();
        for _ in 0..secs / PARENT_POLL {
            self.parts.clock.advance(PARENT_POLL);
            pushed.extend(self.pump());
        }
        pushed
    }

    fn meta(&self, key: &str) -> Option<String> {
        let guard = self.index.lock().unwrap();
        guard
            .conn()
            .query_row("SELECT v FROM app_meta WHERE k = ?1", [key], |r| r.get(0))
            .ok()
    }

    fn count(&self, sql: &str) -> i64 {
        let guard = self.index.lock().unwrap();
        guard.conn().query_row(sql, [], |r| r.get(0)).unwrap()
    }

    /// A live, owned, full-history project with no copy on disk, so the level is computed.
    fn project(&self, name: &str) -> i64 {
        let guard = self.index.lock().unwrap();
        guard
            .conn()
            .execute(
                "INSERT INTO project (name, seed_basename, lineage_key, created_at, updated_at)
                 VALUES (?1, ?1, ?1, 1, 1)",
                [name],
            )
            .unwrap();
        guard.conn().last_insert_rowid()
    }

    /// One forward commit-day, its own write.
    fn commit_day(&self, project: i64, day: i64) {
        let meta = json!({ "when": local_date(day) }).to_string();
        self.index
            .lock()
            .unwrap()
            .conn()
            .execute(
                "INSERT INTO xp_events (ts, project_id, subject_key, kind, dedupe_key, track, meta)
                 VALUES (?1, ?2, ?3, 'commit_day', ?4, 'git', ?5)",
                rusqlite::params![
                    day * DAY,
                    project,
                    format!("subject-{project}"),
                    format!("commit_day:{project}:{day}"),
                    meta
                ],
            )
            .unwrap();
    }

    /// A library past level 1: thirteen recent commit-days, none ending a dormancy.
    fn library(&self) -> i64 {
        let p = self.project("p1");
        for n in 0..13 {
            self.commit_day(p, TODAY - 30 + n);
        }
        p
    }
}

fn displayed(state: &Value) -> u64 {
    state["level"]["figures"]["level"]
        .as_u64()
        .unwrap_or_else(|| panic!("no figures: {state}"))
}

fn xp(state: &Value) -> i64 {
    state["level"]["figures"]["xp"]
        .as_i64()
        .unwrap_or_else(|| panic!("no figures: {state}"))
}

fn line_form(state: &Value) -> Option<&str> {
    state["pendingLine"]["form"].as_str()
}

/// AC-P4-43-1: a ledger row written through a production caller of the payout reaches the
/// band as one pushed payload; a burst coalesces; an unchanged payload and an idle hour push
/// nothing; local midnight pushes the new day; and what is pushed is what the read and the
/// topic's snapshot answer.
#[test]
fn ac_p4_43_1_a_real_debt_day_produces_one_event() {
    let mut r = rig();
    let library = r.library();
    let beta = {
        let mut guard = r.index.lock().unwrap();
        let beta = plain_project(&mut guard, "beta", true);
        connect_ci(&guard, beta);
        drop(guard);
        beta
    };
    let first = r.pump();
    assert_eq!(first.len(), 1, "the first computation is pushed");
    let before = xp(&first[0]);

    // A red build repaired the same day: `settle_singletons` pays the day.
    {
        let mut guard = r.index.lock().unwrap();
        red_then_green(&mut guard, beta, NOW - 6 * HOUR, NOW - 3 * HOUR);
        drop(guard);
    }
    assert_eq!(
        r.count("SELECT count(*) FROM xp_events WHERE kind = 'debt_day'"),
        1
    );
    let paid = r.pump();
    eprintln!("after the payout: {} pushed", paid.len());
    assert_eq!(paid.len(), 1, "one payout, one push");
    assert!(
        xp(&paid[0]) > before,
        "the pushed payload carries the payout: {} -> {}",
        before,
        xp(&paid[0])
    );

    // Forty writes, then one loop iteration: one computation.
    for n in 0..40 {
        r.commit_day(library, TODAY - 200 + n);
    }
    let burst = r.pump();
    eprintln!("forty writes: {} pushed", burst.len());
    assert_eq!(burst.len(), 1);
    assert!(r.pump().is_empty(), "nothing moved since");

    r.get();
    assert!(r.pump().is_empty(), "a read's unchanged payload is pushed");

    // An hour with no write of ours and no midnight.
    let idle = r.run_for(HOUR);
    eprintln!("an idle hour: {} pushed", idle.len());
    assert!(idle.is_empty(), "{idle:?}");

    // Local midnight, with nothing written.
    let now = r.parts.clock.now_unix();
    r.parts.clock.advance((TODAY + 1) * DAY - 10 - now);
    let late = r.pump();
    assert!(late.is_empty(), "{late:?}");
    let crossed = r.run_for(60);
    eprintln!("across midnight: {} pushed", crossed.len());
    assert_eq!(crossed.len(), 1, "{crossed:?}");
    assert_eq!(crossed[0]["rings"]["today"], local_date(TODAY + 1));

    let read = r.get();
    assert_eq!(crossed[0], read, "the push is the read's answer");
    assert_eq!(r.parts.handler.snapshot(Topic::Motivation), read);
}

/// AC-P4-43-18: a rise during quiet returns, when quiet ends, as the current level alone; no rise
/// is no line; a level never presented arrives.
#[test]
fn ac_p4_43_18_leaving_quiet() {
    // A forward rise during quiet.
    let mut r = rig();
    let p = r.library();
    let shown = displayed(&r.get());
    r.ack("levelLineClicked", Some(u32::try_from(shown).unwrap()));
    r.quiet(true);
    for n in 0..40 {
        r.commit_day(p, TODAY - 100 + n);
    }
    r.quiet(false);
    let state = r.get();
    eprintln!("left quiet: {}", state["pendingLine"]);
    assert!(displayed(&state) > shown, "the writes raised the level");
    assert_eq!(line_form(&state), Some("return"));
    assert_eq!(state["pendingLine"]["level"], displayed(&state));
    assert_eq!(state["pendingLine"]["reason"], Value::Null);
    assert_eq!(
        r.meta(KEY_LEVEL_LINE_RETURN),
        Some(displayed(&state).to_string())
    );
    let pushed = r.pump();
    assert_eq!(pushed.last().map(line_form), Some(Some("return")));

    // Quiet entered and left with no rise between.
    let mut still = rig();
    still.library();
    let level = displayed(&still.get());
    still.ack("levelLineClicked", Some(u32::try_from(level).unwrap()));
    still.quiet(true);
    still.quiet(false);
    assert_eq!(still.get()["pendingLine"], Value::Null);
    assert_eq!(still.meta(KEY_LEVEL_LINE_RETURN), None);

    // A level never presented.
    let mut fresh = rig();
    fresh.library();
    fresh.quiet(true);
    fresh.quiet(false);
    assert_eq!(line_form(&fresh.get()), Some("arrival"));
    assert_eq!(fresh.meta(KEY_LEVEL_LINE_RETURN), None);
}

/// AC-P4-43-17: under quiet the pushed payload is `shown: false` and nothing else, while a
/// ledger row and a badge award are still written through their production callers.
#[test]
fn ac_p4_43_17_quiet_strips_at_the_source() {
    let mut r = rig();
    r.library();
    assert_eq!(r.pump().len(), 1);
    r.quiet(true);
    let stripped = r.pump();
    assert_eq!(stripped.len(), 1);
    assert_eq!(
        stripped[0],
        json!({
            "shown": false,
            "level": null,
            "rings": null,
            "xpBar": null,
            "pendingLine": null,
            "terms": null,
        })
    );

    // Nine same-day repairs planted, the tenth through the singleton payout's caller.
    let tenth = TODAY - 2;
    {
        let mut guard = r.index.lock().unwrap();
        let days: Vec<i64> = (0..9).map(|i| TODAY - 20 + i).collect();
        let beta = plant_repairs(&mut guard, "beta", &days);
        connect_ci(&guard, beta);
        plant_commit_day(&guard, beta, tenth);
        red_then_green(
            &mut guard,
            beta,
            tenth * DAY + 9 * HOUR,
            tenth * DAY + 15 * HOUR,
        );
        let award = award_row(guard.conn(), "firefighter");
        eprintln!("under quiet: award {award:?}");
        assert!(award.is_some(), "quiet stopped an award");
        drop(guard);
    }
    assert_eq!(
        r.count("SELECT count(*) FROM xp_events WHERE kind = 'debt_day'"),
        10,
        "quiet stopped the payout"
    );
    let after = r.pump();
    assert!(
        after.is_empty(),
        "the stripped payload did not change: {after:?}"
    );
}

/// The band's `today` is the zone's local date: it flips at local midnight, not UTC's, in the
/// read, in the push, and in the day one a presentation stamps.
#[test]
fn the_band_today_flips_at_local_midnight_under_the_zone() {
    let mut t = TempIndex::new();
    let project = t.insert_project();
    t.insert_location(project, "/w/p");
    let z = FixedZone(330);
    let d = TODAY;
    let m = day_bounds(&z, d).0;
    let local = local_date(d);
    let utc = FixedZone(0).at(m).date();
    eprintln!("local midnight {m}: local {local}, UTC {utc}");
    assert_ne!(utc, local, "the case would pass under UTC");

    // (a) The read.
    t.index_mut()
        .with_tx(|tx| {
            let before = motivation_state(tx, m - 1, &z)?.rings.expect("rings");
            let at = motivation_state(tx, m, &z)?.rings.expect("rings");
            assert_eq!(before.today, local_date(d - 1));
            assert_eq!(at.today, local);
            Ok(())
        })
        .unwrap();

    // (b) The push.
    let sink = Recording::default();
    let mut publisher = MotivationPublisher::new();
    let mut count = 0;
    for (at, expected) in [(m - 60, 1), (m - 1, 0), (m, 1), (m + 19_800, 0)] {
        let emitted = publisher.poll(t.index(), at, &z, &sink).unwrap();
        let events = sink.take();
        count += events.len();
        eprintln!("poll at {at}: {} pushed, {count} in all", events.len());
        assert_eq!(events.len(), expected, "at {at}");
        assert_eq!(emitted, expected == 1);
        if at == m {
            let (topic, event, payload) = &events[0];
            assert_eq!((topic.as_str(), event.as_str()), ("motivation", "changed"));
            assert_eq!(payload["rings"]["today"], local);
        }
    }

    // (c) Day one.
    t.index_mut()
        .with_tx(|tx| {
            acknowledge(tx, MotivationAck::BandShown, None, m, &z)?;
            Ok(())
        })
        .unwrap();
    assert_eq!(day_one(t.index().conn()).unwrap(), Some(local));
}

static STATEMENTS: AtomicUsize = AtomicUsize::new(0);

fn count_statement(_: &str) {
    STATEMENTS.fetch_add(1, Ordering::SeqCst);
}

/// A computation whose own read writes — the floor's ratchet on a raise — does not trigger the
/// next one: the change counter is read after it, so an idle poll runs no statement at all.
#[test]
fn the_publishers_own_write_does_not_trigger_it_again() {
    let mut t = TempIndex::new();
    // Live, owned and of full history with no copy on disk, so the level is computed; and the
    // identity question answered, so the reading may raise the floor.
    t.index()
        .conn()
        .execute(
            "INSERT INTO identity (email, is_user, source, confirmed_at)
             VALUES ('fixture@example.invalid', 1, 'manual', 1)",
            [],
        )
        .unwrap();
    t.index()
        .conn()
        .execute(
            "INSERT INTO project (name, seed_basename, lineage_key, created_at, updated_at)
             VALUES ('p', 'p', 'p', 1, 1)",
            [],
        )
        .unwrap();
    let project = codotheca_core::protocol::ProjectId(t.index().conn().last_insert_rowid());
    for n in 0..30 {
        t.index()
            .conn()
            .execute(
                "INSERT INTO xp_events (ts, project_id, subject_key, kind, dedupe_key, track)
                 VALUES (?1, ?2, 'subject', 'commit_day', ?3, 'git')",
                rusqlite::params![(TODAY - 40 + n) * DAY, project.0, format!("cd:{n}")],
            )
            .unwrap();
    }
    let floor = |index: &TempIndex| -> i64 {
        index
            .index()
            .conn()
            .query_row("SELECT level FROM level_floor", [], |r| r.get(0))
            .unwrap()
    };
    let zone = FixedZone(0);
    let sink = Recording::default();
    let mut publisher = MotivationPublisher::new();
    let before = floor(&t);
    assert!(publisher.poll(t.index(), NOW, &zone, &sink).unwrap());
    let after = floor(&t);
    eprintln!("floor {before} -> {after}");
    assert!(
        after > before,
        "the computation wrote nothing, so this proves nothing"
    );

    STATEMENTS.store(0, Ordering::SeqCst);
    t.index_mut().conn_mut().trace(Some(count_statement));
    let again = publisher.poll(t.index(), NOW + 1, &zone, &sink).unwrap();
    t.index_mut().conn_mut().trace(None);
    let ran = STATEMENTS.load(Ordering::SeqCst);
    eprintln!("an idle poll ran {ran} statements");
    assert!(!again);
    assert_eq!(ran, 0, "the publisher's own write triggered it again");
}

/// Every event emitted, as `(topic, event, payload)`, each checked against the schema first.
#[derive(Debug, Default)]
struct Recording(Mutex<Vec<(String, String, Value)>>);

impl Recording {
    fn take(&self) -> Vec<(String, String, Value)> {
        std::mem::take(&mut *self.0.lock().unwrap())
    }
}

impl EventSink for Recording {
    fn emit(&self, topic: &str, event: &str, payload: Value) {
        codotheca_core::testing::events::validated(topic, event, &payload);
        self.0
            .lock()
            .unwrap()
            .push((topic.to_owned(), event.to_owned(), payload));
    }
}
