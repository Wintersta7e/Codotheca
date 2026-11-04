#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! §43.15's two commands through the core's real dispatcher: `motivation.get` reads and writes
//! nothing of §43's; `motivation.acknowledge` records a presentation the user saw — day one once,
//! arming §40 in the same transaction, the level shown clamped to the level displayed, a badge
//! line's click, and the end of first run.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use codotheca_core::clock::local_date;
use codotheca_core::index::Index;
use codotheca_core::ledger::price::REVIVAL_GAP_DAYS;
use codotheca_core::motivation::keys::{
    KEY_BADGE_LINE_ACKED, KEY_LEVEL_LINE_SINCE, KEY_LEVEL_SHOWN, KEY_MOTIVATION_DAY_ONE,
};
use codotheca_core::proto::dispatch::CommandHandler as _;
use codotheca_core::protocol::Topic;
use codotheca_core::scan::ScanLauncher;
use codotheca_core::testing::{core_handler, CoreHandlerParts, HeldScanLauncher};
use serde_json::{json, Value};

const DAY: i64 = 86_400;
/// Wednesday 2026-09-16.
const TODAY: i64 = 20_712;
const NOW: i64 = TODAY * DAY + 12 * 3_600;

struct Rig {
    _dir: tempfile::TempDir,
    index: Arc<Mutex<Index>>,
    parts: CoreHandlerParts,
}

fn rig() -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let index = Arc::new(Mutex::new(Index::open_at(dir.path(), NOW).unwrap()));
    let launcher: Arc<dyn ScanLauncher> = Arc::new(HeldScanLauncher::new(Arc::clone(&index)));
    let parts = core_handler(dir.path(), &index, NOW, launcher);
    Rig {
        _dir: dir,
        index,
        parts,
    }
}

impl Rig {
    fn get(&mut self) -> Value {
        self.parts
            .handler
            .handle("motivation.get", json!({}))
            .expect("motivation.get is answerable")
    }

    fn ack(&mut self, event: &str, level: Option<u32>) -> Value {
        self.parts
            .handler
            .handle(
                "motivation.acknowledge",
                json!({ "event": event, "level": level }),
            )
            .expect("motivation.acknowledge is answerable")
    }

    fn quiet(&mut self, quiet: bool) {
        self.parts
            .handler
            .handle(
                "settings.set",
                json!({ "patch": { "motivationShown": !quiet } }),
            )
            .expect("settings.set is answerable");
    }

    fn meta(&self, key: &str) -> Option<String> {
        let guard = self.index.lock().unwrap();
        guard
            .conn()
            .query_row("SELECT v FROM app_meta WHERE k = ?1", [key], |r| r.get(0))
            .ok()
    }

    fn set_meta(&self, key: &str, value: &str) {
        let guard = self.index.lock().unwrap();
        guard
            .conn()
            .execute(
                "INSERT INTO app_meta (k, v) VALUES (?1, ?2)
                 ON CONFLICT (k) DO UPDATE SET v = excluded.v",
                [key, value],
            )
            .unwrap();
    }

    /// A live, owned, full-history project with no copy on disk: in every population a badge
    /// reads, and never pending, so the level is computed.
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

    fn commit_day(&self, project: i64, day: i64) {
        let subject = format!("subject-{project}");
        let dedupe = format!("commit_day:{project}:{day}");
        let meta = json!({ "when": local_date(day) }).to_string();
        self.index
            .lock()
            .unwrap()
            .conn()
            .execute(
                "INSERT INTO xp_events (ts, project_id, subject_key, kind, dedupe_key, track, meta)
                 VALUES (?1, ?2, ?3, 'commit_day', ?4, 'git', ?5)",
                rusqlite::params![day * DAY, project, subject, dedupe, meta],
            )
            .unwrap();
    }

    /// A library past level 1: thirteen recent commit-days, none ending a dormancy.
    fn library(&self) {
        let p = self.project("p1");
        for n in 0..13 {
            self.commit_day(p, TODAY - 30 + n);
        }
    }

    /// An award of `kind` earned on `day`; `landed` is the day its line was painted.
    fn award(&self, kind: &str, day: i64, landed: Option<i64>) {
        let guard = self.index.lock().unwrap();
        guard
            .conn()
            .execute(
                "INSERT INTO badge_award
                    (kind, earned_at, tz_offset_min, earned_on, evidence, awarded_at, awarded_on,
                     landed_at, landed_on)
                 VALUES (?1, ?2, 0, ?3, '{\"projectName\":\"p1\"}', ?2, ?3, ?4, ?5)",
                rusqlite::params![
                    kind,
                    day * DAY,
                    local_date(day),
                    landed.map(|d| d * DAY + 3_600),
                    landed.map(local_date),
                ],
            )
            .unwrap();
    }

    fn awards(&self) -> Vec<String> {
        let guard = self.index.lock().unwrap();
        let mut st = guard
            .conn()
            .prepare("SELECT kind FROM badge_award ORDER BY kind")
            .unwrap();
        let kinds = st
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        drop(st);
        drop(guard);
        kinds
    }

    fn first_run_completed_at(&self) -> Option<i64> {
        codotheca_core::firstrun::first_run_completed_at(self.index.lock().unwrap().conn()).unwrap()
    }

    /// Day one three days back and level 1 shown over an empty ledger: no level line, so a badge
    /// line may show.
    fn badge_ready(&self) {
        self.set_meta(KEY_MOTIVATION_DAY_ONE, &local_date(TODAY - 3));
        self.set_meta(KEY_LEVEL_SHOWN, "1");
        self.set_meta(KEY_LEVEL_LINE_SINCE, "0");
    }
}

fn displayed(state: &Value) -> u32 {
    u32::try_from(
        state["level"]["figures"]["level"]
            .as_u64()
            .expect("figures"),
    )
    .unwrap()
}

fn line_form(state: &Value) -> Option<&str> {
    state["pendingLine"]["form"].as_str()
}

/// `bandShown`: day one is written by a presentation and nothing else, once, and the transaction
/// that writes it runs §40's arming sweep.
fn day_one_case() {
    let mut r = rig();
    // A revival long before day one: the sweep's Necromancer predicate holds, and only the sweep
    // can latch it here.
    let p = r.project("revived");
    r.commit_day(p, TODAY - 400);
    r.commit_day(p, TODAY - 400 + REVIVAL_GAP_DAYS);

    for _ in 0..3 {
        r.get();
    }
    assert_eq!(
        r.meta(KEY_MOTIVATION_DAY_ONE),
        None,
        "a read is not a presentation"
    );
    r.quiet(true);
    r.ack("bandShown", None);
    assert_eq!(
        r.meta(KEY_MOTIVATION_DAY_ONE),
        None,
        "quiet mode writes nothing"
    );
    assert!(r.awards().is_empty(), "nothing armed §40 yet");

    r.quiet(false);
    r.ack("bandShown", None);
    assert_eq!(
        r.meta(KEY_MOTIVATION_DAY_ONE).as_deref(),
        Some(local_date(TODAY).as_str())
    );
    let awards = r.awards();
    assert!(
        awards.iter().any(|kind| kind == "necromancer"),
        "the arming sweep ran in the transaction that wrote day one: {awards:?}"
    );

    r.parts.clock.advance(DAY);
    r.ack("bandShown", None);
    assert_eq!(
        r.meta(KEY_MOTIVATION_DAY_ONE).as_deref(),
        Some(local_date(TODAY).as_str()),
        "day one is written once"
    );
}

/// `revealLevelShown`: with the level the reveal displayed it stamps day one and the level shown,
/// so no arrival follows; with `null` (not computed) it stamps day one only.
fn reveal_case() {
    let mut r = rig();
    r.library();
    let level = displayed(&r.get());
    assert!(level >= 2, "the library is past level 1: {level}");
    assert_eq!(line_form(&r.get()), Some("arrival"));

    let state = r.ack("revealLevelShown", Some(level));
    assert_eq!(
        r.meta(KEY_MOTIVATION_DAY_ONE).as_deref(),
        Some(local_date(TODAY).as_str())
    );
    assert_eq!(r.meta(KEY_LEVEL_SHOWN), Some(level.to_string()));
    assert_eq!(
        line_form(&state),
        None,
        "the reply is the state after the write"
    );
    assert_eq!(
        line_form(&r.get()),
        None,
        "a level shown by the reveal does not arrive"
    );

    let mut n = rig();
    n.library();
    n.ack("revealLevelShown", None);
    assert_eq!(
        n.meta(KEY_MOTIVATION_DAY_ONE).as_deref(),
        Some(local_date(TODAY).as_str())
    );
    assert_eq!(n.meta(KEY_LEVEL_SHOWN), None);
    assert_eq!(line_form(&n.get()), Some("arrival"));
}

/// `levelLineClicked`: a level above the one displayed is clamped to it.
fn clamp_case() {
    let mut r = rig();
    r.library();
    let level = displayed(&r.get());
    r.ack("levelLineClicked", Some(level + 3));
    assert_eq!(r.meta(KEY_LEVEL_SHOWN), Some(level.to_string()));
    assert_eq!(line_form(&r.get()), None);

    r.ack("levelLineClicked", Some(1));
    assert_eq!(
        r.meta(KEY_LEVEL_SHOWN),
        Some(level.to_string()),
        "a lower level never lowers what was shown"
    );
}

/// `badgeLineClicked`: clears a landed award's line; over an award due and not yet painted it
/// writes nothing.
fn badge_case() {
    let mut r = rig();
    r.badge_ready();
    r.award("curator", TODAY - 1, Some(TODAY));
    assert_eq!(line_form(&r.get()), Some("badge"));
    let state = r.ack("badgeLineClicked", None);
    assert_eq!(r.meta(KEY_BADGE_LINE_ACKED).as_deref(), Some("curator"));
    assert_eq!(line_form(&state), None);

    let mut due = rig();
    due.badge_ready();
    due.award("curator", TODAY - 1, None);
    assert_eq!(line_form(&due.get()), Some("badge"));
    due.ack("badgeLineClicked", None);
    assert_eq!(
        due.meta(KEY_BADGE_LINE_ACKED),
        None,
        "an unlanded award is not acked"
    );
    assert_eq!(line_form(&due.get()), Some("badge"));
}

/// `firstRunEnded`: stamps the end of first run, and acknowledges the award the reveal's panel
/// landed, so it never returns as a line.
fn first_run_case() {
    let mut r = rig();
    r.badge_ready();
    r.award("curator", TODAY - 1, Some(TODAY));
    let state = r.ack("firstRunEnded", None);
    assert_eq!(r.first_run_completed_at(), Some(NOW));
    assert_eq!(r.meta(KEY_BADGE_LINE_ACKED).as_deref(), Some("curator"));
    assert_eq!(line_form(&state), None);
}

#[test]
fn ac_p4_43_14_day_one_is_written_once_by_the_presentation_only() {
    day_one_case();
}

#[test]
fn ac_p4_43_12_reveal_level_shown_stamps_and_raises_level_shown() {
    reveal_case();
    clamp_case();
    badge_case();
    first_run_case();
}

/// `motivation/snapshot` is the read's own answer, so a resync renders what the read would have.
#[test]
fn the_motivation_snapshot_is_the_reads_answer() {
    let mut r = rig();
    r.library();
    let snapshot = r.parts.handler.snapshot(Topic::Motivation);
    assert!(snapshot["level"].is_object(), "{snapshot}");
    assert_eq!(snapshot, r.get());
}

/// §49.2 rule 12: every acknowledgement the schema declares is driven once; a variant with no
/// case fails.
#[test]
fn every_acknowledgement_the_schema_declares_has_a_case() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../protocol/schema/protocol.json"
    );
    let schema: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let variants: BTreeSet<String> = schema["types"]["MotivationAck"]["variants"]
        .as_array()
        .expect("MotivationAck declares its variants")
        .iter()
        .map(|v| v.as_str().unwrap().to_owned())
        .collect();
    let mut uncovered = Vec::new();
    for event in &variants {
        match event.as_str() {
            "bandShown" => day_one_case(),
            "revealLevelShown" => reveal_case(),
            "levelLineClicked" => clamp_case(),
            "badgeLineClicked" => badge_case(),
            "firstRunEnded" => first_run_case(),
            other => uncovered.push(other.to_owned()),
        }
    }
    let covered = variants.len() - uncovered.len();
    // stderr: stdout carries protocol frames, and the lint holds tests to it too.
    eprintln!("acknowledgements covered: {covered}/{}", variants.len());
    assert!(
        !variants.is_empty(),
        "the schema declared no acknowledgement"
    );
    assert!(uncovered.is_empty(), "no case for {uncovered:?}");
}
