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

/// Records what reached the wire.
#[derive(Debug, Default)]
struct RecordingEvents {
    emitted: Mutex<Vec<(String, String, serde_json::Value)>>,
}

impl RecordingEvents {
    fn failed_reasons(&self) -> Vec<String> {
        self.emitted
            .lock()
            .expect("lock")
            .iter()
            .filter(|(topic, event, _)| topic == "install" && event == "failed")
            .map(|(_, _, payload)| payload["reason"].as_str().unwrap_or_default().to_owned())
            .collect()
    }
}

impl codotheca_core::proto::pubsub::EventSink for RecordingEvents {
    fn emit(&self, topic: &str, event: &str, payload: serde_json::Value) {
        self.emitted
            .lock()
            .expect("lock")
            .push((topic.to_owned(), event.to_owned(), payload));
    }
}

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

/// One project, one root and one pushed run, so the cancel test itself stays about cancelling.
fn seed_run(
    index: &Arc<Mutex<codotheca_core::index::Index>>,
    queue: &codotheca_core::install::queue::InstallQueue,
    root: &std::path::Path,
    paths: &codotheca_core::install::run::RunPaths,
) -> codotheca_core::protocol::InstallRunId {
    use codotheca_core::install::queue::InstallRequest;
    use codotheca_core::protocol::{InstallDestination, ProjectId, RootId};

    let _guard = codotheca_core::proto::txguard::TxGuard::enter();
    let held = index.lock().expect("lock");
    held.conn()
        .execute(
            "INSERT INTO project (id, name, seed_basename, created_at, updated_at)
             VALUES (1, 'a', 'alpha', 0, 0)",
            [],
        )
        .expect("project");
    held.conn()
        .execute(
            "INSERT INTO scan_root (id, kind, distro, path_bytes, path_key, path_display,
                                    added_by, added_at)
             VALUES (1, 'linux', '', ?1, ?1, 'r', 'user', 0)",
            [codotheca_core::paths::path_bytes(root)],
        )
        .expect("root");
    let tx = held.conn().unchecked_transaction().expect("tx");
    let run = queue
        .push(
            &tx,
            InstallRequest {
                project: ProjectId(1),
                root: RootId(1),
                destination: InstallDestination {
                    root_id: RootId(1),
                    seed_basename: "alpha".to_owned(),
                    display: "r/alpha".to_owned(),
                },
            },
            &codotheca_core::paths::path_bytes(&paths.staging),
            &codotheca_core::paths::path_bytes(&paths.destination),
            0,
        )
        .expect("push");
    tx.commit().expect("commit");
    run
}

/// **AC-P2-24-7's cancel half.** After a cancel, in order: the group is gone, the staging
/// directory is removed under the staging warrant, the `install_run` row reads `cancelled`, and
/// there is **no `location` row** — the run never reached the rename.
#[test]
fn a_cancelled_run_removes_its_staging_and_leaves_no_location() {
    use codotheca_core::install::queue::InstallQueue;
    use codotheca_core::install::run::{finish_failed, paths_for, InstallCtx};
    use codotheca_core::install::state::InstallStateStore;
    use codotheca_core::protocol::{InstallFailure, InstallRunId, ProjectId};

    let dir = tempfile::tempdir().expect("tmp");
    let root = dir.path().join("library");
    std::fs::create_dir_all(&root).expect("root");
    let paths = paths_for(&root, "alpha").expect("paths");
    // The clone got as far as writing into staging before it was killed.
    std::fs::create_dir_all(paths.staging.join("objects")).expect("partial clone");

    let index = index_at(dir.path());
    let queue = InstallQueue::new();
    let run = seed_run(&index, &queue, &root, &paths);

    // The token is what the group kill hangs off; firing it is the whole of `install.cancel`.
    let token = codotheca_core::cancel::CancelToken::new();
    queue.register_cancel(run, token.clone());
    assert!(
        codotheca_core::install::run::cancel_install(&queue, run).is_ok(),
        "a live run is cancellable"
    );
    assert!(token.is_cancelled(), "the clone's own token fired");
    assert!(
        codotheca_core::install::run::cancel_install(&queue, InstallRunId(4242)).is_err(),
        "a run this core is not running is refused rather than killed blind — a replay would \
         otherwise fire at a group a later run may by then own"
    );

    let stages = InstallStateStore::new();
    let events = RecordingEvents::default();
    let git = codotheca_core::testing::FakeMutatingGit::new(
        codotheca_core::testing::CloneBehaviour::Succeed,
    );
    let probe = codotheca_core::testing::FakeGitBackend::new();
    let mounts = codotheca_core::testing::FakeMountResolver::new();
    let jobs = codotheca_core::jobs::NullJobSink;
    let ctx = InstallCtx {
        git: &git,
        probe: &probe,
        index: &index,
        jobs: &jobs,
        mounts: &mounts,
        stages: &stages,
        events: &events,
        cancel: &token,
        now: 500,
    };
    finish_failed(&ctx, run, ProjectId(1), InstallFailure::Cancelled);

    assert!(
        !paths.staging.exists(),
        "the staging directory is removed under the warrant"
    );
    let held = index.lock().expect("lock");
    let (state, ended): (String, Option<i64>) = held
        .conn()
        .query_row(
            "SELECT state, ended_at FROM install_run WHERE id = ?1",
            [run.0],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("row");
    assert_eq!(state, "cancelled", "not `failed`: the user asked for this");
    assert_eq!(ended, Some(500));
    let locations: i64 = held
        .conn()
        .query_row("SELECT COUNT(*) FROM location", [], |row| row.get(0))
        .expect("count");
    assert_eq!(locations, 0, "the run never reached the rename");
    assert!(
        events.failed_reasons().contains(&"cancelled".to_owned()),
        "install.failed names the reason a surface branches on"
    );
}
