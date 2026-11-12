//! §56.8: one SQL visibility fragment decides whether a project is on the shelf, in a count or in
//! a live population (AC-P4-56-23), and a fully excluded project leaves every one of them.
#![cfg(feature = "testkit")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use codotheca_core::art::testsupport::CollectingSink;
use codotheca_core::exclusion::visibility::is_fully_excluded;
use codotheca_core::index::Index;
use codotheca_core::projects::{dispatch_projects_command, ProjectsCtx};
use codotheca_core::protocol::ProjectId;
use codotheca_core::stats::reveal::population;
use rusqlite::{params, Connection};
use serde_json::{json, Value};

const NOW: i64 = 1_781_179_200;

/// Every `is_hidden` mention in `core/src` that is not a visibility test, keyed by the file and
/// a piece of the line's own text (never a line number), with the reason it is not one.
const CLASSIFIED: &[(&str, &str, &str)] = &[
    (
        "exclusion/visibility.rs",
        "",
        "the fragment's own definition and its docs",
    ),
    (
        "projects/flags.rs",
        "",
        "the flags writer and its read-back: the column, not a test",
    ),
    (
        "projects/rows.rs",
        "p.is_hidden, p.is_reference",
        "the wire row's flag column",
    ),
    (
        "projects/rows.rs",
        "is_hidden: r.get",
        "the wire row's flag column",
    ),
    (
        "projects/testrow.rs",
        "is_hidden: false",
        "a test row's default",
    ),
    (
        "query/execute.rs",
        "no is_hidden",
        "§8.0b's prose, stated once",
    ),
    (
        "query/execute.rs",
        "IsFlag::Hidden => TermTruth::known(r.is_hidden)",
        "`is:hidden`'s own term",
    ),
    (
        "query/execute.rs",
        "row.is_hidden && !asks_for(IsFlag::Hidden)",
        "§8.0b's per-flag opt-in over a loaded row, read beside `is_excluded`",
    ),
    (
        "identity/merge.rs",
        "",
        "the merge folds and snapshots the flag; it decides no visibility",
    ),
    (
        "identity/commands.rs",
        "\"is_hidden\" => Some(Flag::Hidden)",
        "a flag name's parser",
    ),
    (
        "identity/confirm.rs",
        "is_hidden = 0",
        "the identity join: `p4-56b` changes it (U37), not this fragment",
    ),
    (
        "identity/store.rs",
        "flag(&conn, orphan.project_id, \"is_hidden\")",
        "a test's read-back",
    ),
    (
        "index/pending.rs",
        "",
        "the pending restore writes the flag back",
    ),
    (
        "index/sidecar.rs",
        "",
        "the sidecar carries the flag column",
    ),
    (
        "testing/sidecar.rs",
        "UPDATE project SET is_hidden = 1",
        "a testkit fixture's write",
    ),
    (
        "surfaces/diag.rs",
        "is_hidden, is_reference, is_fork",
        "the diagnostics row dump",
    ),
    (
        "stats/reveal.rs",
        "is_reference, is_archived, is_hidden, created_at",
        "a test fixture's insert",
    ),
    (
        "stats/reveal.rs",
        "UPDATE project SET is_hidden = 1",
        "a test fixture's write",
    ),
    (
        "firstrun/roots.rs",
        "UPDATE project SET is_hidden = 1",
        "a test fixture's write",
    ),
    (
        "git/analyse.rs",
        "fn a_lowercase_tag_or_skip_worktree_is_hidden",
        "a test name about git output, not the column",
    ),
];

fn rust_sources(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_sources(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs")
            && path.file_name().is_some_and(|n| n != "protocol.rs")
        {
            out.push(path);
        }
    }
}

#[test]
fn ac_p4_56_23_every_is_hidden_read_in_core_is_the_fragment_or_classified() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rust_sources(&src, &mut files);
    let mut lines = 0usize;
    let mut classified = 0usize;
    let mut used = BTreeSet::new();
    let mut unclassified = Vec::new();
    for file in &files {
        let rel = file
            .strip_prefix(&src)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        let text = std::fs::read_to_string(file).unwrap();
        for (n, line) in text.lines().enumerate() {
            if !line.contains("is_hidden") {
                continue;
            }
            lines += 1;
            let hit = CLASSIFIED
                .iter()
                .position(|(f, piece, _)| rel == *f && line.contains(piece));
            if let Some(i) = hit {
                classified += 1;
                used.insert(i);
            } else {
                unclassified.push(format!("{rel}:{}: {}", n + 1, line.trim()));
            }
        }
    }
    eprintln!("is_hidden: {lines} lines, {classified} classified");
    assert!(
        lines > 0,
        "the walk found no is_hidden line in {} files",
        files.len()
    );
    assert!(
        unclassified.is_empty(),
        "an is_hidden read outside the visibility fragment — splice \
         `exclusion::visibility::project_visible` or classify it with a reason:\n{}",
        unclassified.join("\n")
    );
    let stale: Vec<_> = CLASSIFIED
        .iter()
        .enumerate()
        .filter(|(i, _)| !used.contains(i))
        .map(|(_, (f, piece, _))| format!("{f}: {piece:?}"))
        .collect();
    assert!(
        stale.is_empty(),
        "classified entries that match no line: {stale:?}"
    );
}

/// A project with one present copy per path; `excluded` marks each copy. Returns its id.
fn project(conn: &Connection, name: &str, copies: &[(&str, &str, bool)]) -> i64 {
    conn.execute(
        "INSERT INTO project (name, seed_basename, last_touched_at, created_at, updated_at)
         VALUES (?1, ?1, ?2, ?2, ?2)",
        params![name, NOW - 100],
    )
    .unwrap();
    let id = conn.last_insert_rowid();
    for (path, presence, excluded) in copies {
        conn.execute(
            "INSERT INTO location (project_id, kind, distro, path_bytes, path_key, path_display,
                                   volume_key, store_key, presence, repo_kind, excluded)
             VALUES (?1, 'linux', '', CAST(?2 AS BLOB), CAST(?2 AS BLOB), ?2, 'v', 's', ?3,
                     'worktree', ?4)",
            params![id, path, presence, i64::from(*excluded)],
        )
        .unwrap();
    }
    id
}

fn list(index: &Index, args: Value) -> Vec<Value> {
    let sink = CollectingSink::default();
    let jobs = codotheca_core::jobs::NullJobSink;
    let mounts = codotheca_core::testing::FakeMountResolver::new();
    let ctx = ProjectsCtx {
        exclusions: &codotheca_core::exclusion::gate::ExclusionGate::default(),
        index,
        events: &sink,
        jobs: &jobs,
        mounts: &mounts,
        sync: &codotheca_core::sync::runner::NullSyncSink,
        now: NOW,
        tz_offset_min: 0,
    };
    let page = dispatch_projects_command(&ctx, "projects.list", args)
        .expect("owned")
        .expect("ok");
    page["rows"].as_array().unwrap().clone()
}

fn ids(rows: &[Value]) -> BTreeSet<i64> {
    rows.iter().map(|r| r["id"].as_i64().unwrap()).collect()
}

#[test]
fn ac_p4_56_23_a_fully_excluded_project_leaves_the_bare_query_and_every_count() {
    let dir = tempfile::tempdir().unwrap();
    let index = Index::open_at(dir.path(), NOW).unwrap();
    let conn = index.conn();
    // `apply_coverage` writes `location.excluded` in production; this reads it back.
    let gone = project(conn, "gone", &[("/r/gone", "present", true)]);
    let mixed = project(
        conn,
        "mixed",
        &[
            ("/r/mixed-a", "present", true),
            ("/r/mixed-b", "offline", false),
        ],
    );
    let plain = project(conn, "plain", &[("/r/plain", "present", false)]);
    let bare = project(conn, "nocopy", &[]);
    let removed_only = project(conn, "removed", &[("/r/removed", "present", true)]);
    conn.execute(
        "UPDATE location SET removed_at = ?1 WHERE project_id = ?2",
        params![NOW, removed_only],
    )
    .unwrap();

    assert!(is_fully_excluded(conn, ProjectId(gone)).unwrap());
    for id in [mixed, plain, bare, removed_only, 9_999] {
        assert!(
            !is_fully_excluded(conn, ProjectId(id)).unwrap(),
            "project {id}"
        );
    }

    let shelf = list(&index, json!({}));
    eprintln!("visibility_fragment: bare query {} rows", shelf.len());
    assert_eq!(
        ids(&shelf),
        BTreeSet::from([mixed, plain, bare, removed_only])
    );
    assert!(shelf.iter().all(|r| r["isExcluded"] == json!(false)));

    // U35: an allowed copy is primary over an excluded one, whatever its presence.
    let mixed_row = shelf.iter().find(|r| r["id"] == json!(mixed)).unwrap();
    let allowed: i64 = conn
        .query_row(
            "SELECT id FROM location WHERE path_display = '/r/mixed-b'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(mixed_row["primaryLocation"]["id"], json!(allowed));

    let opted = list(&index, json!({ "query": "is:excluded" }));
    assert_eq!(ids(&opted), BTreeSet::from([gone]));
    assert_eq!(opted[0]["isExcluded"], json!(true));

    assert_eq!(
        population(conn).unwrap().total,
        4,
        "the reveal's population"
    );
}
