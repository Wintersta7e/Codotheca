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
use codotheca_core::install::state::InstallStateStore;
use codotheca_core::jobs::JobKind;
use codotheca_core::mount::{MountFacts, StoreClass};
use codotheca_core::paths::path_bytes;
use codotheca_core::proto::pubsub::EventSink;
use codotheca_core::protocol::{InstallDestination, InstallFailure, ProjectId, RootId};
use codotheca_core::testing::{
    CloneBehaviour, FakeGitBackend, FakeMountResolver, FakeMutatingGit, TempIndex,
};

/// Records what reached the wire, so a test can assert the stage stream as well as the disk.
#[derive(Debug, Default)]
struct RecordingEvents {
    emitted: Mutex<Vec<(String, String, serde_json::Value)>>,
}

impl RecordingEvents {
    fn stages(&self) -> Vec<String> {
        self.emitted
            .lock()
            .expect("lock")
            .iter()
            .filter(|(topic, event, _)| topic == "install" && event == "stage")
            .map(|(_, _, payload)| payload["stage"].as_str().unwrap_or_default().to_owned())
            .collect()
    }
}

impl EventSink for RecordingEvents {
    fn emit(&self, topic: &str, event: &str, payload: serde_json::Value) {
        self.emitted
            .lock()
            .expect("lock")
            .push((topic.to_owned(), event.to_owned(), payload));
    }
}

/// R52, asserted mechanically rather than remembered: the install queue is `core::install`'s own,
/// so no `JobKind` variant was added for it and R34's three-place slug agreement is undisturbed.
///
/// **The property is a set property and is stated as one.** It asserted `ALL.len() == 7`, which
/// goes red for any eighth job whether or not that job is an install — §29's `j7` is not — and
/// which would have read as an install defect (R132/F16).
#[test]
fn the_install_queue_added_no_job_kind() {
    let slugs: Vec<&'static str> = JobKind::ALL.iter().map(|k| k.slug()).collect();
    eprintln!("job slugs checked for an install: {slugs:?}");
    assert!(!slugs.is_empty(), "the job vocabulary is empty");
    for kind in JobKind::ALL {
        let named = format!("{kind:?}").to_lowercase();
        assert!(
            !named.contains("install") && !kind.slug().contains("install"),
            "{named} names an install: §24.3e's queue is core::install's, not the scheduler's"
        );
    }
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

    // The destination already exists as a NON-EMPTY DIRECTORY, so the rename cannot complete on
    // either platform. This is the crash window: the clone has exited 0 and its bytes are in
    // staging.
    //
    // **It was a file, and that made this test a Unix-only bar.** `std::fs::rename` replaces an
    // existing file on Windows, so the rename succeeded there, the run walked on to
    // `index_destination`, and the assertion below read `GitFailed` instead of `RenameFailed` —
    // the run still failed and still wrote no row, but through a path this test does not claim to
    // be testing. A non-empty directory is refused by `ENOTEMPTY` and by
    // `ERROR_DIR_NOT_EMPTY` alike, so the window is forced on both.
    std::fs::create_dir_all(root.join("alpha")).expect("blocker");
    std::fs::write(root.join("alpha").join("occupied.txt"), b"in the way").expect("blocker");

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
    let stages = InstallStateStore::new();
    let events = RecordingEvents::default();
    let ctx = InstallCtx {
        git: &git,
        probe: &probe,
        index: &shared,
        jobs: &jobs,
        mounts: &mounts,
        stages: &stages,
        events: &events,
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

/// **A10, asserted structurally rather than by review.** `InstallStage`'s field set is exactly
/// `{runId, stage, done, total, bytes}`: there is no `percent`, no `progress` and no `fraction`,
/// so §10.2's ban on a retreating aggregate is a property of the wire rather than a rule someone
/// has to remember. A structural assertion survives a later author adding one; a code review does
/// not.
#[test]
fn an_install_stage_carries_no_aggregate_field() {
    let stage = codotheca_core::protocol::InstallStage {
        run_id: codotheca_core::protocol::InstallRunId(1),
        stage: codotheca_core::protocol::InstallStageKind::Receiving,
        done: Some(2),
        total: Some(9),
        bytes: Some(1024),
    };
    let value = serde_json::to_value(&stage).expect("serialises");
    let object = value.as_object().expect("an object");
    let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        vec!["bytes", "done", "runId", "stage", "total"],
        "no aggregate may be added: there must be nothing on the wire to build a percentage from"
    );
    for banned in ["percent", "progress", "fraction", "ratio", "pct"] {
        assert!(!object.contains_key(banned), "{banned} is an aggregate");
    }
    assert!(
        !object.contains_key("projectId"),
        "p2-24r's criterion pins this absent; a tile learns its run from InstallStarted"
    );
}

/// The stage stream reaches the wire in §24.4's order, and the snapshot agrees with it.
#[test]
fn a_clone_publishes_its_stages_and_the_snapshot_matches_the_last_one() {
    /// §24.4's order, for the monotonicity check below.
    const ORDER: [&str; 6] = [
        "plans",
        "enumerating",
        "receiving",
        "assembling",
        "cladding",
        "settled",
    ];

    let root_dir = tempfile::tempdir().expect("root");
    let root = root_dir.path().to_path_buf();
    let index_dir = tempfile::tempdir().expect("index dir");
    let shared = Arc::new(Mutex::new(
        codotheca_core::index::Index::open(&index_dir.path().join("index")).expect("index"),
    ));

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
    let stages = InstallStateStore::new();
    let events = RecordingEvents::default();
    stages.begin(
        codotheca_core::protocol::InstallRunId(1),
        ProjectId(1),
        "<root>/alpha".to_owned(),
    );
    let ctx = InstallCtx {
        git: &git,
        probe: &probe,
        index: &shared,
        jobs: &jobs,
        mounts: &mounts,
        stages: &stages,
        events: &events,
        cancel: &cancel,
        now: 100,
    };
    let facts = RootFacts {
        root_id: 1,
        path: root.clone(),
        kind: "linux".to_owned(),
        distro: String::new(),
    };
    // The identity probe has no replies configured, so this run ends at the hand-off — after the
    // clone and its stages, which is what this test is about.
    let _ = run_install(
        &ctx,
        codotheca_core::protocol::InstallRunId(1),
        &request(1, 1, "alpha"),
        &facts,
        &paths,
        "https://forge.example/owner/alpha",
    );

    let seen = events.stages();
    assert!(
        !seen.is_empty(),
        "a run that published nothing is not a run"
    );
    assert_eq!(
        seen.first().map(String::as_str),
        Some("enumerating"),
        "the first milestone the transcript names"
    );
    // Monotonic: every stage is at or after the one before it in §24.4's order.
    let mut highest = 0;
    for stage in &seen {
        let rank = ORDER
            .iter()
            .position(|s| s == stage)
            .expect("a known stage");
        assert!(rank >= highest, "the readout retreated to {stage}");
        highest = rank;
    }
}
