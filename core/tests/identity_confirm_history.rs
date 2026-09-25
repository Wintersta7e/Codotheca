//! §38.7 — **`identity.confirm` never destroys what it cannot give back**, driven through the
//! production dispatcher, `firstrun::dispatch`.
//!
//! `AC-P4-38-13`: a confirmation deletes a project's git-derived rows only when its history can be
//! re-read (§38.7.1) — some copy neither removed nor away.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::collections::BTreeSet;
use std::sync::Arc;

use codotheca_core::firstrun::{dispatch, FirstRunEnv};
use codotheca_core::index::Index;
use codotheca_core::jobs::j15_authorship::{persist, AuthorshipFacts};
use codotheca_core::protocol::ProjectId;
use rusqlite::Connection;
use serde_json::{json, Value};

const NOW: i64 = 1_790_000_000;
const A: &str = "a@example.invalid";
const OTHER: &str = "other@example.invalid";

fn env(home: &std::path::Path) -> FirstRunEnv {
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

/// `identity.confirm` through the production dispatcher.
fn confirm(index: &mut Index, home: &std::path::Path, emails: &[&str], apply: bool) -> Value {
    dispatch(
        index.conn_mut(),
        &env(home),
        "identity.confirm",
        &json!({ "emails": emails, "apply": apply }),
        NOW,
    )
    .expect("identity.confirm is first run's")
    .expect("identity.confirm answered")
}

fn identity(conn: &Connection, email: &str, is_user: bool) {
    conn.execute(
        "INSERT INTO identity (email, is_user, source) VALUES (?1, ?2, 'gitconfig')",
        rusqlite::params![email, i64::from(is_user)],
    )
    .unwrap();
}

fn project(conn: &Connection, name: &str) -> i64 {
    conn.execute(
        "INSERT INTO project (name, seed_basename, lineage_key, created_at, updated_at)
         VALUES (?1, ?1, ?1, ?2, ?2)",
        rusqlite::params![name, NOW],
    )
    .unwrap();
    conn.last_insert_rowid()
}

/// One copy of `project`. `removed` stamps `removed_at`; `presence` is the copy's own.
fn location(conn: &Connection, project: i64, presence: &str, removed: bool) {
    let n: i64 = conn
        .query_row("SELECT count(*) FROM location", [], |r| r.get(0))
        .unwrap();
    let path = format!("/copy-{n}");
    conn.execute(
        "INSERT INTO location (project_id, kind, path_bytes, path_key, path_display, store_key,
                               presence, repo_kind, removed_at)
         VALUES (?1, 'linux', ?2, ?2, ?3, 'store', ?4, 'worktree', ?5)",
        rusqlite::params![
            project,
            path.as_bytes(),
            path,
            presence,
            removed.then_some(NOW - 10)
        ],
    )
    .unwrap();
}

/// The census J1.5 would write, through its own writer.
fn authorship(conn: &mut Connection, project: i64, committers: &[&str], mine: Option<bool>) {
    let tx = conn.transaction().unwrap();
    persist(
        &tx,
        ProjectId(project),
        &AuthorshipFacts {
            committers: committers.iter().map(|e| ((*e).to_owned(), 3)).collect(),
            authored_by_user: mine,
        },
    )
    .unwrap();
    tx.commit().unwrap();
}

fn commit_days(conn: &Connection, project: i64, n: usize) {
    for i in 0..n {
        conn.execute(
            "INSERT INTO xp_events (ts, project_id, subject_key, kind, dedupe_key, track)
             VALUES (?1, ?2, 's', 'commit_day', ?3, 'git')",
            rusqlite::params![NOW, project, format!("commit_day:{project}:{i}")],
        )
        .unwrap();
    }
}

fn failed_j4(conn: &Connection, project: i64) {
    conn.execute(
        "INSERT INTO project_job_state (project_id, job, state, fail_count, reason, at)
         VALUES (?1, 'j4', 'failed', 2, 'timeout: walk', ?2)",
        rusqlite::params![project, NOW - 100],
    )
    .unwrap();
}

/// Every `xp_events` key of `project`, by identity.
fn keys(conn: &Connection, project: i64) -> BTreeSet<String> {
    let mut stmt = conn
        .prepare("SELECT dedupe_key FROM xp_events WHERE project_id = ?1")
        .unwrap();
    let keys = stmt
        .query_map([project], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    drop(stmt);
    keys
}

fn j4_state(conn: &Connection, project: i64) -> String {
    conn.query_row(
        "SELECT state FROM project_job_state WHERE project_id = ?1 AND job = 'j4'",
        [project],
        |r| r.get(0),
    )
    .unwrap()
}

/// **`AC-P4-38-13`.** Confirming `[a@]` moves four projects committed only by `other@` into
/// Reference. Their git rows are deleted and re-queued only where the history can be walked again:
/// an uninstalled copy and an offline one keep theirs, frozen; a present copy — alone or beside a
/// removed one — gives them up to the recompute. The preview's count is the join the write
/// performs, and a session-track row is never touched.
#[test]
fn ac_p4_38_13_a_confirm_keeps_the_git_rows_of_a_history_it_cannot_reread() {
    let dir = tempfile::tempdir().unwrap();
    let mut index = Index::open_at(dir.path(), NOW).unwrap();
    let (u, o, r, rr) = {
        let conn = index.conn_mut();
        identity(conn, A, true);
        identity(conn, OTHER, true);
        let u = project(conn, "uninstalled");
        let o = project(conn, "offline");
        let r = project(conn, "readable");
        let rr = project(conn, "removed-and-present");
        location(conn, u, "present", true);
        location(conn, o, "offline", false);
        location(conn, r, "present", false);
        location(conn, rr, "present", true);
        location(conn, rr, "present", false);
        for (p, days) in [(u, 3), (o, 2), (r, 2), (rr, 1)] {
            authorship(conn, p, &[OTHER], Some(true));
            commit_days(conn, p, days);
        }
        failed_j4(conn, u);
        failed_j4(conn, r);
        conn.execute(
            "INSERT INTO xp_events (ts, tz_offset_min, project_id, subject_key, kind, dedupe_key,
                                    track, meta)
             VALUES (?1, 0, ?2, 'lineage:uninstalled|remote:', 'debt_day',
                     'debt_day:lineage:uninstalled|remote::2026-09-21', 'session',
                     '{\"sources\":[\"todo_marker\"],\"closed\":1}')",
            rusqlite::params![NOW, u],
        )
        .unwrap();
        (u, o, r, rr)
    };
    let names = [("U", u), ("O", o), ("R", r), ("RR", rr)];
    let before: Vec<BTreeSet<String>> = names.iter().map(|(_, p)| keys(index.conn(), *p)).collect();

    let preview = confirm(&mut index, dir.path(), &[A], false);
    eprintln!("AC-P4-38-13 preview: {preview}");
    assert_eq!(preview["movedToReference"], 4);
    assert_eq!(
        preview["commitDaysRemoved"], 3,
        "the preview counted rows the write keeps"
    );

    let applied = confirm(&mut index, dir.path(), &[A], true);
    assert_eq!(applied["applied"], true);
    assert_eq!(applied["commitDaysRemoved"], 3);

    let conn = index.conn();
    for ((name, p), was) in names.iter().zip(&before) {
        let now = keys(conn, *p);
        eprintln!("AC-P4-38-13 {name}: {} rows → {}", was.len(), now.len());
    }
    let kept = |p: i64, was: &BTreeSet<String>| {
        assert_eq!(
            &keys(conn, p),
            was,
            "project {p} lost rows it cannot recompute"
        );
    };
    kept(u, &before[0]);
    kept(o, &before[1]);
    assert!(
        keys(conn, r).is_empty(),
        "the readable project kept its rows"
    );
    assert!(
        keys(conn, rr).is_empty(),
        "a present copy beside a removed one kept its rows"
    );
    assert_eq!(
        j4_state(conn, u),
        "failed",
        "an unreadable history was re-queued"
    );
    assert_eq!(
        j4_state(conn, r),
        "queued",
        "the readable history was not re-queued"
    );
    let reference: Vec<i64> = names
        .iter()
        .map(|(_, p)| {
            conn.query_row(
                "SELECT is_reference FROM project WHERE id = ?1",
                [p],
                |row| row.get(0),
            )
            .unwrap()
        })
        .collect();
    assert_eq!(
        reference,
        vec![1, 1, 1, 1],
        "every project moves into Reference"
    );
}
