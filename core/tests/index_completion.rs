#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! The completion columns: not computed is two NULLs, a real zero is writable, one writer.

use codotheca_core::index::completion::{get_completion, set_completion, Completion};
use codotheca_core::index::migrate::{apply_all, MIGRATIONS};
use codotheca_core::index::{open_connection, Index, IndexError};
use codotheca_core::protocol::ProjectId;

fn fresh() -> (tempfile::TempDir, rusqlite::Connection, ProjectId) {
    let dir = tempfile::tempdir().unwrap();
    let mut conn = open_connection(&Index::db_path(dir.path())).unwrap();
    apply_all(&mut conn, MIGRATIONS).unwrap();
    conn.execute(
        "INSERT INTO project (name, seed_basename, created_at, updated_at)
         VALUES ('thing', 'thing', 1, 1)",
        [],
    )
    .unwrap();
    let id = ProjectId(conn.last_insert_rowid());
    (dir, conn, id)
}

#[test]
fn a_project_starts_not_computed_which_is_phase_1_for_every_project() {
    let (_d, conn, id) = fresh();
    assert_eq!(get_completion(&conn, id).unwrap(), Completion::NotComputed);
}

#[test]
fn not_computed_writes_two_nulls_and_never_a_zero() {
    let (_d, conn, id) = fresh();
    set_completion(
        &conn,
        id,
        Completion::Computed {
            lit: 3,
            applicable: 10,
        },
    )
    .unwrap();
    set_completion(&conn, id, Completion::NotComputed).unwrap();

    let (lit, applicable): (Option<i64>, Option<i64>) = conn
        .query_row(
            "SELECT completion_lit, completion_applicable FROM project WHERE id=?1",
            [id.0],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(lit, None);
    assert_eq!(applicable, None);
    assert_eq!(get_completion(&conn, id).unwrap(), Completion::NotComputed);
}

#[test]
fn a_genuine_zero_of_ten_is_writable_and_readable() {
    let (_d, conn, id) = fresh();
    let zero = Completion::Computed {
        lit: 0,
        applicable: 10,
    };
    set_completion(&conn, id, zero).unwrap();
    assert_eq!(get_completion(&conn, id).unwrap(), zero);
}

#[test]
fn zero_applicable_checks_is_refused_before_it_reaches_sqlite() {
    let (_d, conn, id) = fresh();
    match set_completion(
        &conn,
        id,
        Completion::Computed {
            lit: 0,
            applicable: 0,
        },
    ) {
        Err(IndexError::CompletionNotComputable) => {}
        other => panic!("expected CompletionNotComputable, got {other:?}"),
    }
    assert_eq!(get_completion(&conn, id).unwrap(), Completion::NotComputed);
}

#[test]
fn more_lit_than_applicable_is_refused() {
    let (_d, conn, id) = fresh();
    assert!(set_completion(
        &conn,
        id,
        Completion::Computed {
            lit: 11,
            applicable: 10
        }
    )
    .is_err());
}

/// **`AC-P3-31-4`. [p3] Inverted, not deleted** (§31.10).
///
/// §1.2 and §7.7a said nothing in phase 1 computes completion, so nothing called this writer.
/// §31.5 gives it **exactly one** production call site — the evaluator — and the assertion turns
/// over with the claim rather than being dropped.
///
/// **And the walk now prints its file count and fails at zero.** It printed nothing before, which
/// is the *gate whose passing run scans zero files* pattern its own sibling at
/// `core/tests/remote_no_completion_writer.rs:74-81` already got right: a walk over an empty root
/// finds no second caller and passes for the wrong reason.
#[test]
fn set_completion_has_exactly_one_production_call_site() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut scanned = 0_u32;
    let mut callers = Vec::new();
    walk(&root, &mut |file: &std::path::Path| {
        let Ok(text) = std::fs::read_to_string(file) else {
            return;
        };
        // Counted after the read, so a file that vanished between the walk and the read is
        // skipped BEFORE it is counted and the guard below keeps meaning what it says.
        scanned += 1;
        if file.ends_with("index/completion.rs") {
            return;
        }
        if text.contains("set_completion") {
            callers.push(file.display().to_string());
        }
    });
    eprintln!("index_completion: walked {scanned} core source file(s)");
    assert!(scanned > 0, "the walk read no file, so it proved nothing");
    assert_eq!(
        callers.len(),
        1,
        "§31.5: the evaluator is the only caller, and a second one is a second source of truth: \
         {callers:?}"
    );
    assert!(
        callers[0].contains("completion"),
        "the one caller is §31's evaluator: {callers:?}"
    );
}

fn walk(dir: &std::path::Path, f: &mut dyn FnMut(&std::path::Path)) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            walk(&p, f);
        } else if p.extension().is_some_and(|e| e == "rs") {
            f(&p);
        }
    }
}
