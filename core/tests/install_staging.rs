#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

//! **AC-P2-24-7**: what a killed clone leaves, what the sweep does with it, and what the user is
//! told about the part it will not touch.

use std::sync::{Arc, Mutex};

use codotheca_core::install::staging::{sweep_staging, STAGING_DIR_NAME};
use codotheca_core::protocol::ProblemKind;
use codotheca_core::surfaces::problems::{abandoned_installs, list, GROUP_ORDER};

fn index_at(dir: &std::path::Path) -> Arc<Mutex<codotheca_core::index::Index>> {
    Arc::new(Mutex::new(
        codotheca_core::index::Index::open(&dir.join("index")).expect("index"),
    ))
}

/// The ninth group exists and is ordered last — *indexed with a qualification* comes after
/// *did not index*.
#[test]
fn the_group_order_is_nine_and_abandoned_install_is_the_ninth() {
    assert_eq!(GROUP_ORDER.len(), 9);
    assert_eq!(GROUP_ORDER[8], ProblemKind::AbandonedInstall);
}

/// **R26 stays closed.** `abandoned_install` reads `install_run`; `scan_problem` never stores it,
/// so the table's CHECK constraint is untouched and its parser must not accept the string.
#[test]
fn a_stored_abandoned_install_string_is_not_a_scan_problem_kind() {
    assert!(
        codotheca_core::surfaces::problems::problem_kind_from_storage("abandoned_install")
            .is_none(),
        "accepting it here would mean the scan_problem CHECK had to change, which is a migration"
    );
    for stored in [
        "permission_denied",
        "untrusted_repo",
        "unreadable_repo",
        "clock_skew",
        "non_utf8_path",
        "offline_store",
    ] {
        assert!(
            codotheca_core::surfaces::problems::problem_kind_from_storage(stored).is_some(),
            "{stored} is one of the six the table does store"
        );
    }
}

/// A run this session recorded is warranted and removed; one it did not is **left in place** and
/// named, with its path, for the user to remove by hand.
#[test]
fn the_sweep_removes_what_it_can_warrant_and_reports_what_it_cannot() {
    let dir = tempfile::tempdir().expect("tmp");
    let root = dir.path().join("library");
    let staging_root = root.join(STAGING_DIR_NAME);
    std::fs::create_dir_all(staging_root.join("warranted")).expect("mkdir");
    std::fs::create_dir_all(staging_root.join("stranger")).expect("mkdir");

    let index = index_at(dir.path());
    {
        let _guard = codotheca_core::proto::txguard::TxGuard::enter();
        let held = index.lock().expect("lock");
        held.conn()
            .execute(
                "INSERT INTO project (id, name, seed_basename, created_at, updated_at)
                 VALUES (1, 'w', 'warranted', 0, 0)",
                [],
            )
            .expect("project");
        held.conn()
            .execute(
                "INSERT INTO scan_root (id, kind, distro, path_bytes, path_key, path_display,
                                        added_by, added_at)
                 VALUES (1, 'linux', '', ?1, ?1, 'r', 'user', 0)",
                [codotheca_core::paths::path_bytes(&root)],
            )
            .expect("root");
        // Only the first has a durable row, so only the first can be warranted.
        held.conn()
            .execute(
                "INSERT INTO install_run
                    (id, project_id, root_id, staging_bytes, destination_bytes, state, started_at)
                 VALUES (1, 1, 1, ?1, ?2, 'failed', 0)",
                rusqlite::params![
                    codotheca_core::paths::path_bytes(&staging_root.join("warranted")),
                    codotheca_core::paths::path_bytes(&root.join("warranted")),
                ],
            )
            .expect("run");
    }

    let report = sweep_staging(&index, std::slice::from_ref(&root));
    assert_eq!(report.removed, 1, "the run this session recorded");
    assert!(
        !staging_root.join("warranted").exists(),
        "a warranted staging directory goes"
    );
    assert_eq!(report.unwarranted.len(), 1);
    assert!(
        staging_root.join("stranger").exists(),
        "what cannot be warranted is LEFT IN PLACE — removing it would be the one destructive \
         operation this boundary exists to prevent"
    );
    assert_eq!(report.unwarranted[0], staging_root.join("stranger"));

    // And the user is told about it, by path.
    let held = index.lock().expect("lock");
    let items = abandoned_installs(held.conn()).expect("listed");
    assert!(
        items.is_empty(),
        "the only row left is the removed one, whose directory is gone: a run that ended \
         untidily is not a directory the user has to remove"
    );
}

/// The row survives its directory's removal, and the group reports only what is still on disk.
#[test]
fn a_row_whose_directory_is_still_there_is_reported_with_its_path() {
    let dir = tempfile::tempdir().expect("tmp");
    let root = dir.path().join("library");
    let staging_root = root.join(STAGING_DIR_NAME);
    std::fs::create_dir_all(staging_root.join("left")).expect("mkdir");

    let index = index_at(dir.path());
    let _guard = codotheca_core::proto::txguard::TxGuard::enter();
    let held = index.lock().expect("lock");
    held.conn()
        .execute(
            "INSERT INTO project (id, name, seed_basename, created_at, updated_at)
             VALUES (1, 'l', 'left', 0, 0)",
            [],
        )
        .expect("project");
    held.conn()
        .execute(
            "INSERT INTO scan_root (id, kind, distro, path_bytes, path_key, path_display,
                                    added_by, added_at)
             VALUES (1, 'linux', '', ?1, ?1, 'r', 'user', 0)",
            [codotheca_core::paths::path_bytes(&root)],
        )
        .expect("root");
    held.conn()
        .execute(
            "INSERT INTO install_run
                (id, project_id, root_id, staging_bytes, destination_bytes, state, started_at)
             VALUES (1, 1, 1, ?1, ?2, 'cancelled', 0)",
            rusqlite::params![
                codotheca_core::paths::path_bytes(&staging_root.join("left")),
                codotheca_core::paths::path_bytes(&root.join("left")),
            ],
        )
        .expect("run");

    let items = abandoned_installs(held.conn()).expect("listed");
    assert_eq!(items.len(), 1);
    assert!(
        items[0].path_display.contains("left"),
        "the path is named so the user can act on it: {}",
        items[0].path_display
    );
    assert!(
        items[0]
            .detail
            .as_deref()
            .unwrap_or_default()
            .contains("hand"),
        "and the detail says it must be removed by hand"
    );
    assert_eq!(
        items[0].location_id, None,
        "a staging directory is not a location — NULL is not observed, never zero"
    );
}

/// **The one thing in this task a compiler will not catch.**
///
/// `list`'s match has an `other =>` arm that queries `scan_problem`. Without its own arm,
/// `abandoned_install` falls through to it, finds nothing — that table never stores the kind —
/// and the group reports zero items and vanishes. Measured: deleting the arm left every other
/// test in this file green, because they call `abandoned_installs` directly and never reach the
/// dispatch. This one goes through `problems.list`, which is the surface the user sees.
#[test]
fn the_ninth_group_reaches_problems_list_and_not_only_its_own_function() {
    let dir = tempfile::tempdir().expect("tmp");
    let root = dir.path().join("library");
    let staging_root = root.join(STAGING_DIR_NAME);
    std::fs::create_dir_all(staging_root.join("left")).expect("mkdir");

    let index = index_at(dir.path());
    let _guard = codotheca_core::proto::txguard::TxGuard::enter();
    let held = index.lock().expect("lock");
    let conn = held.conn();
    conn.execute(
        "INSERT INTO project (id, name, seed_basename, created_at, updated_at)
         VALUES (1, 'l', 'left', 0, 0)",
        [],
    )
    .expect("project");
    conn.execute(
        "INSERT INTO scan_root (id, kind, distro, path_bytes, path_key, path_display,
                                added_by, added_at)
         VALUES (1, 'linux', '', ?1, ?1, 'r', 'user', 0)",
        [codotheca_core::paths::path_bytes(&root)],
    )
    .expect("root");
    conn.execute(
        "INSERT INTO install_run
            (id, project_id, root_id, staging_bytes, destination_bytes, state, started_at)
         VALUES (1, 1, 1, ?1, ?2, 'failed', 0)",
        rusqlite::params![
            codotheca_core::paths::path_bytes(&staging_root.join("left")),
            codotheca_core::paths::path_bytes(&root.join("left")),
        ],
    )
    .expect("run");
    // `list` reports against a scan run, so there has to be one for it to answer at all.
    conn.execute(
        "INSERT INTO scan_run
            (id, generation, started_at, ended_at, mode, roots_json, walked_dirs, found_repos)
         VALUES (1, 1, 0, 1, 'full', '[]', 10, 1)",
        [],
    )
    .expect("scan run");

    let problems = list(conn, None).expect("listed");
    let group = problems
        .groups
        .iter()
        .find(|g| g.kind == ProblemKind::AbandonedInstall)
        .expect("the ninth group must reach problems.list, not just its own function");
    assert_eq!(group.count, 1);
    assert!(group.items[0].path_display.contains("left"));

    // And it counts, like deferred_slow and unlike ambiguous_lineage: it is a directory on the
    // user's disk that this app made and cannot clean up.
    assert_eq!(
        problems.header.problem_count,
        Some(1),
        "an abandoned install counts toward the figure §11.1 shows"
    );
}
