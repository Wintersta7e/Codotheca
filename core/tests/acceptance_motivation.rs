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

use std::path::Path;
use std::sync::Arc;

use codotheca_core::badges::COMEBACK_GAP_DAYS;
use codotheca_core::clock::{local_date, Clock as _};
use codotheca_core::firstrun::first_run_completed_at;
use codotheca_core::index::migrate::{apply_all, MIGRATIONS};
use codotheca_core::index::{open_connection, Index};
use codotheca_core::ledger::price::PRICES;
use codotheca_core::motivation::keys::level_shown;
use codotheca_core::proto::dispatch::CommandHandler as _;
use codotheca_core::protocol::{LocationId, ProjectId};
use codotheca_core::scan::ScanLauncher;
use codotheca_core::session::orphan::close_orphans;
use codotheca_core::session::store::{self, SessionOpen};
use codotheca_core::testing::zone::FixedZone;
use codotheca_core::testing::{core_handler, CoreHandlerParts, HeldScanLauncher};
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use support::badges_rig::{BadgesRig, Flags, ME, T0};
use support::TestRepo;

const DAY: i64 = 86_400;
/// A fixture day well before `T0`'s, 2024-05-01.
const D1: i64 = 19_844;
/// The last migration a phase-3 core shipped, `0016_health_delta.sql`.
const PHASE3: usize = 16;
/// When the phase-3 core stored its shelf, a month before `T0`.
const PHASE3_AT: i64 = T0 - 30 * DAY;

/// Real repositories and the runners that settle them, and the core's dispatcher over the same
/// index, its clock at `T0`.
struct Live {
    rig: BadgesRig,
    _data: tempfile::TempDir,
    parts: CoreHandlerParts,
}

impl Live {
    fn new() -> Self {
        Self::over(BadgesRig::new())
    }

    /// The dispatcher over `rig`'s index.
    fn over(rig: BadgesRig) -> Self {
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

    /// A session a previous process left open on `project` at `started_at`, `credited` seconds
    /// of activity in, closed by the orphan recovery a core start runs — a production caller of
    /// the Focus credit, which adds the segment's share to its date's one `focus` row.
    fn focus(&self, project: ProjectId, started_at: i64, credited: i64) {
        let mut guard = self.rig.index.lock().unwrap();
        let location = guard
            .conn()
            .query_row(
                "SELECT id FROM location WHERE project_id = ?1",
                [project.0],
                |r| r.get(0),
            )
            .map(LocationId)
            .unwrap();
        let tx = guard.conn_mut().transaction().unwrap();
        let open = SessionOpen {
            project_id: project,
            location_id: Some(location),
            target_id: None,
            started_at,
        };
        let session = store::open_session(&tx, &open).unwrap();
        let segment = store::open_segment(&tx, session, started_at, false).unwrap();
        store::mark_activity(&tx, segment, started_at + credited).unwrap();
        tx.commit().unwrap();
        let report = close_orphans(&mut guard, T0, &FixedZone(0)).unwrap();
        drop(guard);
        assert_eq!(report.segments_closed, 1, "the orphan was not closed");
    }

    /// A library of `days` consecutive commit-days whose level was presented: day one stamped,
    /// the arrival clicked, no line pending.
    fn presented(&mut self, name: &str, days: i64) -> ProjectId {
        let project = self.read(name, &(0..days).map(|i| D1 + i).collect::<Vec<_>>());
        self.present();
        project
    }

    /// Day one stamped, the arrival read and clicked, no line pending.
    fn present(&mut self) {
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

/// `AC-P4-43-7`'s Focus clauses: a forward Focus row names the project as its receipt recorded
/// it, and Focus accrued onto a date's existing row rises with no reason.
fn focus_reasons() {
    // A forward row whose receipt records the name names the project as of the write, not as
    // it stands at the read.
    let mut renamed = Live::new();
    let project = renamed.presented("before", 11);
    let point = PRICES.focus_seconds_per_xp;
    let day = D1 + 20;
    renamed.focus(project, day * DAY + DAY / 2, 10 * point);
    renamed.rig.sql(&format!(
        "UPDATE project SET name = 'after' WHERE id = {}",
        project.0
    ));
    let named = renamed.get();
    let receipt = &named["pendingLine"]["reason"];
    eprintln!("renamed after the write: {receipt}");
    assert_eq!(form(&named), "rise", "{named}");
    assert_eq!(receipt["transitions"], 1);
    assert_eq!(receipt["term"], "focus");
    assert_eq!(receipt["projectName"], "before");
    assert_eq!(receipt["on"], local_date(day));

    // Focus accrued onto a date's existing row: a real rise no new row explains, so no reason.
    let mut focused = Live::new();
    let own = focused.read("focused", &(0..11).map(|i| D1 + i).collect::<Vec<_>>());
    let noon = (D1 + 5) * DAY + DAY / 2;
    // Nine points of Focus keep the eleven commit-days under level 2.
    focused.focus(own, noon, 9 * point);
    focused.present();
    let shown = focused.level_shown().unwrap();
    focused.focus(own, noon + 3_600, point);
    let accrued = focused.get();
    eprintln!("focus accrual: {}", accrued["pendingLine"]);
    assert_eq!(
        focused.rig.xp_event_count_of("focus"),
        1,
        "the second close wrote a row of its own"
    );
    assert_eq!(form(&accrued), "rise", "{accrued}");
    assert!(u64::from(shown) < displayed(&accrued));
    assert!(
        accrued["pendingLine"]["reason"].is_null(),
        "an accrual onto an existing row was given a reason"
    );
}

/// **AC-P4-43-7.** One forward commit-day crossing a threshold names its term, its project and
/// its date; three crossing together are counted, with no term, project or date named. A rise
/// that waited out quiet mode returns with no reason; a forward Focus row names the project as
/// its receipt recorded it; Focus accrued onto an existing row rises with no reason.
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

    // The same rise waits out quiet mode and returns as the level alone: no reason.
    one.handle(
        "settings.set",
        json!({ "patch": { "motivationShown": false } }),
    );
    assert!(one.get()["pendingLine"].is_null(), "quiet mode kept a line");
    one.handle(
        "settings.set",
        json!({ "patch": { "motivationShown": true } }),
    );
    let returned = one.get();
    eprintln!("after quiet: {}", returned["pendingLine"]);
    assert_eq!(form(&returned), "return", "{returned}");
    assert_eq!(
        returned["pendingLine"]["level"].as_u64(),
        Some(displayed(&returned))
    );
    assert!(
        returned["pendingLine"]["reason"].is_null(),
        "a return carries a reason"
    );

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

    focus_reasons();
}

/// The badge line's kind, as the band reads it.
fn badge_kind(state: &Value) -> String {
    assert_eq!(form(state), "badge", "no badge line: {state}");
    state["pendingLine"]["award"]["kind"]
        .as_str()
        .unwrap()
        .to_owned()
}

/// `AC-P4-43-6`'s last clauses: a click never lands a badge, and a landed line nobody clicked
/// leaves when the next local date's `due` names another award.
fn landed_line_leaves_on_the_next_due() {
    let mut next = Live::new();
    let p = next.presented("twice", 12);
    // Past the revival gap as well: the settle awards comeback and necromancer together.
    next.commit_days(p, &[D1 + 260 + COMEBACK_GAP_DAYS]);
    if form(&next.get()) == "rise" {
        next.click_level();
    }
    let awards = next.rig.awards();
    eprintln!("awards: {awards:?}");
    assert!(awards.len() >= 2, "the gap awarded fewer than two badges");
    let first = next.get();
    let kind = badge_kind(&first);
    next.ack("badgeLineClicked", None);
    let clicked = next.get();
    assert_eq!(badge_kind(&clicked), kind, "a click took an unlanded line");
    assert_eq!(
        clicked["pendingLine"]["award"]["landed"], false,
        "a click landed the badge"
    );
    next.handle("badges.markLanded", json!({ "kind": kind }));
    let landed = next.get();
    assert_eq!(badge_kind(&landed), kind);
    assert_eq!(landed["pendingLine"]["award"]["landed"], true);
    let today = next.parts.clock.now_unix();
    next.parts.clock.set_unix(today + DAY);
    let tomorrow = next.get();
    let other = badge_kind(&tomorrow);
    eprintln!("the next date: {kind} landed, {other} due");
    assert_ne!(
        other, kind,
        "the landed {kind} line outlived the next date's due"
    );
    assert_eq!(tomorrow["pendingLine"]["award"]["landed"], false);
}

/// **AC-P4-43-6.** Two rises before a click are one line at the current level, and its click
/// writes the level shown. A due badge — awarded by the J4 settle that read a dormancy gap —
/// waits behind a level line and returns after its click; it lands on paint, stays until its own
/// click, and leaves on it. An unlanded badge line leaves when the fortnight retires it; a click
/// never lands a badge; a landed line nobody clicked leaves when the next date's due names another.
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
    landed_line_leaves_on_the_next_due();
}

/// A database in `dir` at exactly `version` migrations, through the shipped chain.
fn migrated_to(dir: &Path, version: usize) -> Connection {
    let mut conn = open_connection(&Index::db_path(dir)).unwrap();
    apply_all(&mut conn, &MIGRATIONS[..version]).unwrap();
    conn
}

/// A project as a phase-3 core left it, stored at `created_at`: one present copy of a real
/// repository holding one of the user's commits at noon UTC on each of `days`, its history walked
/// (`j4` `ok`).
fn phase3_project(
    conn: &Connection,
    name: &str,
    days: &[i64],
    created_at: i64,
) -> (TestRepo, ProjectId, LocationId) {
    let repo = TestRepo::init();
    for &day in days {
        let date = local_date(day);
        repo.commit_at(&date, &format!("{date}T12:00:00+00:00"));
    }
    let path = repo.path().to_string_lossy().into_owned();
    conn.execute(
        "INSERT INTO project (name, seed_basename, lineage_key, authored_by_user, created_at,
                              updated_at)
         VALUES (?1, ?1, ?1, 1, ?2, ?2)",
        params![name, created_at],
    )
    .unwrap();
    let project = ProjectId(conn.last_insert_rowid());
    conn.execute(
        "INSERT INTO location (project_id, kind, path_bytes, path_key, path_display, store_key,
                               presence, repo_kind)
         VALUES (?1, 'linux', ?2, ?2, ?3, 'store', 'present', 'worktree')",
        params![project.0, path.as_bytes(), path],
    )
    .unwrap();
    let location = LocationId(conn.last_insert_rowid());
    conn.execute(
        "INSERT INTO project_committer (project_id, email, commits) VALUES (?1, ?2, ?3)",
        params![project.0, ME, days.len()],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO project_job_state (project_id, job, state, at) VALUES (?1, 'j4', 'ok', ?2)",
        params![project.0, created_at],
    )
    .unwrap();
    (repo, project, location)
}

/// **AC-P4-43-13.** A real phase-3 index, migrated through the real open path, reads first run's
/// end backfilled and no level shown. Once J4 walks one project's history again through the
/// scheduler, the payload carries an arrival over the partial coverage, with no reason; its click
/// records the level shown, and §40's best backdated award follows as a badge line.
#[test]
fn ac_p4_43_13() {
    let dir = tempfile::tempdir().unwrap();
    let conn = migrated_to(dir.path(), PHASE3);
    conn.execute(
        "INSERT INTO identity (email, is_user, source, confirmed_at)
         VALUES (?1, 1, 'manual', ?2)",
        params![ME, PHASE3_AT - DAY],
    )
    .unwrap();
    // Twelve commit-days, then one past the revival gap: a history that earned its badges
    // long before this version could award them.
    let revived = D1 + 260 + COMEBACK_GAP_DAYS;
    let mut history: Vec<i64> = (0..12).map(|i| D1 + i).collect();
    history.push(revived);
    let kept = phase3_project(&conn, "kept", &history, PHASE3_AT - DAY);
    let waiting = phase3_project(&conn, "waiting", &[D1, D1 + 1], PHASE3_AT);
    let walked = kept.1;
    drop(conn);

    let rig = BadgesRig::reopened(dir, vec![kept, waiting]);
    let guard = rig.index.lock().unwrap();
    let stamp = first_run_completed_at(guard.conn()).unwrap();
    let shown = level_shown(guard.conn()).unwrap();
    let queued: i64 = guard
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM project_job_state WHERE job = 'j4' AND state = 'queued'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    drop(guard);
    eprintln!("migrated: stamp {stamp:?}, level shown {shown:?}, {queued} history walk(s) queued");
    assert_eq!(stamp, Some(PHASE3_AT), "first run's end was not backfilled");
    assert_eq!(shown, None, "the upgrade presented a level");
    assert_eq!(queued, 2, "the upgrade did not queue every walked history");

    let mut live = Live::over(rig);
    live.ack("bandShown", None);
    let before = live.get();
    assert!(
        before["pendingLine"].is_null(),
        "a line before any history was walked again: {before}"
    );

    live.rig.settle_j4(walked);
    let arrival = live.get();
    eprintln!("after one walk: {}", arrival["level"]);
    assert_eq!(form(&arrival), "arrival", "{arrival}");
    assert!(
        arrival["pendingLine"]["reason"].is_null(),
        "an arrival carries a reason"
    );
    assert_eq!(arrival["level"]["state"], "partial");
    assert_eq!(arrival["level"]["pendingProjects"], 1);
    assert_eq!(
        arrival["pendingLine"]["level"].as_u64(),
        Some(displayed(&arrival))
    );

    live.click_level();
    assert_eq!(
        live.level_shown().map(u64::from),
        Some(displayed(&arrival)),
        "the arrival's click did not record the level shown"
    );
    let badge = live.get();
    let kind = badge_kind(&badge);
    let award = &badge["pendingLine"]["award"];
    eprintln!("then: {award}");
    assert_eq!(kind, "necromancer", "day one's first backdated kind");
    assert_eq!(award["earnedOn"], local_date(revived));
    assert_eq!(award["landed"], false);
}
