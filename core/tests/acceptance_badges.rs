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
use std::sync::OnceLock;

use codotheca_core::badges::COMEBACK_GAP_DAYS;
use codotheca_core::clock::local_date;
use codotheca_core::index::rebuild::{rebuild_in_place, RebuildOutcome};
use codotheca_core::index::sidecar::{counts, read, RestoreCtx, RestoreOutcome, SECTIONS};
use codotheca_core::index::Index;
use codotheca_core::ledger::price::REVIVAL_GAP_DAYS;
use codotheca_core::protocol::ProjectId;
use rusqlite::types::Value;
use rusqlite::Connection;
use support::badges_rig::{BadgesRig, Flags};

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
