//! §40.8's landing: one function names the day's one award, and one writer lands it.
//!
//! The awards are fixture rows written straight into `badge_award` — the producer under test is
//! `due`, not the evaluator — and day one is written into `app_meta` as §43's writer would. Every
//! case goes through `dispatch_badges_command`, the function the router hands both commands to,
//! and prints what it read.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use std::path::Path;

use codotheca_core::badges::commands::{dispatch_badges_command, BadgesCtx};
use codotheca_core::badges::TRICKLE_DAYS;
use codotheca_core::clock::local_date;
use codotheca_core::index::Index;
use codotheca_core::ledger::price::REVIVAL_GAP_DAYS;
use codotheca_core::proto::dispatch::CommandFailure;
use codotheca_core::protocol::ErrorCode;
use codotheca_core::testing::zone::FixedZone;
use codotheca_core::testing::TempIndex;
use serde_json::{json, Value};
use support::badges_rig::{BadgesRig, Flags, T0};

/// Day one, as a local day number in 2024.
const D: i64 = 20_000;

/// Noon UTC on local day `day`: the instant a command on that day is answered at.
const fn noon(day: i64) -> i64 {
    day * 86_400 + 43_200
}

/// An award of `kind` earned on local day `earned` and written on local day `awarded`.
fn award(index: &Index, kind: &str, earned: i64, awarded: i64) {
    index
        .conn()
        .execute(
            "INSERT INTO badge_award (kind, earned_at, tz_offset_min, earned_on, evidence,
                                      awarded_at, awarded_on)
             VALUES (?1, ?2, 0, ?3, '{}', ?4, ?5)",
            rusqlite::params![
                kind,
                earned * 86_400,
                local_date(earned),
                noon(awarded) - 3_600,
                local_date(awarded)
            ],
        )
        .unwrap();
}

/// §43's write of day one.
fn day_one(index: &Index, day: i64) {
    index
        .conn()
        .execute(
            "INSERT INTO app_meta (k, v) VALUES ('motivation_day_one', ?1)",
            [local_date(day)],
        )
        .unwrap();
}

fn command(index: &Index, day: i64, name: &str, args: Value) -> Result<Value, CommandFailure> {
    let zone = FixedZone(0);
    let ctx = BadgesCtx {
        index,
        now: noon(day),
        zone: &zone,
    };
    dispatch_badges_command(&ctx, name, args).expect("the badges module owns the command")
}

/// `badges.list` on local day `day`.
fn list(index: &Index, day: i64) -> Value {
    command(index, day, "badges.list", json!({})).unwrap()
}

/// `due` on local day `day`, printed.
fn due(index: &Index, day: i64) -> Option<String> {
    let due = list(index, day)["due"].as_str().map(str::to_owned);
    eprintln!("D{:+}: due {due:?}", day - D);
    due
}

/// `badges.markLanded { kind }` on local day `day`.
fn mark(index: &Index, day: i64, kind: &str) -> Result<Value, CommandFailure> {
    command(index, day, "badges.markLanded", json!({ "kind": kind }))
}

/// `kind`'s landing columns.
fn landed(index: &Index, kind: &str) -> (Option<i64>, Option<String>) {
    index
        .conn()
        .query_row(
            "SELECT landed_at, landed_on FROM badge_award WHERE kind = ?1",
            [kind],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap()
}

fn listed_kinds(index: &Index, day: i64) -> Vec<String> {
    list(index, day)["awards"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["kind"].as_str().unwrap().to_owned())
        .collect()
}

/// After one landing on `t`, a second awarded kind lands nothing on `t` and nothing is due for
/// the rest of it; on `t + 1` the next award is due.
#[test]
fn ac_p4_40_7_one_landing_per_local_day() {
    let lib = TempIndex::new();
    let index = lib.index();
    day_one(index, D);
    award(index, "necromancer", D - 400, D - 1);
    award(index, "polyglot", D - 200, D - 1);

    assert_eq!(due(index, D).as_deref(), Some("necromancer"));
    mark(index, D, "necromancer").unwrap();
    let necromancer = landed(index, "necromancer");
    eprintln!("necromancer landed {necromancer:?}");
    assert_eq!(necromancer, (Some(noon(D)), Some(local_date(D))));

    mark(index, D, "polyglot").unwrap();
    let polyglot = landed(index, "polyglot");
    eprintln!("polyglot after a second markLanded on D: {polyglot:?}");
    assert_eq!(polyglot, (None, None), "a second landing on one local day");
    assert_eq!(due(index, D), None, "due after the day's landing");

    assert_eq!(due(index, D + 1).as_deref(), Some("polyglot"));
    mark(index, D + 1, "polyglot").unwrap();
    assert_eq!(
        landed(index, "polyglot"),
        (Some(noon(D + 1)), Some(local_date(D + 1)))
    );
}

/// §40.14's seven clauses of the order, one assertion each.
#[test]
fn ac_p4_40_8_the_order() {
    let lib = TempIndex::new();
    let index = lib.index();
    day_one(index, D);
    award(index, "polyglot", D - 90, D - 1);
    award(index, "necromancer", D - 400, D - 1);
    award(index, "curator", D - 30, D);

    // 1. Day one lands only the first of necromancer, polyglot, comeback.
    assert_eq!(due(index, D).as_deref(), Some("necromancer"), "clause 1");
    mark(index, D, "necromancer").unwrap();

    // 2. The next day, the next retro award in the landing order.
    assert_eq!(due(index, D + 1).as_deref(), Some("polyglot"), "clause 2");
    mark(index, D + 1, "polyglot").unwrap();

    // 3. A live award is due before the remaining retro one.
    award(index, "marathon", D + 2, D + 2);
    assert_eq!(due(index, D + 2).as_deref(), Some("marathon"), "clause 3");
    mark(index, D + 2, "marathon").unwrap();
    assert_eq!(due(index, D + 3).as_deref(), Some("curator"));

    // 4. After the fortnight no retro award is due, and each is still listed.
    let after = D + TRICKLE_DAYS;
    let kinds = listed_kinds(index, after);
    eprintln!("listed at D+{TRICKLE_DAYS}: {kinds:?}");
    assert_eq!(due(index, after), None, "clause 4");
    assert!(
        kinds.contains(&"curator".to_owned()),
        "clause 4: curator left the list"
    );
    assert_eq!(kinds.len(), 4);

    // 5. A backdated award first written after the fortnight is live, and so news.
    award(index, "comeback", D - 300, D + 20);
    assert_eq!(due(index, D + 20).as_deref(), Some("comeback"), "clause 5");

    // 6. With day one unwritten nothing is due, although awards exist.
    let unarmed = TempIndex::new();
    award(unarmed.index(), "necromancer", D - 400, D - 1);
    award(unarmed.index(), "marathon", D + 2, D + 2);
    assert_eq!(listed_kinds(unarmed.index(), D + 3).len(), 2);
    assert_eq!(due(unarmed.index(), D + 3), None, "clause 6");

    // 7. markLanded of an awarded kind that is not due writes nothing; of an unawarded kind it
    //    fails PROTOCOL.
    mark(index, D + 20, "curator").unwrap();
    let curator = landed(index, "curator");
    eprintln!("curator after markLanded while comeback is due: {curator:?}");
    assert_eq!(curator, (None, None), "clause 7: a kind not due landed");
    let refused = mark(index, D + 20, "undertaker").expect_err("an unawarded kind");
    eprintln!("undertaker: {:?} {}", refused.code, refused.message);
    assert_eq!(refused.code, ErrorCode::Protocol, "clause 7");

    // Day one's own clause: nothing outside the three lands on day one — neither a retro award
    // nor a live one, which a write dated east of the zone now read can make on that date.
    let outside = TempIndex::new();
    day_one(outside.index(), D);
    award(outside.index(), "curator", D - 30, D - 1);
    award(outside.index(), "marathon", D + 1, D + 1);
    assert_eq!(
        due(outside.index(), D),
        None,
        "day one landed a kind outside the three"
    );
    assert_eq!(due(outside.index(), D + 1).as_deref(), Some("marathon"));
    mark(outside.index(), D + 1, "marathon").unwrap();
    assert_eq!(due(outside.index(), D + 2).as_deref(), Some("curator"));
}

/// Quiet mode records and changes no `due`: an award the J4 settle writes keeps `landed_at`
/// NULL, `due` reads the same with §43.5's stored key off, on and absent, and no file under
/// `core/src/badges/` reads that key.
#[test]
fn ac_p4_40_22_quiet_mode_records_and_changes_no_due() {
    let mut rig = BadgesRig::new();
    rig.sql("INSERT INTO app_meta (k, v) VALUES ('motivation_shown', '0')");
    let project = rig.project(
        "revived",
        &[D - 400, D - 400 + REVIVAL_GAP_DAYS],
        Flags::default(),
    );
    rig.settle_j4(project);
    let today = T0.div_euclid(86_400);
    rig.sql(&format!(
        "INSERT INTO app_meta (k, v) VALUES ('motivation_day_one', '{}')",
        local_date(today)
    ));

    let guard = rig.index.lock().unwrap();
    let (landed_at, awarded_on): (Option<i64>, String) = guard
        .conn()
        .query_row(
            "SELECT landed_at, awarded_on FROM badge_award WHERE kind = 'necromancer'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("the J4 settle wrote no necromancer");
    eprintln!("necromancer awarded on {awarded_on}, landed_at {landed_at:?}");
    assert_eq!(landed_at, None);

    let mut seen = Vec::new();
    for key in [Some("0"), Some("1"), None] {
        guard
            .conn()
            .execute("DELETE FROM app_meta WHERE k = 'motivation_shown'", [])
            .unwrap();
        if let Some(v) = key {
            guard
                .conn()
                .execute(
                    "INSERT INTO app_meta (k, v) VALUES ('motivation_shown', ?1)",
                    [v],
                )
                .unwrap();
        }
        let due = due(&guard, today);
        eprintln!("motivation_shown {key:?}: due {due:?}");
        seen.push(due);
    }
    drop(guard);
    assert_eq!(seen[0].as_deref(), Some("necromancer"));
    assert!(
        seen.iter().all(|d| *d == seen[0]),
        "due moved with quiet mode: {seen:?}"
    );

    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/badges");
    let mut scanned = 0;
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "rs") {
            scanned += 1;
            let text = std::fs::read_to_string(&path).unwrap();
            assert!(
                !text.contains("motivation_shown"),
                "{} reads the quiet-mode key",
                path.display()
            );
        }
    }
    eprintln!("scanned {scanned} files under core/src/badges/");
    assert!(scanned > 0, "the scan read no file");
}
