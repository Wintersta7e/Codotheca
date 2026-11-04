//! §43.17's acceptance suite: every rise is a real ledger write through the core — J4 over a real
//! repository, the badge evaluator it runs — and every payload is `motivation.get`'s answer
//! through the real dispatcher, never a hand-built one.

#![cfg(feature = "testkit")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use std::sync::Arc;

use codotheca_core::badges::COMEBACK_GAP_DAYS;
use codotheca_core::clock::{local_date, Clock as _};
use codotheca_core::motivation::keys::level_shown;
use codotheca_core::proto::dispatch::CommandHandler as _;
use codotheca_core::protocol::ProjectId;
use codotheca_core::scan::ScanLauncher;
use codotheca_core::testing::{core_handler, CoreHandlerParts, HeldScanLauncher};
use serde_json::{json, Value};
use support::badges_rig::{BadgesRig, Flags, T0};

const DAY: i64 = 86_400;
/// A fixture day well before `T0`'s, 2024-05-01.
const D1: i64 = 19_844;

/// Real repositories and the runners that settle them, and the core's dispatcher over the same
/// index, its clock at `T0`.
struct Live {
    rig: BadgesRig,
    _data: tempfile::TempDir,
    parts: CoreHandlerParts,
}

impl Live {
    fn new() -> Self {
        let rig = BadgesRig::new();
        let data = tempfile::tempdir().unwrap();
        let launcher: Arc<dyn ScanLauncher> =
            Arc::new(HeldScanLauncher::new(Arc::clone(&rig.index)));
        let parts = core_handler(data.path(), &rig.index, T0, launcher);
        Self {
            rig,
            _data: data,
            parts,
        }
    }

    fn handle(&mut self, command: &str, args: Value) -> Value {
        self.parts
            .handler
            .handle(command, args)
            .unwrap_or_else(|e| panic!("{command}: {e:?}"))
    }

    fn get(&mut self) -> Value {
        let state = self.handle("motivation.get", json!({}));
        eprintln!(
            "level {} line {}",
            state["level"]["figures"]["level"], state["pendingLine"]
        );
        state
    }

    fn ack(&mut self, event: &str, level: Option<u64>) {
        self.handle(
            "motivation.acknowledge",
            json!({ "event": event, "level": level }),
        );
    }

    /// Clicks the pending level line at the level it carries, as the band does.
    fn click_level(&mut self) {
        let line = self.get()["pendingLine"].clone();
        assert!(line["level"].is_u64(), "no level line to click: {line}");
        self.ack("levelLineClicked", line["level"].as_u64());
    }

    fn level_shown(&self) -> Option<u32> {
        let guard = self.rig.index.lock().unwrap();
        let shown = level_shown(guard.conn()).unwrap();
        drop(guard);
        shown
    }

    /// A project whose repository holds a commit on each of `days`, settled through J4: its
    /// first read, which §43.5 makes silent.
    fn read(&mut self, name: &str, days: &[i64]) -> ProjectId {
        let project = self.rig.project(name, days, Flags::default());
        self.rig.settle_j4(project);
        project
    }

    /// A forward commit-day on each of `days`, settled through J4.
    fn commit_days(&self, project: ProjectId, days: &[i64]) {
        for &day in days {
            self.rig
                .commit_file(project, &format!("f{day}.txt"), b"x\n", day);
        }
        self.rig.settle_j4(project);
    }

    /// A library of `days` consecutive commit-days whose level was presented: day one stamped,
    /// the arrival clicked, no line pending.
    fn presented(&mut self, name: &str, days: i64) -> ProjectId {
        let project = self.read(name, &(0..days).map(|i| D1 + i).collect::<Vec<_>>());
        self.ack("bandShown", None);
        let arrival = self.get();
        assert_eq!(form(&arrival), "arrival", "{arrival}");
        assert!(
            arrival["pendingLine"]["reason"].is_null(),
            "an arrival carries a reason"
        );
        self.click_level();
        let rest = self.get();
        eprintln!("presented at xp {}", rest["level"]["figures"]["xp"]);
        assert!(
            rest["pendingLine"].is_null(),
            "a clicked arrival left a line: {rest}"
        );
        project
    }
}

fn form(state: &Value) -> &str {
    state["pendingLine"]["form"].as_str().unwrap_or("none")
}

fn displayed(state: &Value) -> u64 {
    state["level"]["figures"]["level"].as_u64().unwrap()
}

/// **AC-P4-43-2-index.** A new real index, its one project not yet read, answers `notComputed`
/// with no figures — one JSON `null`, never a zero — no bar, no terms and no line. Its rings are
/// waived (day one is unwritten), so no ring carries a figure, and the rhythm's past cells read
/// not computed.
#[test]
fn ac_p4_43_2_index() {
    let mut live = Live::new();
    live.rig.project("unread", &[D1], Flags::default());
    let state = live.get();
    eprintln!("{state}");
    assert_eq!(state["shown"], true);
    assert_eq!(state["level"]["state"], "notComputed");
    assert_eq!(state["level"]["pendingProjects"], 1);
    assert!(state["level"]["figures"].is_null(), "{}", state["level"]);
    assert!(state["xpBar"].is_null());
    assert!(state["terms"].is_null(), "an unknown read as an empty list");
    assert!(state["pendingLine"].is_null());
    assert_eq!(state["rings"]["waived"], true);
    for ring in ["ship", "tend", "focus"] {
        assert!(state["rings"][ring].is_null(), "{ring} carries a figure");
    }
    let cells = state["rings"]["rhythm"]["days"].as_array().unwrap();
    let unknown = cells.iter().filter(|c| *c == "notComputed").count();
    eprintln!("rhythm cells not computed: {unknown} of {}", cells.len());
    assert!(unknown > 0, "no rhythm cell reads not computed: {cells:?}");
}

/// **AC-P4-43-7.** One forward commit-day crossing a threshold names its term, its project and
/// its date; three crossing together are counted, with no term, project or date named.
#[test]
fn ac_p4_43_7() {
    // Eleven commit-days sit just under level 2's threshold: the twelfth crosses alone.
    let mut one = Live::new();
    let project = one.presented("named", 11);
    let shown = one.level_shown().unwrap();
    let day = D1 + 40;
    one.commit_days(project, &[day]);
    let state = one.get();
    let reason = &state["pendingLine"]["reason"];
    eprintln!("one row: {reason}");
    assert_eq!(form(&state), "rise", "{state}");
    assert_eq!(reason["transitions"], 1);
    assert_eq!(reason["term"], "commitDay");
    assert_eq!(reason["projectName"], "named");
    assert_eq!(reason["on"], local_date(day));
    assert!(u64::from(shown) < displayed(&state));

    // Thirty-three sit just under level 3's: three more cross together.
    let mut three = Live::new();
    let counted = three.presented("counted", 33);
    three.commit_days(counted, &[D1 + 60, D1 + 61, D1 + 62]);
    let crossed = three.get();
    let count = &crossed["pendingLine"]["reason"];
    eprintln!("three rows: {count}");
    assert_eq!(form(&crossed), "rise", "{crossed}");
    assert_eq!(count["transitions"], 3);
    assert!(count["term"].is_null(), "a count names no term");
    assert!(count["projectName"].is_null() && count["on"].is_null());
}

/// The badge line's kind, as the band reads it.
fn badge_kind(state: &Value) -> String {
    assert_eq!(form(state), "badge", "no badge line: {state}");
    state["pendingLine"]["award"]["kind"]
        .as_str()
        .unwrap()
        .to_owned()
}

/// **AC-P4-43-6.** Two rises before a click are one line at the current level, and its click
/// writes the level shown. A due badge — awarded by the J4 settle that read a dormancy gap —
/// waits behind a level line and returns after its click; it lands on paint, stays until its own
/// click, and leaves on it. An unlanded badge line leaves when the fortnight retires it.
#[test]
fn ac_p4_43_6() {
    let mut live = Live::new();
    let project = live.presented("rising", 12);
    let shown = live.level_shown().unwrap();

    // Two crossings, no click between them: one line, at the level now displayed.
    let mut day = D1 + 20;
    let mut rises = 0;
    let mut last = live.get();
    while rises < 2 {
        assert!(day < D1 + 200, "two crossings never happened");
        let before = displayed(&last);
        live.commit_days(project, &[day]);
        last = live.get();
        if displayed(&last) > before {
            rises += 1;
        }
        day += 1;
    }
    eprintln!(
        "shown {shown}, displayed {}, line {}",
        displayed(&last),
        last["pendingLine"]
    );
    assert_eq!(form(&last), "rise");
    assert_eq!(
        last["pendingLine"]["level"].as_u64(),
        Some(displayed(&last))
    );
    assert!(displayed(&last) >= u64::from(shown) + 2);

    // A dormancy gap: the settle awards its badges while the rise still pends.
    live.commit_days(project, &[D1 + 260 + COMEBACK_GAP_DAYS]);
    let awards = live.rig.awards();
    eprintln!("awards: {awards:?}");
    assert!(!awards.is_empty(), "the J4 settle awarded nothing");
    let behind = live.get();
    assert_eq!(form(&behind), "rise", "the badge did not wait: {behind}");
    live.click_level();
    assert_eq!(
        live.level_shown().map(u64::from),
        Some(displayed(&behind)),
        "the click wrote the level shown"
    );

    let badge = live.get();
    let kind = badge_kind(&badge);
    assert_eq!(badge["pendingLine"]["award"]["landed"], false);
    live.handle("badges.markLanded", json!({ "kind": kind }));
    let landed = live.get();
    assert_eq!(
        badge_kind(&landed),
        kind,
        "a landed badge left before its click"
    );
    assert_eq!(landed["pendingLine"]["award"]["landed"], true);
    live.ack("badgeLineClicked", None);
    let after = live.get();
    eprintln!("after the {kind} click: {}", after["pendingLine"]);
    assert!(
        after["pendingLine"].is_null() || badge_kind(&after) != kind,
        "the click did not take the {kind} line"
    );

    // An unlanded badge, and the fortnight retiring it.
    let mut retired = Live::new();
    let p = retired.presented("returned", 12);
    retired.commit_days(p, &[D1 + 60 + COMEBACK_GAP_DAYS]);
    if form(&retired.get()) == "rise" {
        retired.click_level();
    }
    let unlanded = retired.get();
    badge_kind(&unlanded);
    let awarded = retired.parts.clock.now_unix();
    retired.parts.clock.set_unix(awarded + 14 * DAY);
    let late = retired.get();
    eprintln!("a fortnight on: {}", late["pendingLine"]);
    assert!(
        late["pendingLine"].is_null(),
        "the fortnight did not retire the unlanded badge"
    );
}
