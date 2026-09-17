#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

//! §24.3e's commit order and §24.3e's queue.
//!
//! **AC-P2-24-9**: a crash between the clone exiting and the rename completing leaves **no**
//! `location` row. The clone writes only into staging, so the destination does not exist and no
//! surface can read the project as cloned.

use std::sync::{Arc, Mutex};

use codotheca_core::cancel::CancelToken;
use codotheca_core::install::queue::{InstallQueue, InstallRequest};
use codotheca_core::install::run::{paths_for, run_install, InstallCtx, RootFacts};
use codotheca_core::install::staging::STAGING_DIR_NAME;
use codotheca_core::jobs::JobKind;
use codotheca_core::mount::{MountFacts, StoreClass};
use codotheca_core::paths::path_bytes;
use codotheca_core::protocol::{InstallDestination, InstallFailure, ProjectId, RootId};
use codotheca_core::testing::{
    CloneBehaviour, FakeGitBackend, FakeMountResolver, FakeMutatingGit, TempIndex,
};

/// R52, asserted mechanically rather than remembered: the install queue is `core::install`'s own,
/// so no `JobKind` variant was added for it and R34's three-place slug agreement is undisturbed.
#[test]
fn the_install_queue_added_no_job_kind() {
    assert_eq!(
        JobKind::ALL.len(),
        7,
        "an install is not a job: §24.3e's queue is core::install's, not the scheduler's"
    );
}

fn request(project: i64, root: i64, seed: &str) -> InstallRequest {
    InstallRequest {
        project: ProjectId(project),
        root: RootId(root),
        destination: InstallDestination {
            root_id: RootId(root),
            seed_basename: seed.to_owned(),
            display: format!("<root>/{seed}"),
        },
    }
}

/// Capacity one in flight, FIFO, and the promotion is the queue's decision rather than its
/// caller's — `finish` returns the next request rather than letting a caller pick one.
#[test]
fn two_starts_run_strictly_one_at_a_time_in_arrival_order() {
    let index = TempIndex::new();
    let binding = index.index();
    let _guard = codotheca_core::proto::txguard::TxGuard::enter();
    let conn = binding.conn();
    conn.execute(
        "INSERT INTO project (id, name, seed_basename, created_at, updated_at)
         VALUES (1, 'alpha', 'alpha', 0, 0), (2, 'beta', 'beta', 0, 0)",
        [],
    )
    .expect("projects");
    conn.execute(
        "INSERT INTO scan_root (id, kind, distro, path_bytes, path_key, path_display,
                                added_by, added_at)
         VALUES (1, 'linux', '', ?1, ?1, '/r', 'user', 0)",
        [b"/r".to_vec()],
    )
    .expect("root");

    let queue = InstallQueue::new();
    let tx = conn.unchecked_transaction().expect("tx");
    let first = queue
        .push(&tx, request(1, 1, "alpha"), b"/r/.s/alpha", b"/r/alpha", 0)
        .expect("first");
    let second = queue
        .push(&tx, request(2, 1, "beta"), b"/r/.s/beta", b"/r/beta", 0)
        .expect("second");
    tx.commit().expect("commit");

    assert_eq!(
        queue.in_flight(),
        Some(first),
        "the first arrival runs first"
    );
    assert_eq!(queue.waiting(), 1, "the second waits rather than running");

    // A caller cannot retire a run that is not the one in flight.
    assert!(
        queue.finish(second).is_none(),
        "only the in-flight run may be retired"
    );
    assert_eq!(queue.in_flight(), Some(first));

    let promoted = queue.finish(first).expect("the second is promoted");
    assert_eq!(promoted.0, second, "FIFO: arrival order, never id order");
    assert_eq!(queue.in_flight(), Some(second));
    assert_eq!(queue.waiting(), 0);
    assert!(queue.finish(second).is_none(), "nothing is left to promote");
    assert_eq!(queue.in_flight(), None);
}

/// The durable row is written **before the first byte**, which is what the staging warrant later
/// rests on: a staging directory with no row behind it cannot be warranted.
#[test]
fn the_run_row_exists_before_anything_is_cloned() {
    let index = TempIndex::new();
    let binding = index.index();
    let _guard = codotheca_core::proto::txguard::TxGuard::enter();
    let conn = binding.conn();
    conn.execute(
        "INSERT INTO project (id, name, seed_basename, created_at, updated_at)
         VALUES (1, 'alpha', 'alpha', 0, 0)",
        [],
    )
    .expect("project");
    conn.execute(
        "INSERT INTO scan_root (id, kind, distro, path_bytes, path_key, path_display,
                                added_by, added_at)
         VALUES (1, 'linux', '', ?1, ?1, '/r', 'user', 0)",
        [b"/r".to_vec()],
    )
    .expect("root");

    let queue = InstallQueue::new();
    let tx = conn.unchecked_transaction().expect("tx");
    let run = queue
        .push(&tx, request(1, 1, "alpha"), b"/r/.s/alpha", b"/r/alpha", 77)
        .expect("push");
    tx.commit().expect("commit");

    let (lifecycle, phase): (String, String) = conn
        .query_row(
            "SELECT state, stage FROM install_run WHERE id = ?1",
            [run.0],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("the row exists");
    assert_eq!(lifecycle, "running", "begun and not ended");
    assert_eq!(
        phase, "plans",
        "§24.4's first stage: accepted, and honest about not yet cloning"
    );
}

/// **AC-P2-24-9.** The clone writes into staging only. When the rename cannot complete, the run
/// fails and there is **no `location` row** — a project is not cloned until that row commits, and
/// mid-install is not a third era.
#[test]
fn a_crash_between_the_clone_and_the_rename_leaves_no_location_row() {
    let root_dir = tempfile::tempdir().expect("root");
    let root = root_dir.path().to_path_buf();

    let index_dir = tempfile::tempdir().expect("index dir");
    let shared = Arc::new(Mutex::new(
        codotheca_core::index::Index::open(&index_dir.path().join("index")).expect("index"),
    ));
    {
        let _guard = codotheca_core::proto::txguard::TxGuard::enter();
        let held = shared.lock().expect("lock");
        held.conn()
            .execute(
                "INSERT INTO project (id, name, seed_basename, created_at, updated_at)
                 VALUES (1, 'alpha', 'alpha', 0, 0)",
                [],
            )
            .expect("project");
    }

    // The destination already exists as a FILE, so the rename cannot complete. This is the
    // crash window: the clone has exited 0 and its bytes are in staging.
    std::fs::write(root.join("alpha"), b"in the way").expect("blocker");

    let paths = paths_for(&root, "alpha").expect("paths");
    let git = FakeMutatingGit::new(CloneBehaviour::Succeed);
    let probe = FakeGitBackend::new();
    let mounts = FakeMountResolver::new();
    mounts.map(
        root.clone(),
        MountFacts {
            store_key: "store".to_owned(),
            volume_key: Some("vol".to_owned()),
            class: StoreClass::Local,
        },
    );
    let jobs = codotheca_core::jobs::NullJobSink;
    let cancel = CancelToken::new();
    let ctx = InstallCtx {
        git: &git,
        probe: &probe,
        index: &shared,
        jobs: &jobs,
        mounts: &mounts,
        cancel: &cancel,
        now: 100,
    };
    let facts = RootFacts {
        root_id: 1,
        path: root.clone(),
        kind: "linux".to_owned(),
        distro: String::new(),
    };

    let outcome = run_install(
        &ctx,
        codotheca_core::protocol::InstallRunId(1),
        &request(1, 1, "alpha"),
        &facts,
        &paths,
        "https://forge.example/owner/alpha",
    );
    assert_eq!(
        outcome,
        Err(InstallFailure::RenameFailed),
        "the rename is where this run ends"
    );

    let held = shared.lock().expect("lock");
    let locations: i64 = held
        .conn()
        .query_row("SELECT COUNT(*) FROM location", [], |row| row.get(0))
        .expect("count");
    drop(held);
    assert_eq!(
        locations, 0,
        "AC-P2-24-9: no location row may exist when the rename did not complete"
    );

    // And the clone's bytes are where the sweep can warrant them, not loose under the root.
    assert!(
        root.join(STAGING_DIR_NAME).join("alpha").exists(),
        "the partial clone stays inside the staging directory"
    );
    assert_eq!(
        path_bytes(&paths.staging),
        path_bytes(&root.join(STAGING_DIR_NAME).join("alpha")),
        "and that is the path the durable row recorded"
    );
}
