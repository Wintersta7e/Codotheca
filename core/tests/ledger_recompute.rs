//! §38.7.2: git rows are deleted on a caller's behalf by one function, `recompute_git_rows`,
//! which ratchets the floor before it deletes anything; the floor has one writer; and the retained
//! half of the ledger is never deleted, by a merge or by an identity confirmation.
//!
//! `AC-P4-38-5` reads the source; `AC-P4-38-3` and `AC-P4-38-12` drive the production paths.
//! The Reference caller is §44's and its clauses are tested there.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use codotheca_core::firstrun::{dispatch, FirstRunEnv};
use codotheca_core::identity::merge::merge_projects;
use codotheca_core::identity::AssociationKind;
use codotheca_core::index::Index;
use codotheca_core::jobs::j15_authorship::{persist, AuthorshipFacts};
use codotheca_core::ledger::curve::level_of;
use codotheca_core::ledger::xp::xp_by_term;
use codotheca_core::ledger::RETAINED_KINDS;
use codotheca_core::protocol::ProjectId;
use rusqlite::Connection;
use serde_json::{json, Value};

const T0: i64 = 1_700_000_000;
const DAY: i64 = 86_400;
const A: &str = "a@example.invalid";
const B: &str = "b@example.invalid";
const C: &str = "c@example.invalid";

// ---------------------------------------------------------------------------------------------
// AC-P4-38-5 — one floor writer, one deleter.
// ---------------------------------------------------------------------------------------------

/// The statements that may appear only under `core/src/ledger/`.
const LEDGER_ONLY: [&str; 4] = [
    "DELETE FROM xp_events",
    "INSERT INTO level_floor",
    "UPDATE level_floor",
    "DELETE FROM level_floor",
];

/// Every `.rs` file under `core/src`, as `(path under core/src, text)`.
fn sources() -> Vec<(String, String)> {
    fn walk(dir: &Path, root: &Path, out: &mut Vec<(String, String)>) {
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            // A directory that vanished between the walk and the read is skipped before it is
            // counted.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
            Err(error) => panic!("{}: {error}", dir.display()),
        };
        for entry in entries {
            let path = entry.expect("a readable entry").path();
            if path.is_dir() {
                walk(&path, root, out);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                match std::fs::read_to_string(&path) {
                    Ok(text) => out.push((
                        path.strip_prefix(root)
                            .unwrap_or(&path)
                            .display()
                            .to_string()
                            .replace('\\', "/"),
                        text,
                    )),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => panic!("{}: {error}", path.display()),
                }
            }
        }
    }
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut out = Vec::new();
    walk(&root, &root, &mut out);
    out.sort();
    out
}

/// Whether the text after a `#[cfg(test)]` gates a `mod`: past whitespace, further attributes
/// and a `pub` or `pub(crate)`, the next word is `mod`.
fn gates_mod(after: &str) -> bool {
    let mut rest = after;
    loop {
        rest = rest.trim_start();
        let Some(attribute) = rest.strip_prefix("#[") else {
            break;
        };
        let mut depth = 1_usize;
        let mut end = None;
        for (i, c) in attribute.char_indices() {
            if c == '[' {
                depth += 1;
            } else if c == ']' {
                depth -= 1;
                if depth == 0 {
                    end = Some(i + 1);
                    break;
                }
            }
        }
        let Some(end) = end else {
            return false;
        };
        rest = attribute.get(end..).unwrap_or("");
    }
    let rest = rest
        .strip_prefix("pub(crate) ")
        .or_else(|| rest.strip_prefix("pub "))
        .unwrap_or(rest);
    rest.starts_with("mod ")
}

/// A file's production code: everything before its first `#[cfg(test)]` that gates a `mod`.
fn production(text: &str) -> &str {
    const GATE: &str = "#[cfg(test)]";
    let mut from = 0;
    while let Some(at) = text.get(from..).and_then(|rest| rest.find(GATE)) {
        let start = from + at;
        let after = start + GATE.len();
        if gates_mod(text.get(after..).unwrap_or("")) {
            return text.get(..start).unwrap_or(text);
        }
        from = after;
    }
    text
}

/// One space wherever the source breaks a line, continues a string or indents.
fn flattened(code: &str) -> String {
    code.replace("\\\n", " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Every `(path, statement)` naming a floor write or an `xp_events` delete, and every string
/// literal that names both `app_meta` and the legacy floor key.
fn hits(sources: &[(String, String)]) -> Vec<(String, String)> {
    let mut hits = Vec::new();
    for (path, text) in sources {
        let flat = flattened(production(text));
        for statement in LEDGER_ONLY {
            if flat.contains(statement) {
                hits.push((path.clone(), statement.to_owned()));
            }
        }
        // The text between quotes: a statement's literal, or one piece of a `format!`.
        if flat
            .split('"')
            .skip(1)
            .step_by(2)
            .any(|literal| literal.contains("app_meta") && literal.contains("level_floor"))
        {
            hits.push((path.clone(), "app_meta … level_floor".to_owned()));
        }
    }
    hits
}

#[test]
fn ac_p4_38_5_one_floor_writer_one_deleter() {
    let sources = sources();
    let found = hits(&sources);
    eprintln!(
        "files scanned: {}, hits: {} {found:?}",
        sources.len(),
        found.len()
    );
    assert!(!sources.is_empty(), "no source file was scanned");

    // The scan sees the two statements that must exist, so a silent scan cannot pass.
    for expected in [
        ("ledger/floor.rs", "UPDATE level_floor"),
        ("ledger/recompute.rs", "DELETE FROM xp_events"),
    ] {
        assert!(
            found
                .iter()
                .any(|(path, statement)| (path.as_str(), statement.as_str()) == expected),
            "the scan missed {expected:?}"
        );
    }
    let outside: Vec<&(String, String)> = found
        .iter()
        .filter(|(path, _)| !path.starts_with("ledger/"))
        .collect();
    assert_eq!(
        outside,
        Vec::<&(String, String)>::new(),
        "a floor write or an xp_events delete outside the ledger module"
    );

    let stamp: Vec<&str> = sources
        .iter()
        .filter(|(_, text)| text.contains("stamp_level_floor"))
        .map(|(path, _)| path.as_str())
        .collect();
    assert_eq!(stamp, Vec::<&str>::new(), "stamp_level_floor still exists");
}

#[test]
fn the_scan_cuts_test_modules_and_nothing_else() {
    let file = "fn f() { let _ = \"UPDATE level_floor SET level = 2\"; }\n\
                #[cfg(test)]\nfn helper() {}\n\
                fn g() { let _ = \"DELETE FROM \\\n    xp_events WHERE id = 1\"; }\n\
                #[cfg(test)]\n#[allow(clippy::unwrap_used)]\nmod tests {\n\
                fn h() { let _ = \"DELETE FROM level_floor\"; }\n}\n";
    let found = hits(&[("identity/x.rs".to_owned(), file.to_owned())]);
    let statements: Vec<&str> = found.iter().map(|(_, s)| s.as_str()).collect();
    assert_eq!(
        statements,
        ["DELETE FROM xp_events", "UPDATE level_floor"],
        "a statement split over a line continuation is read; a gated item is not a test module"
    );
}

// ---------------------------------------------------------------------------------------------
// Fixtures shared by the behavioural clauses.
// ---------------------------------------------------------------------------------------------

fn project(conn: &Connection, name: &str, created_at: i64) -> i64 {
    conn.execute(
        "INSERT INTO project (name, seed_basename, lineage_key, created_at, updated_at)
         VALUES (?1, ?1, 'L', ?2, ?2)",
        rusqlite::params![name, created_at],
    )
    .unwrap();
    conn.last_insert_rowid()
}

/// One copy of `project`, with its own `presence`.
fn location(conn: &Connection, project: i64, presence: &str) {
    let path = format!("/copy-of-{project}-{presence}");
    conn.execute(
        "INSERT INTO location (project_id, kind, path_bytes, path_key, path_display, store_key,
                               presence, repo_kind)
         VALUES (?1, 'linux', ?2, ?2, ?3, 'store', ?4, 'worktree')",
        rusqlite::params![project, path.as_bytes(), path, presence],
    )
    .unwrap();
}

/// Every history job of `project` finished.
fn concluded(conn: &Connection, project: i64) {
    for job in ["j1", "j1_5", "j4"] {
        conn.execute(
            "INSERT INTO project_job_state (project_id, job, state, at) VALUES (?1, ?2, 'ok', ?3)",
            rusqlite::params![project, job, T0],
        )
        .unwrap();
    }
}

/// `project`'s `j4` row: `(state, reason)`.
fn j4(conn: &Connection, project: i64) -> (String, Option<String>) {
    conn.query_row(
        "SELECT state, reason FROM project_job_state WHERE project_id = ?1 AND job = 'j4'",
        [project],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .unwrap()
}

/// `n` commit-days on consecutive days ending at `last_day`.
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

/// One row of every retained kind on `project`: `(id, kind)`.
fn retained(conn: &Connection, project: i64) -> Vec<(i64, &'static str)> {
    RETAINED_KINDS
        .iter()
        .map(|&kind| {
            conn.execute(
                "INSERT INTO xp_events (ts, tz_offset_min, project_id, subject_key, kind,
                                        dedupe_key, track)
                 VALUES (?1, 0, ?2, 's', ?3, ?4, 'session')",
                rusqlite::params![T0, project, kind, format!("{kind}:{project}")],
            )
            .unwrap();
            (conn.last_insert_rowid(), kind)
        })
        .collect()
}

fn git_rows(conn: &Connection, project: i64) -> i64 {
    conn.query_row(
        "SELECT count(*) FROM xp_events WHERE project_id = ?1 AND track = 'git'",
        [project],
        |r| r.get(0),
    )
    .unwrap()
}

/// The project a retained row now belongs to, or `None` when the row is gone or owns none.
fn owner(conn: &Connection, id: i64) -> Option<i64> {
    use rusqlite::OptionalExtension as _;
    conn.query_row(
        "SELECT project_id FROM xp_events WHERE id = ?1",
        [id],
        |r| r.get::<_, Option<i64>>(0),
    )
    .optional()
    .unwrap()
    .flatten()
}

fn identity(conn: &Connection, email: &str) {
    conn.execute(
        "INSERT INTO identity (email, is_user, source) VALUES (?1, 1, 'gitconfig')",
        [email],
    )
    .unwrap();
}

/// The census J1.5 would write, through its own writer.
fn committers(conn: &mut Connection, project: i64, email: &str) {
    let tx = conn.transaction().unwrap();
    persist(
        &tx,
        ProjectId(project),
        &AuthorshipFacts {
            committers: vec![(email.to_owned(), 3)],
            authored_by_user: Some(true),
        },
    )
    .unwrap();
    tx.commit().unwrap();
}

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

/// `identity.confirm {apply: true}` through the production dispatcher.
fn confirm(conn: &mut Connection, emails: &[&str], now: i64) -> Value {
    let home = tempfile::tempdir().unwrap();
    dispatch(
        conn,
        &env(home.path()),
        "identity.confirm",
        &json!({ "emails": emails, "apply": true }),
        now,
    )
    .expect("identity.confirm is first run's")
    .expect("identity.confirm answered")
}

/// §1.5's merge, whole, in one transaction.
fn merge(conn: &mut Connection, a: i64, b: i64, now: i64) -> (i64, i64) {
    let tx = conn.transaction().unwrap();
    let out = merge_projects(
        &tx,
        a,
        b,
        AssociationKind::Manual,
        &json!({ "rule": "manual" }),
        now,
    )
    .unwrap();
    tx.commit().unwrap();
    (out.survivor, out.absorbed)
}

fn floor_row(conn: &Connection) -> (i64, Option<i64>) {
    conn.query_row(
        "SELECT level, raised_at FROM level_floor WHERE id = 1",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .unwrap()
}

// ---------------------------------------------------------------------------------------------
// AC-P4-38-3 — retained rows are never deleted.
// ---------------------------------------------------------------------------------------------

#[test]
fn ac_p4_38_3_retained_rows_are_never_deleted() {
    // The schema refuses the delete outright.
    let dir = tempfile::tempdir().unwrap();
    let mut index = Index::open_at(dir.path(), T0).unwrap();
    let p = project(index.conn(), "p", 100);
    let rows = retained(index.conn(), p);
    for (id, kind) in &rows {
        let deleted = index
            .conn()
            .execute("DELETE FROM xp_events WHERE id = ?1", [id]);
        assert!(deleted.is_err(), "a {kind} row was deleted");
    }
    eprintln!("retained kinds refused a delete: {}", rows.len());
    assert_eq!(rows.len(), RETAINED_KINDS.len());

    // A real merge moves every retained row to the survivor, by row id, and deletes both
    // sides' git rows.
    let older = project(index.conn(), "older", 100);
    let newer = project(index.conn(), "newer", 200);
    let mut moved = Vec::new();
    for side in [older, newer] {
        location(index.conn(), side, "present");
        concluded(index.conn(), side);
        commit_days(index.conn(), side, T0 / DAY - 30 * side, 5);
        moved.extend(retained(index.conn(), side));
    }
    let (survivor, absorbed) = merge(index.conn_mut(), newer, older, T0 + 10);
    assert_eq!((survivor, absorbed), (older, newer));
    for (id, kind) in &moved {
        assert_eq!(
            owner(index.conn(), *id),
            Some(survivor),
            "the {kind} row {id} did not move to the survivor"
        );
    }
    eprintln!("retained rows reparented by the merge: {}", moved.len());
    assert_eq!(git_rows(index.conn(), survivor), 0);
    assert_eq!(git_rows(index.conn(), absorbed), 0);

    // An identity confirmation that moves a project to Reference leaves every retained row.
    let conn = index.conn_mut();
    identity(conn, A);
    identity(conn, B);
    let mine = project(conn, "mine", 300);
    let theirs = project(conn, "theirs", 400);
    let mut kept = Vec::new();
    for (side, email) in [(mine, A), (theirs, B)] {
        location(conn, side, "present");
        concluded(conn, side);
        committers(conn, side, email);
        commit_days(conn, side, T0 / DAY - 100 - 30 * side, 5);
        kept.extend(retained(conn, side));
    }
    confirm(conn, &[A], T0 + 20);
    assert_eq!(
        git_rows(conn, theirs),
        0,
        "the confirmation deleted nothing"
    );
    for (id, kind) in &kept {
        assert!(
            owner(conn, *id).is_some(),
            "the confirmation deleted the {kind} row {id}"
        );
    }
    eprintln!("retained rows kept by the confirmation: {}", kept.len());
}

// ---------------------------------------------------------------------------------------------
// AC-P4-38-12 — the one recompute path, and its ratchet.
// ---------------------------------------------------------------------------------------------

static TRACE: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn record(sql: &str) {
    TRACE
        .lock()
        .unwrap()
        .push(sql.split_whitespace().collect::<Vec<_>>().join(" "));
}

/// The statements `f` issues on `conn`, whitespace-flattened, in order.
fn traced(conn: &mut Connection, f: impl FnOnce(&mut Connection)) -> Vec<String> {
    TRACE.lock().unwrap().clear();
    conn.trace(Some(record));
    f(conn);
    conn.trace(None);
    std::mem::take(&mut *TRACE.lock().unwrap())
}

/// Whether `statement` is `recompute_git_rows`' delete: one project's git track, nothing else.
fn is_the_one_delete(statement: &str) -> bool {
    statement
        .strip_prefix("DELETE FROM xp_events WHERE project_id = ")
        .and_then(|rest| rest.strip_suffix(" AND track = 'git'"))
        .is_some_and(|project| !project.contains(' '))
}

/// Every `xp_events` delete in `statements` is the one path's, and the position of the first.
fn deletes_through_the_one_path(statements: &[String]) -> usize {
    let deletes: Vec<&String> = statements
        .iter()
        .filter(|s| s.contains("DELETE FROM xp_events"))
        .collect();
    eprintln!("xp_events deletes traced: {deletes:?}");
    assert!(!deletes.is_empty(), "the trace holds no delete");
    assert_eq!(
        deletes
            .iter()
            .filter(|s| !is_the_one_delete(s))
            .collect::<Vec<_>>(),
        Vec::<&&String>::new(),
        "a git row was deleted outside recompute_git_rows"
    );
    statements
        .iter()
        .position(|s| s.contains("DELETE FROM xp_events"))
        .unwrap()
}

fn first_ratchet(statements: &[String]) -> Option<usize> {
    statements
        .iter()
        .position(|s| s.starts_with("UPDATE level_floor"))
}

#[test]
fn ac_p4_38_12_the_one_recompute_path() {
    let dir = tempfile::tempdir().unwrap();
    let mut index = Index::open_at(dir.path(), T0).unwrap();
    let conn = index.conn_mut();
    for email in [A, B, C] {
        identity(conn, email);
    }
    let by_a = project(conn, "by-a", 100);
    let by_b = project(conn, "by-b", 200);
    let by_b_away = project(conn, "by-b-away", 300);
    let by_c = project(conn, "by-c", 400);
    for (p, email, presence) in [
        (by_a, A, "present"),
        (by_b, B, "present"),
        (by_b_away, B, "offline"),
        (by_c, C, "present"),
    ] {
        location(conn, p, presence);
        concluded(conn, p);
        committers(conn, p, email);
        commit_days(conn, p, T0 / DAY - 30 * p, 20);
    }

    // The first answer: the card is unanswered when the path runs, so nothing is ratcheted.
    let first = traced(conn, |c| {
        confirm(c, &[A, B], T0 + 10);
    });
    deletes_through_the_one_path(&first);
    assert_eq!(
        first_ratchet(&first),
        None,
        "an unanswered card was ratcheted"
    );
    assert_eq!(floor_row(conn), (1, None));
    assert_eq!(git_rows(conn, by_c), 0);

    // After it, a narrowing ratchets from the pre-change reading before its first delete.
    let before = level_of(xp_by_term(conn).unwrap().total());
    assert!(before > 1, "the fixture's level must be worth protecting");
    let away_rows = git_rows(conn, by_b_away);
    let second = traced(conn, |c| {
        confirm(c, &[A], T0 + 20);
    });
    let delete = deletes_through_the_one_path(&second);
    let ratchet = first_ratchet(&second).expect("the answered, computed path ratchets");
    eprintln!("ratchet at statement {ratchet}, first delete at {delete}");
    assert!(ratchet < delete, "the floor was ratcheted after a delete");
    assert_eq!(floor_row(conn), (i64::from(before), Some(T0 + 20)));

    // The readable project lost its git rows and its finished history walks again; the one with
    // no readable copy keeps both.
    assert_eq!(git_rows(conn, by_b), 0);
    assert_eq!(
        j4(conn, by_b),
        ("queued".to_owned(), Some("user_requested".to_owned()))
    );
    assert_eq!(
        git_rows(conn, by_b_away),
        away_rows,
        "a frozen history lost rows"
    );
    assert_eq!(j4(conn, by_b_away), ("ok".to_owned(), None));

    // A merge takes the same path, ratchet first.
    let older = project(conn, "older", 500);
    let newer = project(conn, "newer", 600);
    for p in [older, newer] {
        location(conn, p, "present");
        concluded(conn, p);
        committers(conn, p, A);
        commit_days(conn, p, T0 / DAY - 30 * p, 40);
    }
    let raised = level_of(xp_by_term(conn).unwrap().total());
    assert!(
        raised > before,
        "the merge fixture must earn a level above the floor"
    );
    let merged = traced(conn, |c| {
        merge(c, newer, older, T0 + 30);
    });
    let merge_delete = deletes_through_the_one_path(&merged);
    let merge_ratchet = first_ratchet(&merged).expect("a merge ratchets");
    assert!(
        merge_ratchet < merge_delete,
        "a merge ratcheted after a delete"
    );
    assert_eq!(floor_row(conn), (i64::from(raised), Some(T0 + 30)));
    assert_eq!(git_rows(conn, older), 0);
    assert_eq!(
        j4(conn, older),
        ("queued".to_owned(), Some("user_requested".to_owned()))
    );
}
