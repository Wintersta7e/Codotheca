//! §40's acceptance suite: the badge types on the wire, read from the schema itself, and each
//! award driven through its production caller.
//!
//! Every test here prints what it read and fails when it read nothing, so a renamed type or an
//! emptied field set cannot pass by scanning zero fields.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use codotheca_core::badges::{COMEBACK_GAP_DAYS, CURATOR_PROJECTS, MARATHON_CREDITED_SECONDS};
use codotheca_core::cancel::CancelToken;
use codotheca_core::clock::local_date;
use codotheca_core::commands::launch::{startup, LaunchCtx};
use codotheca_core::detail::{dispatch_detail_command, DetailCtx};
use codotheca_core::git::{RepoHandle, StoreKey};
use codotheca_core::index::rebuild::{rebuild_in_place, RebuildOutcome};
use codotheca_core::index::sidecar::{counts, read, RestoreCtx, RestoreOutcome, SECTIONS};
use codotheca_core::index::Index;
use codotheca_core::jobs::NullJobSink;
use codotheca_core::launch::spawn::RecordingSpawner;
use codotheca_core::ledger::price::REVIVAL_GAP_DAYS;
use codotheca_core::mount::StoreClass;
use codotheca_core::protocol::{ProjectId, SessionId};
use codotheca_core::provider::GitHubProvider;
use codotheca_core::session::activity::FakeIgnoreCheck;
use codotheca_core::session::manager::{LaunchedSession, SessionManager};
use codotheca_core::session::watch::{ActivityBatch, ActivitySource, FakeActivitySource};
use codotheca_core::session::SessionError;
use codotheca_core::sync::http::ObservingTransport;
use codotheca_core::sync::runner::{NullSyncSink, SyncRunner};
use codotheca_core::sync::task::SyncTask;
use codotheca_core::sync::SyncDeps;
use codotheca_core::testing::events::ValidatingSink;
use codotheca_core::testing::zone::FixedZone;
use codotheca_core::testing::{
    FakeClock, FakeGitBackend, FakeMountResolver, FakeTokenStore, FakeTransport,
};
use rusqlite::types::Value;
use rusqlite::Connection;
use support::badges_rig::{
    award_row, connect_ci, orphaned_session, plant_repairs, red_then_green, repair_pairs,
    session_index, session_row, BadgesRig, Flags, T0,
};

const SCHEMA: &str = include_str!("../../protocol/schema/protocol.json");
const NOW: i64 = 1_760_000_000;

/// A local day number in early 2024: the first commit-day of every gap below.
const D1: i64 = 19_724;

/// What one drive left behind over one fixture.
#[derive(Debug)]
struct TwinOutcome {
    /// Every `badge_award` row before the drive.
    before: Vec<Vec<Value>>,
    /// Every `badge_award` row after it.
    awards: Vec<Vec<Value>>,
    /// `SELECT COUNT(*) FROM xp_events` after it.
    xp_events: i64,
    /// Every event the runners emitted, as `topic/event`.
    events: Vec<String>,
}

/// The same drive over two fresh fixtures, identical but for a pre-latched row of `kind` in the
/// second.
#[derive(Debug)]
struct Twin {
    fresh: TwinOutcome,
    latched: TwinOutcome,
}

fn run_once<S>(
    kind: Option<&str>,
    fixture: &impl Fn() -> (BadgesRig, S),
    drive: &impl Fn(&BadgesRig, &S),
) -> TwinOutcome {
    let (rig, state) = fixture();
    if let Some(kind) = kind {
        rig.sql(&format!(
            "INSERT INTO badge_award (kind, earned_at, tz_offset_min, earned_on, evidence,
                                      awarded_at, awarded_on)
             VALUES ('{kind}', 0, 0, '1970-01-01', '{{}}', 0, '1970-01-01')"
        ));
    }
    let before = rig.awards();
    drive(&rig, &state);
    let events = rig
        .events
        .events
        .lock()
        .unwrap()
        .iter()
        .map(|(topic, event, _)| format!("{topic}/{event}"))
        .collect();
    TwinOutcome {
        before,
        awards: rig.awards(),
        xp_events: rig.xp_event_count(),
        events,
    }
}

/// `drive` over `fixture`, fresh and with `kind` pre-latched.
fn twin<S>(
    kind: &str,
    fixture: impl Fn() -> (BadgesRig, S),
    drive: impl Fn(&BadgesRig, &S),
) -> Twin {
    Twin {
        fresh: run_once(None, &fixture, &drive),
        latched: run_once(Some(kind), &fixture, &drive),
    }
}

/// A project whose two commit-days sit §38's dormancy constant apart, settled by a real J4.
fn necromancer_twin() -> &'static Twin {
    static OUTCOME: OnceLock<Twin> = OnceLock::new();
    OUTCOME.get_or_init(|| {
        twin(
            "necromancer",
            || {
                let mut rig = BadgesRig::new();
                let project =
                    rig.project("revived", &[D1, D1 + REVIVAL_GAP_DAYS], Flags::default());
                (rig, project)
            },
            |rig: &BadgesRig, project: &ProjectId| rig.settle_j4(*project),
        )
    })
}

/// The field names of one declared struct, in declaration order.
fn fields_of(name: &str) -> Vec<String> {
    let doc: serde_json::Value = serde_json::from_str(SCHEMA).unwrap();
    let Some(decl) = doc["types"].get(name) else {
        panic!("{name} is not a declared type");
    };
    let fields = decl["fields"]
        .as_object()
        .unwrap_or_else(|| panic!("{name} declares no fields"));
    let names: Vec<String> = fields.keys().cloned().collect();
    eprintln!("{name}: {} fields {names:?}", names.len());
    assert!(!names.is_empty(), "{name} declares zero fields");
    names
}

/// Field names any of `words` occurs in, case-insensitively.
fn naming_any<'a>(fields: &'a [String], words: &[&str]) -> Vec<&'a str> {
    fields
        .iter()
        .filter(|f| {
            let lower = f.to_lowercase();
            words.iter().any(|w| lower.contains(w))
        })
        .map(String::as_str)
        .collect()
}

/// §40.10: a badge is awarded or it is absent. No type on the wire can carry how near an
/// unearned badge is, so the sets are pinned exactly and a later field fails here for a human
/// to read.
#[test]
fn ac_p4_40_6_the_badge_types_carry_no_progress() {
    const PROGRESS: &[&str] = &[
        "threshold",
        "progress",
        "remaining",
        "toward",
        "unearned",
        "locked",
        "length",
        "count",
        "total",
    ];
    let expected: [(&str, &[&str]); 4] = [
        (
            "BadgeAward",
            &["kind", "earnedOn", "projectId", "evidence", "landed"],
        ),
        (
            "BadgeEvidence",
            &[
                "projectName",
                "gapDays",
                "year",
                "languages",
                "creditedSeconds",
                "projects",
                "runStartOn",
            ],
        ),
        ("BadgeEvidenceProject", &["name", "evaluable"]),
        ("BadgesListed", &["awards", "due", "basis"]),
    ];
    let mut read = 0;
    for (name, want) in expected {
        let fields = fields_of(name);
        read += fields.len();
        let named = naming_any(&fields, PROGRESS);
        assert!(named.is_empty(), "{name} carries progress: {named:?}");
        let got: BTreeSet<&str> = fields.iter().map(String::as_str).collect();
        let want: BTreeSet<&str> = want.iter().copied().collect();
        assert_eq!(got, want, "{name}'s fields are not §40.10's set");
    }
    eprintln!("{read} fields read across 4 types");
    assert!(read > 0, "read zero fields");
}

/// §40.7: evidence names what was earned, never a volume of lines or commits.
#[test]
fn ac_p4_40_17_evidence_names_no_line_or_commit() {
    const VOLUME: &[&str] = &["line", "insertion", "deletion", "commit"];
    let mut read = 0;
    for name in ["BadgeEvidence", "BadgeEvidenceProject"] {
        let fields = fields_of(name);
        read += fields.len();
        let named = naming_any(&fields, VOLUME);
        assert!(named.is_empty(), "{name} names a volume: {named:?}");
    }
    eprintln!("{read} evidence fields read");
    assert!(read > 0, "read zero fields");
}

/// Every row of `badge_award`, every column, by kind.
fn awards(conn: &Connection) -> Vec<Vec<Value>> {
    let mut st = conn
        .prepare("SELECT * FROM badge_award ORDER BY kind")
        .unwrap();
    let width = st.column_count();
    st.query_map([], |r| (0..width).map(|i| r.get::<_, Value>(i)).collect())
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

/// The position of `project_id` in `SELECT *`.
fn project_column(conn: &Connection) -> usize {
    conn.prepare("SELECT * FROM badge_award")
        .unwrap()
        .column_names()
        .iter()
        .position(|name| *name == "project_id")
        .expect("badge_award has project_id")
}

/// Corrupt the index in `dir` and rebuild it through the production entry.
fn corrupt_and_rebuild(dir: &Path, now: i64) -> Index {
    std::fs::write(Index::db_path(dir), b"this is not a database").unwrap();
    match rebuild_in_place(dir, now) {
        Ok(RebuildOutcome::Rebuilt(report)) => {
            eprintln!("rebuilt: restored {:?}", report.restored);
        }
        other => panic!("expected a rebuild, got {other:?}"),
    }
    Index::open_at(dir, now + 1).unwrap()
}

/// §40.7, §48.8.3: awards travel as a global latch section. Every column comes back but
/// `project_id`, which waits for its subject; a second restore over held awards writes nothing.
#[test]
fn ac_p4_40_19_every_column_but_the_project_restores_and_a_second_restore_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let index = Index::open_at(dir.path(), NOW).unwrap();
    let conn = index.conn();
    conn.execute(
        "INSERT INTO project (name, seed_basename, lineage_key, created_at, updated_at)
         VALUES ('awarded', 'awarded', 'ee', 1, 1)",
        [],
    )
    .unwrap();
    let project = conn.last_insert_rowid();
    conn.execute_batch(&format!(
        "INSERT INTO badge_award (kind, earned_at, tz_offset_min, earned_on, project_id,
                                  subject_key, evidence, awarded_at, awarded_on)
         VALUES ('necromancer', 86400, 0, '1970-01-02', {project}, 'lineage:ee|remote:',
                 '{{\"projectName\":\"awarded\",\"gapDays\":200}}', 90000, '1970-01-02'),
                ('lifeline', 172800, 60, '1970-01-03', NULL, NULL,
                 '{{\"runStartOn\":\"1969-12-04\"}}', 180000, '1970-01-03'),
                ('polyglot', 0, 0, '1970-01-01', NULL, NULL,
                 '{{\"year\":1970}}', 90000, '1970-01-02');
         UPDATE badge_award SET landed_at = 190000, landed_on = '1970-01-03'
          WHERE kind = 'lifeline';"
    ))
    .unwrap();
    let before = awards(conn);
    let at = project_column(conn);
    assert_eq!(before.len(), 3);
    assert_eq!(
        before[1][at],
        Value::Integer(project),
        "necromancer names it"
    );

    let doc = read(&index.export_sidecar(NOW + 10).unwrap().path).unwrap();
    let exported = counts(&doc).get("badge_award").copied();
    eprintln!("rows exported: {exported:?}");
    assert_eq!(exported, Some(3));
    drop(index);

    let mut rebuilt = corrupt_and_rebuild(dir.path(), NOW + 20);
    let after = awards(rebuilt.conn());
    eprintln!("rows restored: {}", after.len());
    assert_eq!(after.len(), before.len(), "every award came back");
    for (was, now) in before.iter().zip(&after) {
        assert_eq!(now[at], Value::Null, "project_id waits for its subject");
        let mut expected = was.clone();
        expected[at] = Value::Null;
        assert_eq!(*now, expected, "every other column restores");
    }

    let spec = SECTIONS.iter().find(|s| s.name == "badge_award").unwrap();
    let rows = &doc.payload.sections["badge_award"];
    let written = rebuilt
        .with_tx(|tx| {
            let ctx = RestoreCtx {
                now: NOW + 30,
                project: None,
                locations: &[],
                source_generation: 1,
            };
            let mut written = 0;
            for row in rows {
                if let RestoreOutcome::Applied(n) = (spec.restore)(tx, row, &ctx)? {
                    written += n;
                } else {
                    panic!("a Global section answered Pending");
                }
            }
            Ok(written)
        })
        .unwrap();
    eprintln!("second restore wrote {written} rows");
    assert_eq!(written, 0);
    assert_eq!(
        awards(rebuilt.conn()),
        after,
        "a second restore changed a byte"
    );
}

/// A project row, named, with no location — §23's not-cloned shape until one is planted.
fn plant_project(conn: &Connection, name: &str) -> i64 {
    conn.execute(
        "INSERT INTO project (name, seed_basename, created_at, updated_at)
         VALUES (?1, ?1, 1, 1)",
        [name],
    )
    .unwrap();
    conn.last_insert_rowid()
}

/// A location for `project`, removed at `removed_at` when given.
fn plant_location(conn: &Connection, project: i64, removed_at: Option<i64>) {
    let path = format!("/srv/copies/{project}");
    conn.execute(
        "INSERT INTO location (project_id, kind, path_bytes, path_key, path_display, store_key,
                               presence, repo_kind, removed_at)
         VALUES (?1, 'linux', ?2, ?2, ?3, 'store', 'present', 'worktree', ?4)",
        rusqlite::params![project, path.as_bytes(), path, removed_at],
    )
    .unwrap();
}

/// `project`'s J4 row in `state`, with `reason`.
fn plant_j4(conn: &Connection, project: i64, state: &str, reason: Option<&str>) {
    conn.execute(
        "INSERT INTO project_job_state (project_id, job, state, fail_count, reason, at)
         VALUES (?1, 'j4', ?2, 0, ?3, 1)",
        rusqlite::params![project, state, reason],
    )
    .unwrap();
}

/// `historyComplete` as the `stats.reveal` handler returns it.
fn history_complete(conn: &Connection) -> bool {
    let out = codotheca_core::stats::reveal::reveal(conn, NOW).unwrap();
    out.span_days.basis.history_complete
}

/// §40.11: a project with no present location cannot run J4, so it cannot hold history
/// incomplete; a migration re-queue keeps the days its walk wrote, so it counts as covered.
#[test]
fn ac_p4_40_21_a_not_cloned_project_does_not_hold_history_incomplete() {
    let dir = tempfile::tempdir().unwrap();
    let index = Index::open_at(dir.path(), NOW).unwrap();
    let conn = index.conn();
    for name in ["one", "two", "three"] {
        let p = plant_project(conn, name);
        plant_location(conn, p, None);
        plant_j4(conn, p, "ok", None);
    }
    plant_project(conn, "not-cloned");
    let walked_and_not_cloned = history_complete(conn);
    eprintln!("three walked + one not cloned: historyComplete = {walked_and_not_cloned}");
    assert!(walked_and_not_cloned, "a not-cloned project held it false");

    let gone = plant_project(conn, "gone");
    plant_location(conn, gone, Some(NOW - 10));
    let with_removed = history_complete(conn);
    eprintln!("+ one whose only location is removed: historyComplete = {with_removed}");
    assert!(with_removed, "a removed location held it false");

    let upgraded = plant_project(conn, "upgraded");
    plant_location(conn, upgraded, None);
    plant_j4(conn, upgraded, "queued", Some("app_upgraded"));
    let with_requeue = history_complete(conn);
    eprintln!("+ one re-queued app_upgraded: historyComplete = {with_requeue}");
    assert!(
        with_requeue,
        "the migration re-queue did not count as covered"
    );

    let asked = plant_project(conn, "asked");
    plant_location(conn, asked, None);
    plant_j4(conn, asked, "queued", Some("user_requested"));
    let with_user_requeue = history_complete(conn);
    eprintln!("+ one queued user_requested: historyComplete = {with_user_requeue}");
    assert!(
        !with_user_requeue,
        "a user's re-queue counted as covered history"
    );
}

/// §40.3, §40.5: Necromancer is awarded through the J4 settle, its production caller, never by
/// calling the evaluator: earned on the second date, naming the project, with the gap as
/// evidence. Over the same library with Necromancer already held, the settle writes nothing.
#[test]
fn ac_p4_40_1_necromancer_through_the_j4_settle() {
    let twin = necromancer_twin();
    eprintln!(
        "fresh: {} award rows, {} xp_events, events {:?}",
        twin.fresh.awards.len(),
        twin.fresh.xp_events,
        twin.fresh.events
    );
    eprintln!(
        "latched: {} award rows, {} xp_events, events {:?}",
        twin.latched.awards.len(),
        twin.latched.xp_events,
        twin.latched.events
    );
    assert_eq!(twin.fresh.before, Vec::<Vec<Value>>::new());
    // The fixture's gap is past Comeback's too, so the settle writes that row beside it.
    let fresh = of_kind(&twin.fresh.awards, "necromancer");
    assert_eq!(fresh.len(), 1, "the settle wrote no necromancer");
    // `SELECT *` in §40.7's column order.
    let [kind, earned_at, tz_offset_min, earned_on, project_id, subject_key, evidence, ..] =
        fresh[0].as_slice()
    else {
        panic!("badge_award has fewer columns than §40.7 declares");
    };
    let d2 = D1 + REVIVAL_GAP_DAYS;
    assert_eq!(*kind, Value::Text("necromancer".to_owned()));
    assert_eq!(*earned_at, Value::Integer(d2 * 86_400));
    assert_eq!(*tz_offset_min, Value::Integer(0));
    assert_eq!(*earned_on, Value::Text(local_date(d2)));
    assert_eq!(*project_id, Value::Integer(1));
    assert_eq!(
        *subject_key,
        Value::Text("lineage:revived|remote:".to_owned())
    );
    let Value::Text(evidence) = evidence else {
        panic!("evidence is not text");
    };
    let evidence: serde_json::Value = serde_json::from_str(evidence).unwrap();
    assert_eq!(evidence["gapDays"], REVIVAL_GAP_DAYS);
    assert_eq!(evidence["projectName"], "revived");

    assert_eq!(
        of_kind(&twin.latched.awards, "necromancer"),
        of_kind(&twin.latched.before, "necromancer"),
        "a held kind was evaluated again"
    );
}

/// The rows of `kind` among `rows`, which are `SELECT *` with the kind first.
fn of_kind(rows: &[Vec<Value>], kind: &str) -> Vec<Vec<Value>> {
    rows.iter()
        .filter(|row| row.first() == Some(&Value::Text(kind.to_owned())))
        .cloned()
        .collect()
}

/// `project`'s `primary_language`, as a finished J3 would have written it.
fn set_language(rig: &BadgesRig, project: ProjectId, language: &str) {
    rig.sql(&format!(
        "UPDATE project SET primary_language = '{language}' WHERE id = {}",
        project.0
    ));
}

/// §40.3, §40.5: Polyglot through both settles that feed it. Four projects of **H** carry four
/// languages; the fifth project's language is not computed until its J3 settles, and that
/// settle awards — earned on the commit-day that brought the fifth language, which is not the
/// fifth project's own day.
#[test]
fn ac_p4_40_1_polyglot_through_the_j4_and_j3_settles() {
    let mut rig = BadgesRig::new();
    let planted = [
        ("go", D1 + 1, Some("Go")),
        ("python", D1 + 2, Some("Python")),
        ("ruby", D1 + 3, Some("Ruby")),
        ("rust", D1 + 5, None),
        ("c", D1 + 10, Some("C")),
    ];
    let mut projects = Vec::new();
    for (name, day, language) in planted {
        let project = rig.project(name, &[day], Flags::default());
        if let Some(language) = language {
            set_language(&rig, project, language);
        } else {
            rig.commit_file(project, "src/main.rs", b"fn main() {}\n", day);
        }
        projects.push(project);
    }
    for &project in &projects {
        rig.settle_j4(project);
    }
    let after_j4 = rig.award("polyglot");
    eprintln!("after five J4 settles, four languages: {after_j4:?}");
    assert_eq!(after_j4, None, "four languages awarded polyglot");

    rig.settle_j3(projects[3]);
    let language: String = {
        let guard = rig.index.lock().unwrap();
        let language = guard
            .conn()
            .query_row(
                "SELECT primary_language FROM project WHERE id = ?1",
                [projects[3].0],
                |r| r.get(0),
            )
            .unwrap();
        drop(guard);
        language
    };
    let award = rig.award("polyglot");
    eprintln!("J3 wrote {language}; award {award:?}");
    assert_eq!(language, "Rust");
    let (earned_on, project, evidence) = award.expect("the J3 settle wrote no polyglot");
    assert_eq!(earned_on, local_date(D1 + 10));
    assert_eq!(project, None);
    assert_eq!(evidence["year"], 2024);
    assert_eq!(
        evidence["languages"],
        serde_json::json!(["Go", "Python", "Ruby", "Rust", "C"])
    );
}

/// §40.3, §40.5: Comeback through the J4 settle — two commit-days `COMEBACK_GAP_DAYS` apart,
/// every present-location project walked. Day-grain, naming no project, the gap as evidence.
#[test]
fn ac_p4_40_1_comeback_through_the_j4_settle() {
    let mut rig = BadgesRig::new();
    let project = rig.project("returned", &[D1, D1 + COMEBACK_GAP_DAYS], Flags::default());
    rig.settle_j4(project);
    let award = rig.award("comeback");
    eprintln!(
        "historyComplete {}, award {award:?}",
        rig.history_complete()
    );
    let (earned_on, project_id, evidence) = award.expect("the J4 settle wrote no comeback");
    assert_eq!(earned_on, local_date(D1 + COMEBACK_GAP_DAYS));
    assert_eq!(project_id, None);
    assert_eq!(evidence["gapDays"], COMEBACK_GAP_DAYS);
    assert_eq!(
        evidence.as_object().map(serde_json::Map::len),
        Some(1),
        "comeback carries the gap and nothing else"
    );
}

/// §40.11's second clause: beside a not-cloned project (§23 — a row with no location, which
/// never runs J4), history is complete and Comeback is awarded.
#[test]
fn ac_p4_40_21_comeback_is_awarded_beside_a_not_cloned_project() {
    let mut rig = BadgesRig::new();
    rig.sql(
        "INSERT INTO project (name, seed_basename, created_at, updated_at)
         VALUES ('not-cloned', 'not-cloned', 0, 0)",
    );
    let project = rig.project("returned", &[D1, D1 + COMEBACK_GAP_DAYS], Flags::default());
    rig.settle_j4(project);
    let complete = rig.history_complete();
    let award = rig.award("comeback");
    eprintln!("historyComplete {complete}, award {award:?}");
    assert!(complete, "a not-cloned project held history incomplete");
    assert!(award.is_some(), "no comeback beside a not-cloned project");
}

/// The session tests' first instant, 2023-11-14 22:13:20 UTC: four hours on is the next UTC
/// day and still the same day five hours west.
const T_SESSION: i64 = 1_700_000_000;

/// Five hours west of UTC, in minutes east.
const WEST: i32 = -300;

/// The session manager's tick.
const TICK: i64 = 15;

/// A hand-driven activity source the test keeps a handle on once the manager owns it.
#[derive(Debug, Clone, Default)]
struct SharedActivity(Arc<Mutex<FakeActivitySource>>);

impl ActivitySource for SharedActivity {
    fn watch(&mut self, session: SessionId, root: &Path) -> Result<(), SessionError> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .watch(session, root)
    }

    fn unwatch(&mut self, session: SessionId) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .unwatch(session);
    }

    fn drain(&mut self) -> Vec<ActivityBatch> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .drain()
    }
}

/// §40.3, §40.5: Marathon through the session manager's close. A launched session saved into
/// every tick for `MARATHON_CREDITED_SECONDS`, then stopped, writes the row in the close's
/// transaction: earned at the session's end, dated by the zone at that instant.
#[test]
fn ac_p4_40_1_marathon_through_close_session() {
    let mut lib = session_index();
    let clock = Arc::new(FakeClock::new(T_SESSION));
    let activity = SharedActivity::default();
    let mut manager = SessionManager::new(
        clock.clone(),
        Arc::new(ValidatingSink::default()),
        Box::new(activity.clone()),
        Arc::new(FakeIgnoreCheck::new(&["dist/"])),
        Arc::new(FixedZone(WEST)),
    );
    let session = manager
        .launch(
            lib.index.index_mut(),
            LaunchedSession {
                project_id: lib.project,
                location_id: Some(lib.location),
                target_id: Some(lib.target),
                repo: RepoHandle::bare(
                    Path::new("/w/marathon"),
                    StoreKey::new("s"),
                    StoreClass::Local,
                ),
                waiter: None,
            },
        )
        .unwrap();
    for _ in 0..MARATHON_CREDITED_SECONDS / TICK {
        activity.0.lock().unwrap().push(session, "src/main.rs");
        clock.advance(TICK);
        manager.tick(lib.index.index_mut()).unwrap();
    }
    let before = award_row(lib.index.index().conn(), "marathon");
    manager.stop(lib.index.index_mut(), session).unwrap();
    let (ended_at, credited) = session_row(&lib.index, session);
    let award = award_row(lib.index.index().conn(), "marathon");
    eprintln!("open: {before:?}; closed at {ended_at:?} credited {credited}: {award:?}");
    assert_eq!(before, None, "a session still open was awarded");
    assert_eq!(credited, MARATHON_CREDITED_SECONDS);
    let award = award.expect("the close wrote no marathon");
    assert_eq!(Some(award.earned_at), ended_at);
    assert_eq!(award.tz_offset_min, i64::from(WEST));
    assert_eq!(award.earned_on, "2023-11-14", "not dated in the zone");
    assert_eq!(award.project_id, Some(lib.project.0));
    assert_eq!(
        award.subject_key.as_deref(),
        Some("lineage:marathon|remote:")
    );
    assert_eq!(
        award.evidence,
        serde_json::json!({"projectName": "marathon", "creditedSeconds": credited})
    );
}

/// §40.3, §40.5: Marathon through the second close path — the startup lane's orphan recovery.
/// A session a crash left open with `MARATHON_CREDITED_SECONDS` of credit is closed at its
/// watermark, never at now, and that close writes the row.
#[test]
fn ac_p4_40_1_marathon_through_close_orphans() {
    let mut lib = session_index();
    let session = orphaned_session(&mut lib, T_SESSION, MARATHON_CREDITED_SECONDS);
    let later = T_SESSION + 3 * 86_400;
    let clock = Arc::new(FakeClock::new(later));
    let events = Arc::new(ValidatingSink::default());
    let mut sessions = SessionManager::new(
        clock,
        events.clone(),
        Box::new(FakeActivitySource::new()),
        Arc::new(FakeIgnoreCheck::new(&["dist/"])),
        Arc::new(FixedZone(WEST)),
    );
    let spawner = RecordingSpawner::new();
    let mounts = FakeMountResolver::new();
    let report = startup(&mut LaunchCtx {
        index: lib.index.index_mut(),
        sessions: &mut sessions,
        spawner: &spawner,
        events: events.as_ref(),
        mounts: &mounts,
        now: later,
    })
    .unwrap();
    let (ended_at, credited) = session_row(&lib.index, session);
    let award = award_row(lib.index.index().conn(), "marathon");
    eprintln!("{report:?}; closed at {ended_at:?} credited {credited}: {award:?}");
    assert_eq!(report.sessions_closed, 1);
    let award = award.expect("the orphan close wrote no marathon");
    assert_eq!(award.earned_at, T_SESSION + MARATHON_CREDITED_SECONDS);
    assert_eq!(Some(award.earned_at), ended_at);
    assert_eq!(award.earned_on, "2023-11-14");
    assert_eq!(award.awarded_at, later);
    assert_eq!(award.project_id, Some(lib.project.0));
    assert_eq!(award.evidence["creditedSeconds"], MARATHON_CREDITED_SECONDS);
}

/// Nine projects of **P** already at §31's 100%, and a tenth carrying the facts §31's checks
/// read, its CI config answer `ci` — the library each Curator site test completes.
fn nine_at_gold_and(rig: &mut BadgesRig, ci: &str) -> ProjectId {
    let nine: Vec<(ProjectId, i64)> = (1..CURATOR_PROJECTS)
        .map(|i| {
            (
                rig.project(&format!("gold{i}"), &[D1], Flags::default()),
                10,
            )
        })
        .collect();
    rig.at_top_rung(&nine);
    let tenth = rig.project("tenth", &[D1], Flags::default());
    rig.completion_facts(tenth, ci);
    tenth
}

/// The names Curator's evidence counted, and how many evaluable checks each had.
fn curator_counted(rig: &BadgesRig) -> Option<Vec<(String, i64)>> {
    rig.award("curator")
        .map(|(_, project, evidence)| listed(project, &evidence))
}

/// A Curator row's evidence, as `(name, evaluable)` per project it lists.
fn listed(project: Option<i64>, evidence: &serde_json::Value) -> Vec<(String, i64)> {
    assert_eq!(project, None, "curator names no one project");
    let counted: Vec<(String, i64)> = evidence["projects"]
        .as_array()
        .expect("curator evidence lists its projects")
        .iter()
        .map(|p| {
            (
                p["name"].as_str().unwrap().to_owned(),
                p["evaluable"].as_i64().unwrap(),
            )
        })
        .collect();
    eprintln!("curator counted {} projects: {counted:?}", counted.len());
    counted
}

/// The tenth project at gold, as the evidence lists it: by its name, with its evaluable count.
fn tenth_listed(rig: &BadgesRig, tenth: ProjectId, counted: &[(String, i64)]) {
    let (lit, evaluable) = rig.projection(tenth);
    eprintln!("tenth: {lit:?} of {evaluable:?}");
    assert!(
        lit.is_some() && lit == evaluable,
        "the tenth is not at gold"
    );
    assert_eq!(counted.len(), CURATOR_PROJECTS);
    assert!(counted.contains(&("tenth".to_owned(), evaluable.unwrap())));
}

/// The job settle's Curator site: the tenth project reaches its top rung through a real J2.
#[test]
fn ac_p4_40_1_curator_through_evaluate_and_write() {
    let mut rig = BadgesRig::new();
    let tenth = nine_at_gold_and(&mut rig, "present");
    assert_eq!(curator_counted(&rig), None, "nine is not ten");
    rig.settle_j2(tenth);
    let counted = curator_counted(&rig).expect("the settle wrote no curator");
    tenth_listed(&rig, tenth, &counted);
}

/// The `projects.setCheckNa` site: the tenth project's one failing check, its CI config, ruled
/// not applicable through the command, lifts it to gold, and the row is there once it commits.
#[test]
fn ac_p4_40_1_curator_through_set_check_na() {
    let mut rig = BadgesRig::new();
    let tenth = nine_at_gold_and(&mut rig, "absent");
    rig.settle_j2(tenth);
    let (lit, evaluable) = rig.projection(tenth);
    eprintln!("tenth after its settle: {lit:?} of {evaluable:?}");
    assert!(lit < evaluable, "the fixture must start below gold");
    assert_eq!(curator_counted(&rig), None, "nine is not ten");

    let guard = rig.index.lock().unwrap();
    let ctx = DetailCtx {
        index: &guard,
        git: &FakeGitBackend::new(),
        mount: &FakeMountResolver::new(),
        events: rig.events.as_ref(),
        jobs: &NullJobSink,
        sync: &NullSyncSink,
        now: T0,
        zone: &FixedZone(0),
    };
    let args = serde_json::json!({ "id": tenth.0, "check": "ci", "na": true });
    dispatch_detail_command(&ctx, "projects.setCheckNa", args)
        .expect("the detail module owns this command")
        .expect("the ruling was refused");
    drop(guard);
    let counted = curator_counted(&rig).expect("the ruling wrote no curator");
    tenth_listed(&rig, tenth, &counted);
}

/// The sync completion hook's site: a sync settle over the tenth project recomputes it onto
/// facts that moved since its last settle — its CI config is now present — and lifts it to gold.
#[test]
fn ac_p4_40_1_curator_through_the_sync_completion_hook() {
    let mut rig = BadgesRig::new();
    let tenth = nine_at_gold_and(&mut rig, "absent");
    rig.settle_j2(tenth);
    rig.completion_facts(tenth, "present");
    let (lit, evaluable) = rig.projection(tenth);
    eprintln!("tenth before the sync: {lit:?} of {evaluable:?}");
    assert!(lit < evaluable, "the fixture must start below gold");
    assert_eq!(curator_counted(&rig), None, "nine is not ten");

    let clock = Arc::new(FakeClock::new(T0));
    let observing = Arc::new(ObservingTransport::new(
        Arc::new(FakeTransport::new()),
        clock.clone(),
    ));
    let deps = SyncDeps {
        provider: Arc::new(GitHubProvider::new(
            observing.clone(),
            "forge.example.invalid".to_owned(),
        )),
        transport: observing,
        tokens: Arc::new(FakeTokenStore::available()),
        clock,
        cancel: CancelToken::new(),
        zone: Arc::new(FixedZone(0)),
    };
    let runner = SyncRunner::new(Arc::clone(&rig.index), deps, rig.events.clone());
    runner.enqueue(SyncTask::ProjectRemote { project_id: tenth });
    runner.start();
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline && rig.award("curator").is_none() {
        std::thread::sleep(Duration::from_millis(25));
    }
    runner.request_stop();
    runner.join();
    let counted = curator_counted(&rig).expect("the sync settle wrote no curator");
    tenth_listed(&rig, tenth, &counted);
}

/// Seconds in a day, and in an hour.
const DAY_S: i64 = 86_400;
const HOUR_S: i64 = 3_600;

/// The local day of Firefighter's tenth pair in the payout tests: nine planted before it.
const TENTH: i64 = D1 + 9;

/// A library of two projects of **H** on real repositories, each walked by J4: `alpha`
/// committed on nine days and `beta` long before them — and on [`TENTH`] too when `committed`.
/// Both are connected to CI, and `alpha` repairs a red build the same day on each of its nine,
/// through the singleton settle.
fn firefighter_rig(committed: bool) -> (BadgesRig, ProjectId) {
    let mut rig = BadgesRig::new();
    let days: Vec<i64> = (0..9).map(|i| D1 + i).collect();
    let alpha = rig.project("alpha", &days, Flags::default());
    let beta_days: &[i64] = if committed {
        &[D1 - 100, TENTH]
    } else {
        &[D1 - 100]
    };
    let beta = rig.project("beta", beta_days, Flags::default());
    rig.settle_j4(alpha);
    rig.settle_j4(beta);
    let mut guard = rig.index.lock().unwrap();
    connect_ci(&guard, alpha);
    connect_ci(&guard, beta);
    for day in days {
        red_then_green(
            &mut guard,
            alpha,
            day * DAY_S + 9 * HOUR_S,
            day * DAY_S + 15 * HOUR_S,
        );
    }
    drop(guard);
    (rig, beta)
}

/// `beta` repairs a red build on [`TENTH`], through the singleton settle; the pairs after it.
fn tenth_repair(rig: &BadgesRig, beta: ProjectId) -> usize {
    let mut guard = rig.index.lock().unwrap();
    red_then_green(
        &mut guard,
        beta,
        TENTH * DAY_S + 9 * HOUR_S,
        TENTH * DAY_S + 15 * HOUR_S,
    );
    let pairs = repair_pairs(&guard).len();
    drop(guard);
    pairs
}

/// The Firefighter row names `beta` and [`TENTH`], day-grain, naming no project.
fn assert_tenth(award: Option<(String, Option<i64>, serde_json::Value)>) {
    let (earned_on, project, evidence) = award.expect("no firefighter row");
    assert_eq!(earned_on, local_date(TENTH));
    assert_eq!(project, None);
    assert_eq!(evidence, serde_json::json!({ "projectName": "beta" }));
}

/// §40.3, §40.5: Firefighter through the singleton payout's caller and the J4 settle. Ten
/// same-day repairs, each a `ci_red` item opened and closed on one local date through
/// `settle_singletons`. Where all ten dates already hold a commit-day, the tenth repair's
/// settle writes the row; where the tenth's commit-day is not walked yet, nothing is written
/// until a later J4 settle brings it, and the row appears at that settle.
#[test]
fn ac_p4_40_1_firefighter_through_the_payout_and_the_j4_settle() {
    committed_before_the_tenth_repair();
    walked_after_the_tenth_repair();
}

/// Every date already holds its commit-day: the tenth repair's singleton settle writes the row.
fn committed_before_the_tenth_repair() {
    let (rig, beta) = firefighter_rig(true);
    assert_eq!(
        rig.award("firefighter"),
        None,
        "nine pairs awarded firefighter"
    );
    let pairs = tenth_repair(&rig, beta);
    let award = rig.award("firefighter");
    eprintln!("committed first: {pairs} pairs after the tenth repair, award {award:?}");
    assert_tenth(award);
}

/// The tenth date's commit-day is not walked yet: nothing until the J4 settle that brings it.
fn walked_after_the_tenth_repair() {
    let (rig, beta) = firefighter_rig(false);
    let repaired = tenth_repair(&rig, beta);
    let before = rig.award("firefighter");
    eprintln!("not yet walked: {repaired} pairs after the tenth repair, award {before:?}");
    assert_eq!(
        before, None,
        "a repair with no commit-day that date counted"
    );
    rig.commit_file(beta, "fix.txt", b"fixed\n", TENTH);
    rig.settle_j4(beta);
    let pairs = repair_pairs(&rig.index.lock().unwrap()).len();
    let award = rig.award("firefighter");
    eprintln!("after the J4 settle: {pairs} pairs, award {award:?}");
    assert_tenth(award);
}

/// The project holding `FIREFIGHTER_PAIRS` qualifying pairs, its last on [`TENTH`].
fn plant_ten(index: &mut Index) {
    let days: Vec<i64> = (0..10).map(|i| D1 + i).collect();
    plant_repairs(index, "beta", &days);
    let pairs = repair_pairs(index);
    eprintln!("planted {} qualifying pairs: {pairs:?}", pairs.len());
    assert_eq!(pairs.len(), codotheca_core::badges::FIREFIGHTER_PAIRS);
}

/// `left@version` answered by an advisory sweep at `at`: vulnerable below 2.0.0 with a fix when
/// `vulnerable`, clean otherwise.
fn answer_left(index: &Index, version: &str, at: i64, vulnerable: bool) {
    let conn = index.conn();
    conn.execute(
        "INSERT INTO advisory_sweep (started_at, settled_at, outcome, complete)
         VALUES (?1, ?1, 'done', 1)",
        [at],
    )
    .unwrap();
    let sweep = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO advisory_triple
           (ecosystem, package_name, version, sweep_id, observed_at, answered)
         VALUES ('npm', 'left', ?1, ?2, ?3, 1)",
        rusqlite::params![version, sweep, at],
    )
    .unwrap();
    if vulnerable {
        conn.execute(
            "INSERT INTO advisory (advisory_id, severity, summary, url, observed_at)
             VALUES ('GHSA-aaaa', 'critical', 's', 'u', ?1)",
            [at],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO advisory_match
               (ecosystem, package_name, version, advisory_id, fix_available, fixed_version)
             VALUES ('npm', 'left', ?1, 'GHSA-aaaa', 1, '2.0.0')",
            [version],
        )
        .unwrap();
    }
}

/// The advisory settle at `at`, in a transaction of its own, under UTC.
fn settle_advisories(index: &Mutex<Index>, at: i64) {
    let zone = FixedZone(0);
    let mut guard = index.lock().unwrap();
    guard
        .with_tx(|tx| {
            codotheca_core::advisories::items::settle_advisory_items(
                tx,
                codotheca_core::clock::LocalZone::at(&zone, at),
                &zone,
            )
            .unwrap();
            Ok(())
        })
        .unwrap();
    drop(guard);
}

/// `kind`'s `(earned_on, awarded_at)` on `index`.
fn earned(index: &Mutex<Index>, kind: &str) -> Option<(String, i64)> {
    let guard = index.lock().unwrap();
    let row = guard
        .conn()
        .query_row(
            "SELECT earned_on, awarded_at FROM badge_award WHERE kind = ?1",
            [kind],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .ok();
    drop(guard);
    row
}

/// §40.5: Firefighter through the advisory payout's caller. Over a library already holding
/// `FIREFIGHTER_PAIRS` qualifying pairs, a scored advisory closure paid by the advisory settle
/// writes the row in that settle's transaction.
#[test]
fn ac_p4_40_1_firefighter_through_the_advisory_payout() {
    let dir = tempfile::tempdir().unwrap();
    let index = Mutex::new(Index::open_at(dir.path(), NOW).unwrap());
    let work = tempfile::tempdir().unwrap();
    let project = support::ledger_rig::advisory_project(&index, work.path(), NOW, true);
    plant_ten(&mut index.lock().unwrap());

    support::ledger_rig::lock_left(&index, project, work.path(), "1.0.0", NOW);
    answer_left(&index.lock().unwrap(), "1.0.0", NOW, true);
    settle_advisories(&index, NOW);
    let opened = earned(&index, "firefighter");
    eprintln!("after the settle that opened the advisory: {opened:?}");
    assert_eq!(
        opened, None,
        "a settle that paid nothing evaluated firefighter"
    );

    let later = NOW + DAY_S;
    support::ledger_rig::lock_left(&index, project, work.path(), "2.0.0", later);
    answer_left(&index.lock().unwrap(), "2.0.0", later, false);
    settle_advisories(&index, later);
    let award = earned(&index, "firefighter");
    eprintln!("after the settle that paid the upgrade: {award:?}");
    assert_eq!(award, Some((local_date(TENTH), later)));
}

/// §40.5: Firefighter through J7's payout. Over a library already holding `FIREFIGHTER_PAIRS`
/// qualifying pairs, a marker closure paid in J7's chunk transaction writes the row there.
#[test]
fn ac_p4_40_1_firefighter_through_the_j7_payout() {
    use codotheca_core::git::{JobClass, JobContext};
    use codotheca_core::jobs::j7_markers;

    let fixture = support::ledger_rig::j7_fixture(NOW);
    let index = Mutex::new(fixture.index);
    plant_ten(&mut index.lock().unwrap());
    let git = FakeGitBackend::new();
    let repo = RepoHandle::bare(
        Path::new("/does/not/matter"),
        StoreKey::new("test-store"),
        StoreClass::Local,
    );
    let run = |now: i64| {
        let cancel = CancelToken::new();
        let ctx = JobContext::new(JobClass::Background, &cancel, None);
        let zone = FixedZone(0);
        j7_markers::run_j7(
            &index,
            &git,
            &repo,
            &ctx,
            j7_markers::ScanRun {
                project: fixture.project,
                location: fixture.location,
                cursor: None,
                at: codotheca_core::clock::LocalZone::at(&zone, now),
                zone: &zone,
                detected_in: codotheca_core::protocol::HealthDetectedIn::Background,
                announce: None,
            },
        )
        .unwrap();
    };
    support::ledger_rig::marker_tree(&git, &index, fixture.location, "head-one", 5);
    run(NOW);
    let opened = earned(&index, "firefighter");
    eprintln!("after the run that opened five markers: {opened:?}");
    assert_eq!(
        opened, None,
        "a run that paid nothing evaluated firefighter"
    );

    support::ledger_rig::marker_tree(&git, &index, fixture.location, "head-two", 4);
    run(NOW + 60);
    let award = earned(&index, "firefighter");
    eprintln!("after the run that closed one: {award:?}");
    assert_eq!(award, Some((local_date(TENTH), NOW + 60)));
}
