#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use codotheca_core::index::path::{PathPlatform, StoredPath};

#[test]
fn a_non_utf8_path_survives_in_bytes_and_is_mangled_in_display() {
    // 0xff is not valid UTF-8 anywhere. A real Linux filename may contain it.
    let raw = b"/home/u/pro\xffject".to_vec();
    let p = StoredPath::from_bytes(raw.clone(), PathPlatform::Unix);

    assert_eq!(
        p.bytes(),
        &raw[..],
        "path_bytes must be the exact operational bytes"
    );

    let (_, _, display) = p.as_params();
    assert!(
        display.contains('\u{fffd}'),
        "path_display is lossy and must show the replacement character"
    );
    assert_ne!(
        display.as_bytes(),
        &raw[..],
        "if display round-tripped, §1.10's rule would be unenforceable rather than merely broken"
    );
}

#[test]
fn windows_keys_fold_case_and_separators_and_unix_keys_do_not() {
    let a = StoredPath::from_bytes(br"C:\Users\Sam\Repo".to_vec(), PathPlatform::Windows);
    let b = StoredPath::from_bytes(br"c:/users/sam/repo".to_vec(), PathPlatform::Windows);
    assert_eq!(a.key(), b.key(), "on Windows these are one path");
    assert_ne!(
        a.bytes(),
        b.bytes(),
        "but the operational bytes are not rewritten"
    );

    let c = StoredPath::from_bytes(b"/home/sam/Repo".to_vec(), PathPlatform::Unix);
    let d = StoredPath::from_bytes(b"/home/sam/repo".to_vec(), PathPlatform::Unix);
    assert_ne!(
        c.key(),
        d.key(),
        "on Unix case is significant and folding would merge two repos"
    );
}

#[test]
fn keys_collapse_repeated_separators_and_drop_a_trailing_one() {
    let a = StoredPath::from_bytes(b"/home//sam/repo/".to_vec(), PathPlatform::Unix);
    let b = StoredPath::from_bytes(b"/home/sam/repo".to_vec(), PathPlatform::Unix);
    assert_eq!(a.key(), b.key());

    // The root is not emptied by the trailing-separator rule.
    let root = StoredPath::from_bytes(b"/".to_vec(), PathPlatform::Unix);
    assert_eq!(root.key(), b"/");
}

#[test]
fn as_params_yields_the_three_columns_in_ddl_order() {
    let p = StoredPath::from_bytes(br"C:\P\Thing".to_vec(), PathPlatform::Windows);
    let (bytes, key, display) = p.as_params();
    assert_eq!(bytes, br"C:\P\Thing");
    assert_eq!(key, b"c:/p/thing");
    assert_eq!(display, r"C:\P\Thing");
}

/// §1.10: `path_display` is write-once and never read by the core. An `INSERT` naming the
/// column is expected; a `SELECT` reading it back is the round-trip the invariant forbids.
#[test]
fn no_sql_outside_path_rs_selects_a_path_display() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut offenders = Vec::new();
    walk(&root.join("src"), &mut |file: &std::path::Path| {
        if file.ends_with("index/path.rs") {
            return;
        }
        let Ok(text) = std::fs::read_to_string(file) else {
            return;
        };
        let lowered = text.to_ascii_lowercase();
        for statement in lowered.split(';') {
            if statement.contains("select") && statement.contains("path_display") {
                offenders.push(file.display().to_string());
                break;
            }
        }
    });
    assert!(
        offenders.is_empty(),
        "these files read path_display back: {offenders:?} — \
         route the read through path::display_paths_for_ui"
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

use codotheca_core::index::migrate::{apply_all, MIGRATIONS};
use codotheca_core::index::path::{display_paths_for_ui, DisplayPathTable};
use codotheca_core::index::{open_connection, Index};

#[test]
fn the_one_door_returns_the_display_string_for_the_ui() {
    let dir = tempfile::tempdir().unwrap();
    let mut conn = open_connection(&Index::db_path(dir.path())).unwrap();
    apply_all(&mut conn, MIGRATIONS).unwrap();

    conn.execute(
        "INSERT INTO project (name, seed_basename, created_at, updated_at)
         VALUES ('thing', 'thing', 1, 1)",
        [],
    )
    .unwrap();
    let project_id = conn.last_insert_rowid();

    let p = StoredPath::from_bytes(b"/home/u/pro\xffject".to_vec(), PathPlatform::Unix);
    let (bytes, key, display) = p.as_params();
    conn.execute(
        "INSERT INTO location
           (project_id, kind, path_bytes, path_key, path_display,
            volume_key, store_key, presence, repo_kind)
         VALUES (?1, 'linux', ?2, ?3, ?4, 'vol', 'store', 'present', 'worktree')",
        rusqlite::params![project_id, bytes, key, display],
    )
    .unwrap();
    let id = conn.last_insert_rowid();

    let got = display_paths_for_ui(&conn, DisplayPathTable::Location, &[id]).unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].0, id);
    assert!(got[0].1.contains('\u{fffd}'));

    // An id that is not there yields no row rather than an empty string.
    let none = display_paths_for_ui(&conn, DisplayPathTable::Location, &[id + 99]).unwrap();
    assert!(none.is_empty());
}
