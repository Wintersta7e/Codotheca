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

use codotheca_core::badges::lifeline::LIFELINE_RUN_KEY;
use codotheca_core::badges::{
    COMEBACK_GAP_DAYS, CURATOR_PROJECTS, MARATHON_CREDITED_SECONDS, UNDERTAKER_PROJECTS,
};
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
    session_index, session_row, AwardRow, BadgesRig, Flags, SessionLib, T0,
};
use support::lifeline_rig::{
    date as lifeline_date, real_handler, tick_and_settle, LifelineRig, D0 as LIFELINE_D0,
};

const SCHEMA: &str = include_str!("../../protocol/schema/protocol.json");
const NOW: i64 = 1_760_000_000;

/// A local day number in early 2024: the first commit-day of every gap below.
const D1: i64 = 19_724;

/// What one drive left behind over one fixture.
#[derive(Debug, Clone)]
struct TwinOutcome {
    /// Every `badge_award` row before the drive.
    before: Vec<Vec<Value>>,
    /// Every `badge_award` row after it.
    awards: Vec<Vec<Value>>,
    /// Every `badge_award.evidence` after it, as stored.
    evidence: Vec<String>,
    /// `SELECT COUNT(*) FROM xp_events` after it.
    xp_events: i64,
    /// Every event the runners emitted, as `topic/event`.
    events: Vec<String>,
}

/// The same drive over two fresh fixtures, identical but for a pre-latched row of `kind` in the
/// second.
#[derive(Debug, Clone)]
struct Twin {
    fresh: TwinOutcome,
    latched: TwinOutcome,
}

/// The latched twin's one extra row: `kind` held before the drive, with empty evidence.
fn latch(conn: &Connection, kind: &str) {
    conn.execute(
        "INSERT INTO badge_award (kind, earned_at, tz_offset_min, earned_on, evidence,
                                  awarded_at, awarded_on)
         VALUES (?1, 0, 0, '1970-01-01', '{}', 0, '1970-01-01')",
        [kind],
    )
    .unwrap();
}

/// Every event `sink` recorded, as `topic/event`.
fn event_names(sink: &ValidatingSink) -> Vec<String> {
    sink.events
        .lock()
        .unwrap()
        .iter()
        .map(|(topic, event, _)| format!("{topic}/{event}"))
        .collect()
}

/// What a drive left on `conn`, given the award rows before it and the events it emitted.
fn observed(conn: &Connection, before: Vec<Vec<Value>>, events: Vec<String>) -> TwinOutcome {
    let evidence = conn
        .prepare("SELECT evidence FROM badge_award ORDER BY kind")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    TwinOutcome {
        before,
        awards: awards(conn),
        evidence,
        xp_events: conn
            .query_row("SELECT COUNT(*) FROM xp_events", [], |r| r.get(0))
            .unwrap(),
        events,
    }
}

fn run_once<S>(
    kind: Option<&str>,
    fixture: &impl Fn() -> (BadgesRig, S),
    drive: &impl Fn(&BadgesRig, &S),
) -> TwinOutcome {
    let (rig, state) = fixture();
    if let Some(kind) = kind {
        latch(rig.index.lock().unwrap().conn(), kind);
    }
    let before = rig.awards();
    drive(&rig, &state);
    let guard = rig.index.lock().unwrap();
    let outcome = observed(guard.conn(), before, event_names(&rig.events));
    drop(guard);
    outcome
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

/// A twin over a fixture no [`BadgesRig`] holds: `drive` builds it, latches the kind it is
/// handed, and reports what it left behind.
fn twin_by(kind: &str, drive: impl Fn(Option<&str>) -> TwinOutcome) -> Twin {
    Twin {
        fresh: drive(None),
        latched: drive(Some(kind)),
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

/// §40.10: `badges.list` answers the awarded rows and nothing else — three awards come back as
/// exactly three, and no other kind is named anywhere in the payload. Its basis is the reveal's.
#[test]
fn ac_p4_40_6_the_list_returns_awarded_rows_only() {
    let lib = codotheca_core::testing::TempIndex::new();
    let index = lib.index();
    let held = ["comeback", "marathon", "necromancer"];
    for (i, kind) in (0_i64..).zip(held) {
        let earned_at = NOW - i * 86_400;
        index
            .conn()
            .execute(
                "INSERT INTO badge_award (kind, earned_at, tz_offset_min, earned_on, evidence,
                                          awarded_at, awarded_on)
                 VALUES (?1, ?2, 0, ?3, '{}', ?4, ?3)",
                rusqlite::params![kind, earned_at, local_date(earned_at / 86_400), NOW],
            )
            .unwrap();
    }
    let zone = FixedZone(0);
    let ctx = codotheca_core::badges::commands::BadgesCtx {
        index,
        now: NOW,
        zone: &zone,
    };
    let payload = codotheca_core::badges::commands::dispatch_badges_command(
        &ctx,
        "badges.list",
        serde_json::json!({}),
    )
    .expect("owned")
    .expect("listed");
    eprintln!("{payload}");
    let awards = payload["awards"].as_array().unwrap();
    let kinds: BTreeSet<&str> = awards.iter().map(|a| a["kind"].as_str().unwrap()).collect();
    assert_eq!(awards.len(), 3);
    assert_eq!(kinds, held.into_iter().collect());
    let text = payload.to_string();
    let mut absent = 0;
    for kind in codotheca_core::protocol::BadgeKind::ALL {
        let name = codotheca_core::badges::kind_text(kind);
        if !held.contains(&name.as_str()) {
            absent += 1;
            assert!(!text.contains(&name), "the list names unearned {name}");
        }
    }
    eprintln!("{absent} unearned kinds absent from the payload");
    assert_eq!(absent, 5);
    let pop = codotheca_core::stats::reveal::population(index.conn()).unwrap();
    assert_eq!(
        payload["basis"],
        serde_json::json!({
            "projectsCovered": pop.history_covered,
            "projectsTotal": pop.total,
            "historyComplete": pop.history_complete,
        })
    );
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
        .position(|name| name == "project_id")
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
    let (session, before) = marathon_session(&mut lib, Arc::new(ValidatingSink::default()));
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

/// A session on `lib` launched through the manager, saved into every tick for
/// `MARATHON_CREDITED_SECONDS`, then stopped, its events on `events`: the session, and the
/// marathon row while it was still open.
fn marathon_session(
    lib: &mut SessionLib,
    events: Arc<ValidatingSink>,
) -> (SessionId, Option<AwardRow>) {
    let clock = Arc::new(FakeClock::new(T_SESSION));
    let activity = SharedActivity::default();
    let mut manager = SessionManager::new(
        clock.clone(),
        events,
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
    (session, before)
}

/// §40.3, §40.5: Marathon through the second close path — the startup lane's orphan recovery.
/// A session a crash left open with `MARATHON_CREDITED_SECONDS` of credit is closed at its
/// watermark, never at now, and that close writes the row.
#[test]
fn ac_p4_40_1_marathon_through_close_orphans() {
    let mut lib = session_index();
    let session = orphaned_session(&mut lib, T_SESSION, MARATHON_CREDITED_SECONDS);
    let closed = startup_later(&mut lib, &Arc::new(ValidatingSink::default()));
    let later = T_SESSION + ORPHAN_LATER;
    let (ended_at, credited) = session_row(&lib.index, session);
    let award = award_row(lib.index.index().conn(), "marathon");
    eprintln!("{closed} closed at {ended_at:?} credited {credited}: {award:?}");
    assert_eq!(closed, 1);
    let award = award.expect("the orphan close wrote no marathon");
    assert_eq!(award.earned_at, T_SESSION + MARATHON_CREDITED_SECONDS);
    assert_eq!(Some(award.earned_at), ended_at);
    assert_eq!(award.earned_on, "2023-11-14");
    assert_eq!(award.awarded_at, later);
    assert_eq!(award.project_id, Some(lib.project.0));
    assert_eq!(award.evidence["creditedSeconds"], MARATHON_CREDITED_SECONDS);
}

/// How long after the orphaned session's start the next process starts.
const ORPHAN_LATER: i64 = 3 * 86_400;

/// The startup lane of a process starting [`ORPHAN_LATER`] after `T_SESSION` over `lib`, its
/// events on `events`: how many orphaned sessions it closed.
fn startup_later(lib: &mut SessionLib, events: &Arc<ValidatingSink>) -> u64 {
    let later = T_SESSION + ORPHAN_LATER;
    let mut sessions = SessionManager::new(
        Arc::new(FakeClock::new(later)),
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
    report.sessions_closed
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
    rule_ci_na(&rig, tenth);
    let counted = curator_counted(&rig).expect("the ruling wrote no curator");
    tenth_listed(&rig, tenth, &counted);
}

/// `projects.setCheckNa` rules the tenth project's CI config not applicable, through the detail
/// dispatcher.
fn rule_ci_na(rig: &BadgesRig, tenth: ProjectId) {
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
    sync_to_gold(&rig, tenth);
    let counted = curator_counted(&rig).expect("the sync settle wrote no curator");
    tenth_listed(&rig, tenth, &counted);
}

/// A sync runner over the rig, the tenth project's remote enqueued, run until its settle has
/// lifted the tenth to gold — the projection write the award shares a transaction with.
fn sync_to_gold(rig: &BadgesRig, tenth: ProjectId) {
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
    let at_gold = || {
        let (lit, evaluable) = rig.projection(tenth);
        lit.is_some() && lit == evaluable
    };
    while Instant::now() < deadline && !at_gold() {
        std::thread::sleep(Duration::from_millis(25));
    }
    runner.request_stop();
    runner.join();
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

/// One kind's `(earned_on, awarded_at)`, when it is held.
type Earned = Option<(String, i64)>;

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
    let (opened, award) = open_then_pay_advisory(&index, project, work.path());
    eprintln!("after the settle that opened the advisory: {opened:?}");
    assert_eq!(
        opened, None,
        "a settle that paid nothing evaluated firefighter"
    );
    eprintln!("after the settle that paid the upgrade: {award:?}");
    assert_eq!(award, Some((local_date(TENTH), NOW + DAY_S)));
}

/// `left@1.0.0` locked, answered vulnerable and settled at `NOW`; then `2.0.0`, answered clean
/// and settled a day later, which pays the closure. The firefighter row after each settle.
fn open_then_pay_advisory(
    index: &Mutex<Index>,
    project: ProjectId,
    work: &Path,
) -> (Earned, Earned) {
    support::ledger_rig::lock_left(index, project, work, "1.0.0", NOW);
    answer_left(&index.lock().unwrap(), "1.0.0", NOW, true);
    settle_advisories(index, NOW);
    let opened = earned(index, "firefighter");
    let later = NOW + DAY_S;
    support::ledger_rig::lock_left(index, project, work, "2.0.0", later);
    answer_left(&index.lock().unwrap(), "2.0.0", later, false);
    settle_advisories(index, later);
    (opened, earned(index, "firefighter"))
}

/// §40.5: Firefighter through J7's payout. Over a library already holding `FIREFIGHTER_PAIRS`
/// qualifying pairs, a marker closure paid in J7's chunk transaction writes the row there.
#[test]
fn ac_p4_40_1_firefighter_through_the_j7_payout() {
    let fixture = support::ledger_rig::j7_fixture(NOW);
    let index = Mutex::new(fixture.index);
    plant_ten(&mut index.lock().unwrap());
    let (opened, award) = j7_open_then_close(&index, fixture.project, fixture.location);
    eprintln!("after the run that opened five markers: {opened:?}");
    assert_eq!(
        opened, None,
        "a run that paid nothing evaluated firefighter"
    );
    eprintln!("after the run that closed one: {award:?}");
    assert_eq!(award, Some((local_date(TENTH), NOW + 60)));
}

/// J7 over five markers at `NOW`, then over four a minute later, which closes one and pays it.
/// The firefighter row after each run.
fn j7_open_then_close(
    index: &Mutex<Index>,
    project: ProjectId,
    location: codotheca_core::protocol::LocationId,
) -> (Earned, Earned) {
    use codotheca_core::git::{JobClass, JobContext};
    use codotheca_core::jobs::j7_markers;

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
            index,
            &git,
            &repo,
            &ctx,
            j7_markers::ScanRun {
                project,
                location,
                cursor: None,
                at: codotheca_core::clock::LocalZone::at(&zone, now),
                zone: &zone,
                detected_in: codotheca_core::protocol::HealthDetectedIn::Background,
                announce: None,
            },
        )
        .unwrap();
    };
    support::ledger_rig::marker_tree(&git, index, location, "head-one", 5);
    run(NOW);
    let opened = earned(index, "firefighter");
    support::ledger_rig::marker_tree(&git, index, location, "head-two", 4);
    run(NOW + 60);
    (opened, earned(index, "firefighter"))
}

/// `undertaker`'s row on the rig's index, when it is held.
fn undertaker_row(rig: &BadgesRig) -> Option<AwardRow> {
    let guard = rig.index.lock().unwrap();
    let row = award_row(guard.conn(), "undertaker");
    drop(guard);
    row
}

/// Undertaker through §38's `clean_archive` writer, at that writer's caller in the settle: five
/// projects archived through `projects.setFlags`, each read by a J1 and then a settle that pays its
/// row. The fifth settle runs at 01:00 UTC, the evening before five hours west, so the award is
/// dated by the fifth row's own instant and offset, not UTC's date and not the first row's.
#[test]
fn ac_p4_40_1_undertaker_through_the_clean_archive_writer() {
    let mut rig = BadgesRig::new();
    let names = ["first", "second", "third", "fourth", "fifth"];
    assert_eq!(names.len(), UNDERTAKER_PROJECTS);
    // T0 is 08:00 UTC; the five settles run from 21:00 to 01:00 UTC.
    let mut at = T0 + 13 * HOUR_S;
    let mut last = Vec::new();
    for (i, name) in names.iter().enumerate() {
        let project = rig.archivable(name);
        rig.set_archived(project, true);
        rig.observe_archive(project, at, WEST);
        last = rig.clean_archive_rows(project);
        let held = undertaker_row(&rig);
        eprintln!("{name}: clean_archive {last:?}, undertaker {held:?}");
        assert_eq!(last.len(), 1, "{name}'s settle paid no clean_archive");
        if i + 1 < UNDERTAKER_PROJECTS {
            assert_eq!(held, None, "{} projects awarded undertaker", i + 1);
        }
        at += HOUR_S;
    }
    let (ts, offset) = last[0];
    let row = undertaker_row(&rig).expect("the fifth settle wrote no undertaker");
    assert_eq!(row.earned_at, ts, "earned at the fifth project's row");
    assert_eq!(row.tz_offset_min, i64::from(WEST));
    assert_eq!(offset, i64::from(WEST));
    assert_eq!(
        row.earned_on, "2027-01-15",
        "the row's local date, not UTC's"
    );
    assert_eq!(row.awarded_at, ts, "awarded at the paying settle");
    assert_eq!((row.project_id, row.subject_key), (None, None));
    let listed: Vec<(&str, bool)> = row.evidence["projects"]
        .as_array()
        .expect("undertaker evidence lists its projects")
        .iter()
        .map(|p| (p["name"].as_str().unwrap(), p["evaluable"].is_null()))
        .collect();
    assert_eq!(listed, names.map(|n| (n, true)));
}

/// §40.5's day close is Lifeline's production path: thirty clear days through the core's tick,
/// one project read by the daily pass each day, award the row at the close of the thirtieth.
#[test]
fn ac_p4_40_1_lifeline_through_the_tick() {
    let mut rig = LifelineRig::new();
    rig.project("daily");
    rig.tick();
    for _ in 0..codotheca_core::badges::LIFELINE_DAYS {
        rig.after_days(1);
    }
    let guard = rig.index.lock().unwrap();
    let award = award_row(guard.conn(), "lifeline");
    drop(guard);
    eprintln!(
        "after {}: {:?} {award:?}",
        lifeline_date(rig.today()),
        rig.run()
    );
    let award = award.expect("the tick awarded no lifeline");
    let last = LIFELINE_D0 + codotheca_core::badges::LIFELINE_DAYS - 1;
    assert_eq!(award.earned_on, lifeline_date(last));
    assert_eq!(
        award.evidence,
        serde_json::json!({ "runStartOn": lifeline_date(LIFELINE_D0) })
    );
}

/// `project`, given the lineage the restored award's subject names.
fn with_lineage(conn: &Connection, project: i64) -> i64 {
    conn.execute(
        "UPDATE project SET lineage_key = 'awarded-lineage' WHERE id = ?1",
        [project],
    )
    .unwrap();
    project
}

/// §40.7, an award restored with no project gets it at the first day close after the scan
/// rediscovers its subject, through the resolver that never guesses — every column then equals
/// the pre-rebuild row — and the run state came back through the settings export.
#[test]
fn ac_p4_40_19_the_day_close_resolves_a_restored_project_by_subject() {
    let dir = tempfile::tempdir().unwrap();
    let index = Index::open_at(dir.path(), NOW).unwrap();
    let conn = index.conn();
    let project = with_lineage(conn, plant_project(conn, "awarded"));
    conn.execute(
        "INSERT INTO badge_award (kind, earned_at, tz_offset_min, earned_on, project_id,
                                  subject_key, evidence, awarded_at, awarded_on)
         VALUES ('necromancer', 86400, 0, '1970-01-02', ?1, ?2,
                 '{\"projectName\":\"awarded\",\"gapDays\":200}', 90000, '1970-01-02')",
        rusqlite::params![project, "lineage:awarded-lineage|remote:"],
    )
    .unwrap();
    let today = local_date(NOW.div_euclid(86_400));
    let run = format!(r#"{{"open":"{today}","runStart":"2025-10-01","runEnd":"2025-10-08"}}"#);
    index.set_app_meta(LIFELINE_RUN_KEY, &run).unwrap();
    let before = awards(conn);
    let at = project_column(conn);
    index.export_sidecar(NOW + 10).unwrap();
    drop(index);

    let rebuilt = corrupt_and_rebuild(dir.path(), NOW + 20);
    let restored_run = rebuilt.app_meta(LIFELINE_RUN_KEY).unwrap();
    eprintln!("run restored: {restored_run:?}");
    assert_eq!(restored_run.as_deref(), Some(run.as_str()));
    assert_eq!(
        awards(rebuilt.conn())[0][at],
        Value::Null,
        "waits for its subject"
    );

    // The scan rediscovers the subject.
    let found = with_lineage(rebuilt.conn(), plant_project(rebuilt.conn(), "awarded"));
    assert_eq!(found, project, "the fixture's ids line up");
    let shared = Arc::new(Mutex::new(rebuilt));
    let clock = Arc::new(FakeClock::new(NOW + 86_400));
    let (mut handler, pump, _) = real_handler(dir.path(), &shared, &clock);
    tick_and_settle(&mut handler, &pump);
    let guard = shared.lock().unwrap();
    let after = awards(guard.conn());
    drop(guard);
    codotheca_core::proto::dispatch::CommandHandler::shutdown(&mut handler);
    eprintln!("after the day close: {after:?}");
    assert_eq!(after, before, "every column equals the pre-rebuild row");
}

/// `rig`'s index under the lock, with every `badge_award` row and the `project_id` column.
fn awards_and_column(rig: &BadgesRig) -> (Vec<Vec<Value>>, usize) {
    let guard = rig.index.lock().unwrap();
    let out = (awards(guard.conn()), project_column(guard.conn()));
    drop(guard);
    out
}

/// §40.7: a merge reparents an award and recomputes nothing. A Necromancer award on project B,
/// then `projects.merge {a, b}` through the identity dispatcher absorbing B: every row that
/// named B names the survivor, and every other column is byte-identical.
#[test]
fn ac_p4_40_20_a_merge_repoints_the_award_and_nothing_else() {
    let mut rig = BadgesRig::new();
    let keeper = rig.project("keeper", &[D1], Flags::default());
    let revived = rig.project("revived", &[D1, D1 + REVIVAL_GAP_DAYS], Flags::default());
    rig.settle_j4(keeper);
    rig.settle_j4(revived);
    let (before, at) = awards_and_column(&rig);
    eprintln!("before the merge: {before:?}");
    let on_b = before
        .iter()
        .filter(|row| row[at] == Value::Integer(revived.0))
        .count();
    assert!(on_b > 0, "the fixture holds no award on the absorbed side");
    assert!(
        !of_kind(&before, "necromancer").is_empty(),
        "no necromancer"
    );

    let guard = rig.index.lock().unwrap();
    let ctx = codotheca_core::identity::commands::IdentityCtx {
        index: &guard,
        events: rig.events.as_ref(),
        now: T0,
    };
    let merged = codotheca_core::identity::commands::dispatch_identity_command(
        &ctx,
        "projects.merge",
        serde_json::json!({ "a": keeper.0, "b": revived.0 }),
    )
    .expect("projects.merge is identity's")
    .expect("the merge commits");
    drop(guard);
    eprintln!("merge: {merged}");
    assert_eq!(merged["survivor"], keeper.0);
    assert_eq!(merged["absorbed"], revived.0);

    let (after, _) = awards_and_column(&rig);
    eprintln!("after the merge: {after:?}");
    let expected: Vec<Vec<Value>> = before
        .iter()
        .map(|row| {
            let mut row = row.clone();
            if row[at] == Value::Integer(revived.0) {
                row[at] = Value::Integer(keeper.0);
            }
            row
        })
        .collect();
    assert_eq!(after, expected, "a column other than project_id moved");
}

/// §40.7: an award is never recomputed. Necromancer through a J4 settle; then the address
/// behind it is unticked through `identity.confirm {apply: true}` on a present copy, so the
/// `commit_day` rows go — and the award row stays byte-identical.
#[test]
fn ac_p4_40_3_a_narrowed_identity_set_leaves_the_award() {
    let mut rig = BadgesRig::new();
    let revived = rig.project("revived", &[D1, D1 + REVIVAL_GAP_DAYS], Flags::default());
    rig.settle_j4(revived);
    let commit_days = |of: &BadgesRig| -> i64 {
        let guard = of.index.lock().unwrap();
        let n = guard
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM xp_events WHERE kind = 'commit_day' AND project_id = ?1",
                [revived.0],
                |r| r.get(0),
            )
            .unwrap();
        drop(guard);
        n
    };
    let (before, _) = awards_and_column(&rig);
    let days_before = commit_days(&rig);
    eprintln!("before: {days_before} commit_day rows, awards {before:?}");
    assert!(days_before > 0, "the settle wrote no commit_day");
    assert!(
        !of_kind(&before, "necromancer").is_empty(),
        "no necromancer"
    );

    let home = tempfile::tempdir().unwrap();
    let env = codotheca_core::firstrun::FirstRunEnv {
        sources: codotheca_core::firstrun::sources::SourceEnv {
            home: home.path().to_path_buf(),
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
    };
    let mut guard = rig.index.lock().unwrap();
    let applied = codotheca_core::firstrun::dispatch(
        guard.conn_mut(),
        &env,
        "identity.confirm",
        &serde_json::json!({ "emails": [], "apply": true }),
        T0,
    )
    .expect("identity.confirm is first run's")
    .expect("identity.confirm answered");
    drop(guard);
    eprintln!("identity.confirm: {applied}");

    let days_after = commit_days(&rig);
    let (after, _) = awards_and_column(&rig);
    eprintln!("after: {days_after} commit_day rows, awards {after:?}");
    assert_eq!(
        days_after, 0,
        "the narrowing kept the rows behind the award"
    );
    assert_eq!(after, before, "the narrowing touched an award");
}
/// One production caller's twin pair, and the kind it writes.
#[derive(Debug)]
struct CallerTwin {
    caller: &'static str,
    kind: &'static str,
    twin: Twin,
}

/// Five projects of **H**, each with its language computed, on five 2024 commit-days.
fn five_languages() -> (BadgesRig, Vec<ProjectId>) {
    let mut rig = BadgesRig::new();
    let mut projects = Vec::new();
    for (name, day, language) in [
        ("go", D1 + 1, "Go"),
        ("python", D1 + 2, "Python"),
        ("ruby", D1 + 3, "Ruby"),
        ("rust", D1 + 5, "Rust"),
        ("c", D1 + 10, "C"),
    ] {
        let project = rig.project(name, &[day], Flags::default());
        set_language(&rig, project, language);
        projects.push(project);
    }
    (rig, projects)
}

/// Four languages computed and settled; the fifth project's waits for its J3.
fn fifth_language_pending() -> (BadgesRig, ProjectId) {
    let (rig, projects) = five_languages();
    let rust = projects[3];
    rig.sql(&format!(
        "UPDATE project SET primary_language = NULL WHERE id = {}",
        rust.0
    ));
    rig.commit_file(rust, "src/main.rs", b"fn main() {}\n", D1 + 5);
    for &project in &projects {
        rig.settle_j4(project);
    }
    (rig, rust)
}

/// Five projects archived through `projects.setFlags`, not yet read by a J1.
fn five_archived() -> (BadgesRig, Vec<ProjectId>) {
    let mut rig = BadgesRig::new();
    let projects: Vec<ProjectId> = ["first", "second", "third", "fourth", "fifth"]
        .iter()
        .map(|name| rig.archivable(name))
        .collect();
    for &project in &projects {
        rig.set_archived(project, true);
    }
    (rig, projects)
}

/// Firefighter's J4 path: the tenth repair already settled on a date whose commit is not walked.
fn tenth_repair_unwalked() -> (BadgesRig, ProjectId) {
    let (rig, beta) = firefighter_rig(false);
    tenth_repair(&rig, beta);
    rig.commit_file(beta, "fix.txt", b"fixed\n", TENTH);
    (rig, beta)
}

/// `conn`, latched with `latched` when given, and its award rows before the drive.
fn latched_before(conn: &Connection, latched: Option<&str>) -> Vec<Vec<Value>> {
    if let Some(kind) = latched {
        latch(conn, kind);
    }
    awards(conn)
}

/// The job settle's twins: after J4 for every kind it evaluates, and after J3.
fn job_settle_twins() -> Vec<CallerTwin> {
    let j4 = |rig: &BadgesRig, project: &ProjectId| rig.settle_j4(*project);
    vec![
        CallerTwin {
            caller: "the job settle after J4",
            kind: "necromancer",
            twin: necromancer_twin().clone(),
        },
        CallerTwin {
            caller: "the job settle after J4",
            kind: "comeback",
            twin: twin(
                "comeback",
                || {
                    let mut rig = BadgesRig::new();
                    let p =
                        rig.project("returned", &[D1, D1 + COMEBACK_GAP_DAYS], Flags::default());
                    (rig, p)
                },
                j4,
            ),
        },
        CallerTwin {
            caller: "the job settle after J4",
            kind: "polyglot",
            twin: twin(
                "polyglot",
                five_languages,
                |rig, projects: &Vec<ProjectId>| {
                    for &project in projects {
                        rig.settle_j4(project);
                    }
                },
            ),
        },
        CallerTwin {
            caller: "the job settle after J3",
            kind: "polyglot",
            twin: twin(
                "polyglot",
                fifth_language_pending,
                |rig, rust: &ProjectId| {
                    rig.settle_j3(*rust);
                },
            ),
        },
        CallerTwin {
            caller: "the job settle after J4",
            kind: "firefighter",
            twin: twin("firefighter", tenth_repair_unwalked, j4),
        },
    ]
}

/// The completion projection's three writers, the singleton payout and the `clean_archive` writer.
fn rig_writer_twins() -> Vec<CallerTwin> {
    vec![
        CallerTwin {
            caller: "the job settle after J2",
            kind: "curator",
            twin: twin(
                "curator",
                || {
                    let mut rig = BadgesRig::new();
                    let tenth = nine_at_gold_and(&mut rig, "present");
                    (rig, tenth)
                },
                |rig, tenth: &ProjectId| rig.settle_j2(*tenth),
            ),
        },
        CallerTwin {
            caller: "projects.setCheckNa",
            kind: "curator",
            twin: twin(
                "curator",
                || {
                    let mut rig = BadgesRig::new();
                    let tenth = nine_at_gold_and(&mut rig, "absent");
                    rig.settle_j2(tenth);
                    (rig, tenth)
                },
                |rig, tenth: &ProjectId| rule_ci_na(rig, *tenth),
            ),
        },
        CallerTwin {
            caller: "the sync completion hook",
            kind: "curator",
            twin: twin(
                "curator",
                || {
                    let mut rig = BadgesRig::new();
                    let tenth = nine_at_gold_and(&mut rig, "absent");
                    rig.settle_j2(tenth);
                    rig.completion_facts(tenth, "present");
                    (rig, tenth)
                },
                |rig, tenth: &ProjectId| sync_to_gold(rig, *tenth),
            ),
        },
        CallerTwin {
            caller: "the singleton payout",
            kind: "firefighter",
            twin: twin(
                "firefighter",
                || firefighter_rig(true),
                |rig, beta: &ProjectId| {
                    tenth_repair(rig, *beta);
                },
            ),
        },
        CallerTwin {
            caller: "the clean_archive writer",
            kind: "undertaker",
            twin: twin(
                "undertaker",
                five_archived,
                |rig, projects: &Vec<ProjectId>| {
                    let mut at = T0 + 13 * HOUR_S;
                    for &project in projects {
                        rig.observe_archive(project, at, WEST);
                        at += HOUR_S;
                    }
                },
            ),
        },
    ]
}

/// The two session closes, the two payouts that take no event sink, and the day close.
fn other_twins() -> Vec<CallerTwin> {
    vec![
        CallerTwin {
            caller: "close_session",
            kind: "marathon",
            twin: twin_by("marathon", |latched| {
                let mut lib = session_index();
                let before = latched_before(lib.index.index().conn(), latched);
                let events = Arc::new(ValidatingSink::default());
                marathon_session(&mut lib, events.clone());
                observed(lib.index.index().conn(), before, event_names(&events))
            }),
        },
        CallerTwin {
            caller: "close_orphans",
            kind: "marathon",
            twin: twin_by("marathon", |latched| {
                let mut lib = session_index();
                orphaned_session(&mut lib, T_SESSION, MARATHON_CREDITED_SECONDS);
                let before = latched_before(lib.index.index().conn(), latched);
                let events = Arc::new(ValidatingSink::default());
                startup_later(&mut lib, &events);
                observed(lib.index.index().conn(), before, event_names(&events))
            }),
        },
        // The advisory settle and J7 take no event sink: there is nothing for them to emit to.
        CallerTwin {
            caller: "the advisory payout",
            kind: "firefighter",
            twin: twin_by("firefighter", |latched| {
                let dir = tempfile::tempdir().unwrap();
                let index = Mutex::new(Index::open_at(dir.path(), NOW).unwrap());
                let work = tempfile::tempdir().unwrap();
                let project = support::ledger_rig::advisory_project(&index, work.path(), NOW, true);
                plant_ten(&mut index.lock().unwrap());
                let before = latched_before(index.lock().unwrap().conn(), latched);
                open_then_pay_advisory(&index, project, work.path());
                let guard = index.lock().unwrap();
                observed(guard.conn(), before, Vec::new())
            }),
        },
        CallerTwin {
            caller: "J7's payout",
            kind: "firefighter",
            twin: twin_by("firefighter", |latched| {
                let fixture = support::ledger_rig::j7_fixture(NOW);
                let index = Mutex::new(fixture.index);
                plant_ten(&mut index.lock().unwrap());
                let before = latched_before(index.lock().unwrap().conn(), latched);
                j7_open_then_close(&index, fixture.project, fixture.location);
                let guard = index.lock().unwrap();
                observed(guard.conn(), before, Vec::new())
            }),
        },
        // The handler's own publisher is detached in this rig; the job pump's sink is read.
        CallerTwin {
            caller: "the day close",
            kind: "lifeline",
            twin: twin_by("lifeline", |latched| {
                let mut rig = LifelineRig::new();
                rig.project("daily");
                let before = latched_before(rig.index.lock().unwrap().conn(), latched);
                rig.tick();
                for _ in 0..codotheca_core::badges::LIFELINE_DAYS {
                    rig.after_days(1);
                }
                let guard = rig.index.lock().unwrap();
                observed(guard.conn(), before, event_names(&rig.jobs_events))
            }),
        },
    ]
}

/// The twin pair of every production caller §40.5 names but arming, which lands with §43's
/// acknowledgement, each driven the way its AC-P4-40-1 test drives it. Built once, checked as it is built: the fresh twin writes
/// the kind, the latched twin leaves the held row as it was.
fn outcomes() -> &'static [CallerTwin] {
    static OUTCOMES: OnceLock<Vec<CallerTwin>> = OnceLock::new();
    OUTCOMES.get_or_init(|| {
        let mut out = job_settle_twins();
        out.extend(rig_writer_twins());
        out.extend(other_twins());
        for c in &out {
            let fresh = &c.twin.fresh;
            let latched = &c.twin.latched;
            eprintln!(
                "{} ({}): fresh {} rows, {} xp_events, {} events; latched {} rows, {} xp_events, {} events",
                c.caller,
                c.kind,
                fresh.awards.len(),
                fresh.xp_events,
                fresh.events.len(),
                latched.awards.len(),
                latched.xp_events,
                latched.events.len()
            );
            assert!(of_kind(&fresh.before, c.kind).is_empty(), "{}: held before", c.caller);
            assert_eq!(of_kind(&fresh.awards, c.kind).len(), 1, "{} wrote no {}", c.caller, c.kind);
            assert_eq!(
                of_kind(&latched.awards, c.kind),
                of_kind(&latched.before, c.kind),
                "{}: a held {} was written again",
                c.caller,
                c.kind
            );
        }
        out
    })
}

/// The eight kinds' wire slugs, read from the schema's `BadgeKind`.
fn badge_kinds() -> Vec<String> {
    let doc: serde_json::Value = serde_json::from_str(SCHEMA).unwrap();
    let kinds: Vec<String> = doc["types"]["BadgeKind"]["variants"]
        .as_array()
        .expect("BadgeKind declares its variants")
        .iter()
        .map(|v| v.as_str().unwrap().to_owned())
        .collect();
    assert_eq!(
        kinds.len(),
        8,
        "BadgeKind is not the eight kinds: {kinds:?}"
    );
    kinds
}

/// Every `.rs` file under `core/src`, comments stripped.
fn core_sources() -> Vec<(String, String)> {
    let files = support::source_scan::rust_sources("src");
    eprintln!("{} files under core/src", files.len());
    assert!(!files.is_empty(), "the scan read no file");
    files
}

/// The files outside `core/src/badges/` that may name the award table: the sidecar section and
/// the merge's re-point (§40.7).
const AWARD_NAMERS: &[&str] = &["index/sidecar.rs", "identity/merge.rs"];

/// The testkit fixture registry keys its badge fixture by the sidecar section's name, and that
/// fixture writes through `badges::store`. Compiled only with `testkit`, and pinned to this one
/// line so the file can never grow a statement of its own.
const FIXTURE_KEY: (&str, &str) = ("testing/sidecar.rs", "(\"badge_award\", awarded_badges),");

/// §40.7, §40.14: the latch in code. No statement under `core/src` deletes an award or a project,
/// and the table is named only where §40.7 lets it be.
#[test]
fn ac_p4_40_3_the_latch_in_code() {
    let mut hits = Vec::new();
    let mut namers = BTreeSet::new();
    let files = core_sources();
    for (name, source) in &files {
        let sql = support::source_scan::sql_normal(source);
        if sql.contains("DELETE FROM BADGE_AWARD") {
            hits.push(format!("{name}: DELETE FROM badge_award"));
        }
        for (at, _) in sql.match_indices("DELETE FROM PROJECT") {
            let next = sql.as_bytes().get(at + "DELETE FROM PROJECT".len());
            if !next.is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_') {
                hits.push(format!("{name}: DELETE FROM project"));
            }
        }
        let naming: Vec<&str> = source
            .lines()
            .filter(|l| l.contains("badge_award"))
            .collect();
        if naming.is_empty() {
            continue;
        }
        namers.insert(name.clone());
        let allowed = name.starts_with("badges/")
            || AWARD_NAMERS.contains(&name.as_str())
            || (name == FIXTURE_KEY.0 && naming.iter().all(|l| l.trim() == FIXTURE_KEY.1));
        if !allowed {
            hits.push(format!("{name} names badge_award: {naming:?}"));
        }
    }
    eprintln!("files naming badge_award: {namers:?}");
    eprintln!("hits: {hits:?}");
    assert!(
        namers.iter().any(|n| n.starts_with("badges/")),
        "no file under badges/ names its own table: the scan read the wrong tree"
    );
    assert!(hits.is_empty(), "{hits:?}");
}

/// §40.8: a landing has one writer. Every `UPDATE badge_award` under `core/src` is read up to the
/// end of its string literal; only `mark_landed`'s sets `landed_at`.
#[test]
fn ac_p4_40_3_only_mark_landed_updates_a_landing() {
    let mut updates = Vec::new();
    for (name, source) in &core_sources() {
        let sql = support::source_scan::sql_normal(source);
        for (at, _) in sql.match_indices("UPDATE BADGE_AWARD") {
            let statement = sql
                .get(at..)
                .unwrap_or_default()
                .split('"')
                .next()
                .unwrap_or_default()
                .to_owned();
            updates.push((name.clone(), statement));
        }
    }
    eprintln!("UPDATE badge_award statements: {updates:?}");
    let landing: Vec<&str> = updates
        .iter()
        .filter(|(_, statement)| statement.contains("LANDED_AT"))
        .map(|(name, _)| name.as_str())
        .collect();
    assert_eq!(
        landing,
        ["badges/landing.rs"],
        "the landing has another writer"
    );
}

/// §40.9: no award writes the ledger. Every caller's twin pair ends with equal `xp_events`
/// counts; `xp_events`' declaration names no badge kind; and the file declaring §38's dormancy
/// constant, the level module, never names the award table.
#[test]
fn ac_p4_40_4_no_award_writes_the_ledger() {
    let twins = outcomes();
    for c in twins {
        eprintln!(
            "{} ({}): xp_events {} fresh, {} latched",
            c.caller, c.kind, c.twin.fresh.xp_events, c.twin.latched.xp_events
        );
        assert_eq!(
            c.twin.fresh.xp_events, c.twin.latched.xp_events,
            "{}: an award wrote the ledger",
            c.caller
        );
    }
    eprintln!("{} twin pairs compared", twins.len());

    let dir = tempfile::tempdir().unwrap();
    let index = Index::open_at(dir.path(), NOW).unwrap();
    let create: String = index
        .conn()
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'xp_events'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let kinds = badge_kinds();
    let named: Vec<&String> = kinds
        .iter()
        .filter(|k| create.to_lowercase().contains(k.as_str()))
        .collect();
    eprintln!(
        "xp_events: {} bytes of DDL, badge kinds named {named:?}",
        create.len()
    );
    assert!(create.contains("CHECK"), "xp_events declares no kind CHECK");
    assert!(named.is_empty(), "xp_events' CHECK names {named:?}");

    let files = core_sources();
    let declaring: Vec<&(String, String)> = files
        .iter()
        .filter(|(_, source)| source.contains("const REVIVAL_GAP_DAYS"))
        .collect();
    eprintln!(
        "the dormancy constant is declared in {:?}",
        declaring.iter().map(|(n, _)| n).collect::<Vec<_>>()
    );
    assert_eq!(declaring.len(), 1, "the constant is declared once");
    assert!(
        !declaring[0].1.contains("badge_award"),
        "{} reads the award table",
        declaring[0].0
    );
}

/// The schema's types reachable from `root` through struct fields, `root` included.
fn reachable(types: &serde_json::Value, root: &str) -> BTreeSet<String> {
    let mut seen = BTreeSet::new();
    let mut pending = vec![root.to_owned()];
    while let Some(name) = pending.pop() {
        if !seen.insert(name.clone()) {
            continue;
        }
        if let Some(fields) = types[name.as_str()]["fields"].as_object() {
            for field in fields.values() {
                let inner: String = field
                    .as_str()
                    .unwrap_or_default()
                    .chars()
                    .filter(|c| !matches!(c, '[' | ']' | '?'))
                    .collect();
                pending.push(inner);
            }
        }
    }
    seen
}

/// §40.8: an award is never announced outside its landing. Every caller's twin pair emits the
/// same events once `motivation/changed` is set aside — compared as multisets, since a job pump's
/// settles may interleave; no file under `core/src/badges/` emits; and the schema declares no
/// `badges` topic and no event but `motivation/changed` whose payload reaches a `Badge*` type.
#[test]
fn ac_p4_40_5_an_award_emits_no_event_but_motivation_changed() {
    let mut compared = 0;
    for c in outcomes() {
        let without = |events: &[String]| {
            let mut kept: Vec<String> = events
                .iter()
                .filter(|e| *e != "motivation/changed")
                .cloned()
                .collect();
            kept.sort();
            kept
        };
        let fresh = without(&c.twin.fresh.events);
        let latched = without(&c.twin.latched.events);
        eprintln!(
            "{} ({}): {} events {fresh:?}",
            c.caller,
            c.kind,
            fresh.len()
        );
        assert_eq!(fresh, latched, "{}: an award emitted an event", c.caller);
        compared += fresh.len();
    }
    eprintln!("{compared} events compared across the twin pairs");
    assert!(compared > 0, "no caller emitted anything to compare");

    let badges = support::source_scan::rust_sources("src/badges");
    let emitting: Vec<&String> = badges
        .iter()
        .filter(|(_, source)| source.contains(".emit("))
        .map(|(name, _)| name)
        .collect();
    eprintln!(
        "{} files under core/src/badges/, emitting {emitting:?}",
        badges.len()
    );
    assert!(!badges.is_empty(), "the scan read no file");
    assert!(emitting.is_empty(), "{emitting:?} emit");

    let doc: serde_json::Value = serde_json::from_str(SCHEMA).unwrap();
    let topics = doc["topics"]
        .as_object()
        .expect("the schema declares topics");
    assert!(
        !topics.contains_key("badges"),
        "the schema declares a badges topic"
    );
    let mut scanned = 0;
    for (topic, events) in topics {
        for (event, payload) in events.as_object().unwrap() {
            scanned += 1;
            let badge: Vec<String> = reachable(&doc["types"], payload.as_str().unwrap())
                .into_iter()
                .filter(|t| t.starts_with("Badge"))
                .collect();
            // §43.15: `motivation/snapshot` is not a second announcement. It answers a
            // subscription or an overflow with `motivation.get`'s own state, the payload
            // `motivation/changed` carries, and no award fires it.
            if !matches!(
                format!("{topic}/{event}").as_str(),
                "motivation/changed" | "motivation/snapshot"
            ) {
                assert!(badge.is_empty(), "{topic}/{event} carries {badge:?}");
            }
        }
    }
    eprintln!("{scanned} events across {} topics scanned", topics.len());
    assert!(scanned > 0, "the schema scan read no event");
}

/// §40.10: evidence is a closed set. Every evidence the twin fixtures stored decodes through the
/// one decoder, and the production reader refuses a stored row carrying an undeclared key.
#[test]
fn ac_p4_40_17_every_stored_evidence_decodes_strictly() {
    let mut decoded = 0;
    for c in outcomes() {
        for raw in c.twin.fresh.evidence.iter().chain(&c.twin.latched.evidence) {
            codotheca_core::badges::evidence::decode(raw)
                .unwrap_or_else(|e| panic!("{}: {raw} does not decode: {e:?}", c.caller));
            decoded += 1;
        }
    }
    eprintln!("{decoded} stored evidence values decoded");
    assert!(decoded > 0, "the fixtures stored no evidence");

    let dir = tempfile::tempdir().unwrap();
    let index = Index::open_at(dir.path(), NOW).unwrap();
    index
        .conn()
        .execute(
            "INSERT INTO badge_award (kind, earned_at, tz_offset_min, earned_on, evidence,
                                      awarded_at, awarded_on)
             VALUES ('comeback', 0, 0, '1970-01-01', '{\"gapDays\":30,\"lines\":12}', 0,
                     '1970-01-01')",
            [],
        )
        .unwrap();
    let read = codotheca_core::badges::landing::list_awards(index.conn());
    eprintln!("a row with an undeclared key: {read:?}");
    assert!(
        matches!(read, Err(codotheca_core::badges::BadgeError::Evidence(_))),
        "the reader accepted an undeclared evidence key"
    );
}

/// §40.5, §40.14: every kind has its own production arm. `predicate`'s dispatch in
/// `core/src/badges/evaluate.rs`, read from the source, maps each `BadgeKind` variant to the
/// predicate named for it — no placeholder answering nothing, no shared or wildcard arm.
#[test]
fn ac_p4_40_1_every_kind_has_a_production_arm() {
    let files = support::source_scan::rust_sources("src/badges");
    let evaluate = &files
        .iter()
        .find(|(name, _)| name == "badges/evaluate.rs")
        .expect("core/src/badges/evaluate.rs exists")
        .1;
    let body = evaluate
        .split("fn predicate(")
        .nth(1)
        .expect("evaluate.rs declares predicate")
        .split("match kind {")
        .nth(1)
        .expect("predicate matches on the kind");
    let arms: Vec<&str> = body
        .lines()
        .map(str::trim)
        .take_while(|l| *l != "}")
        .filter(|l| !l.is_empty())
        .collect();
    eprintln!("predicate's {} arms: {arms:?}", arms.len());
    let kinds = badge_kinds();
    assert_eq!(
        arms.len(),
        kinds.len(),
        "an arm is shared, missing or a wildcard"
    );
    for slug in &kinds {
        let variant: String = slug
            .chars()
            .enumerate()
            .map(|(i, c)| if i == 0 { c.to_ascii_uppercase() } else { c })
            .collect();
        let arm = format!("BadgeKind::{variant} => {slug}(tx, ctx),");
        assert!(arms.contains(&arm.as_str()), "no production arm `{arm}`");
        let defined = files
            .iter()
            .any(|(_, source)| source.contains(&format!("fn {slug}(")));
        assert!(defined, "the predicate {slug} is not defined under badges/");
    }
}
