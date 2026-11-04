#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! §43.10: the end of first run is stamped where first run ends — the turn's answer when no walk
//! is running, the walk's end when one is, and the first walk to end for a core that starts with
//! a shelf and no stamp. A failed walk ends nothing; an `autostart` patch is not an ending.

use std::sync::{Arc, Mutex};

use codotheca_core::assembly::startup::run_startup;
use codotheca_core::firstrun::first_run_completed_at;
use codotheca_core::index::Index;
use codotheca_core::motivation::first_run::FirstRunArm;
use codotheca_core::proto::dispatch::CommandHandler as _;
use codotheca_core::proto::pubsub::Queued;
use codotheca_core::protocol::Topic;
use codotheca_core::scan::ScanLauncher;
use codotheca_core::testing::{core_handler, CoreHandlerParts, HeldOutcome, HeldScanLauncher};

const NOW: i64 = 1_750_000_000;

struct Rig {
    _dir: tempfile::TempDir,
    index: Arc<Mutex<Index>>,
    held: Arc<HeldScanLauncher>,
    parts: CoreHandlerParts,
}

fn rig() -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let index = Arc::new(Mutex::new(Index::open_at(dir.path(), NOW).unwrap()));
    let held = Arc::new(HeldScanLauncher::new(Arc::clone(&index)));
    let launcher: Arc<dyn ScanLauncher> = held.clone();
    let parts = core_handler(dir.path(), &index, NOW, launcher);
    Rig {
        _dir: dir,
        index,
        held,
        parts,
    }
}

impl Rig {
    fn stamp(&self) -> Option<i64> {
        first_run_completed_at(self.index.lock().unwrap().conn()).unwrap()
    }

    fn start_scan(&mut self) {
        self.parts
            .handler
            .handle("scan.start", serde_json::json!({"full": false}))
            .expect("scan.start is answerable");
    }

    /// The turn's answer, as the renderer sends it: `firstRunEnded`.
    fn answer(&mut self) {
        self.parts
            .handler
            .handle(
                "motivation.acknowledge",
                serde_json::json!({"event": "firstRunEnded", "level": null}),
            )
            .expect("motivation.acknowledge is answerable");
    }

    fn pump(&mut self) {
        self.parts.handler.pump();
    }

    fn advance(&self, secs: i64) {
        self.parts.clock.advance(secs);
    }

    fn now(&self) -> i64 {
        codotheca_core::clock::Clock::now_unix(&*self.parts.clock)
    }

    /// A shown project on one copy, created at `created_at`; `removed` removes that copy, so the
    /// project is not current.
    fn project(&self, name: &str, created_at: i64, removed: bool) -> i64 {
        let guard = self.index.lock().unwrap();
        let conn = guard.conn();
        conn.execute(
            "INSERT INTO project (name, seed_basename, remote_key, authored_by_user,
                                  is_reference, last_touched_at, created_at, updated_at)
             VALUES (?1, ?1, ?1, 1, 0, ?2, ?2, ?2)",
            rusqlite::params![name, created_at],
        )
        .unwrap();
        let id = conn.last_insert_rowid();
        let path = format!("/srv/{name}");
        conn.execute(
            "INSERT INTO location (project_id, kind, path_bytes, path_key, path_display,
                                   store_key, presence, repo_kind, removed_at)
             VALUES (?1, 'linux', ?2, ?2, ?3, 'store', 'present', 'worktree', ?4)",
            rusqlite::params![id, path.as_bytes(), path, removed.then_some(created_at)],
        )
        .unwrap();
        drop(guard);
        id
    }

    fn core_snapshots(&self) -> Vec<serde_json::Value> {
        self.parts
            .events
            .with(|p| p.queued(Topic::Core))
            .into_iter()
            .filter_map(|q| match q {
                Queued::Snapshot { data, .. } => Some(data),
                Queued::Delta { .. } => None,
            })
            .collect()
    }
}

/// §43.10, path 1 after `SKIP AHEAD`: the turn is answered while the walk still runs, and the
/// walk's tail finds a project after the answer. A stamp at the answer would mark it new.
#[test]
fn ac_p4_43_15_skip_ahead_stamps_at_the_walk_end_not_the_answer() {
    let mut r = rig();
    r.start_scan();
    r.advance(10);
    r.answer();
    assert_eq!(
        r.stamp(),
        None,
        "the answer does not stamp while the walk runs"
    );
    r.pump();
    assert_eq!(r.stamp(), None, "the walk is still running");

    r.advance(10);
    let created_at = r.now();
    r.project("tail", created_at, false);

    r.advance(10);
    r.held.release(HeldOutcome::Finished).unwrap();
    r.pump();
    let stamp = r.stamp().expect("the walk's end stamps");
    assert!(
        stamp >= created_at,
        "the tail's project ({created_at}) is not after the stamp ({stamp}), so it reads no NEW"
    );
}

/// Path 1 with no walk in progress: the answer is the end.
#[test]
fn an_answer_with_no_walk_running_stamps_at_once() {
    let mut r = rig();
    r.parts.events.with(|p| p.subscribe(Topic::Core));
    r.answer();
    assert_eq!(r.stamp(), Some(NOW));
    let snapshots = r.core_snapshots();
    assert_eq!(
        snapshots.len(),
        1,
        "the answer's stamp re-publishes: {snapshots:?}"
    );
    assert_eq!(snapshots[0]["firstRunCompletedAt"], NOW);
    r.advance(10);
    r.answer();
    assert_eq!(
        r.stamp(),
        Some(NOW),
        "write-once: a second answer stamps nothing"
    );
}

/// Path 2: a core that starts with a live project and no stamp stamps at the end of the first
/// run to end in its lifetime, and not before. With no current project it never arms.
#[test]
fn ac_p4_43_15_restart() {
    let mut r = rig();
    r.project("shelved", NOW - 100, false);
    let summary = run_startup(&mut r.parts.handler);
    assert_eq!(
        summary.first_run,
        Some(FirstRunArm::AwaitRunStart { after: 0 })
    );
    r.pump();
    assert_eq!(r.stamp(), None, "no run has ended yet");
    r.start_scan();
    r.pump();
    r.advance(5);
    r.pump();
    assert_eq!(r.stamp(), None, "the run is still live");
    r.advance(5);
    r.held.release(HeldOutcome::Finished).unwrap();
    r.pump();
    assert_eq!(r.stamp(), Some(NOW + 10), "the first run's end stamps");

    // No project at all, and a project whose only copy was removed: neither is a shelf.
    for removed in [None, Some(true)] {
        let mut empty = rig();
        if let Some(removed) = removed {
            empty.project("gone", NOW - 100, removed);
        }
        let idle = run_startup(&mut empty.parts.handler);
        assert_eq!(idle.first_run, Some(FirstRunArm::Idle), "{removed:?}");
        empty.start_scan();
        empty.pump();
        empty.held.release(HeldOutcome::Finished).unwrap();
        empty.pump();
        assert_eq!(
            empty.stamp(),
            None,
            "an empty library stamps nothing: {removed:?}"
        );
    }
}

/// Path 2 does not miss a run that starts and ends between two loop steps (a walk over an empty
/// root set ends inside its own request): the first run to end after the arming stamps, live or
/// not when the loop looks. A failed one in the same gap ends nothing and is stepped over.
#[test]
fn path_two_stamps_a_run_that_ended_before_the_loop_saw_it_live() {
    let mut r = rig();
    r.project("shelved", NOW - 100, false);
    run_startup(&mut r.parts.handler);
    r.pump();

    r.start_scan();
    r.held.release(HeldOutcome::Failed).unwrap();
    r.advance(5);
    r.pump();
    assert_eq!(
        r.stamp(),
        None,
        "a failed walk ends nothing, seen live or not"
    );

    r.advance(5);
    r.start_scan();
    r.held.release(HeldOutcome::Finished).unwrap();
    r.advance(5);
    r.pump();
    assert_eq!(
        r.stamp(),
        Some(NOW + 15),
        "the walk ended before the loop saw it; its end still stamps, at that loop step"
    );
}

/// Deviation 6: a failed run writes no `ended_at` and ends nothing; the next run to end — here
/// cancelled — stamps.
#[test]
fn a_failed_walk_does_not_stamp_and_the_next_cancelled_one_does() {
    let mut r = rig();
    r.start_scan();
    r.answer();
    assert_eq!(r.stamp(), None);
    r.held.release(HeldOutcome::Failed).unwrap();
    r.pump();
    assert_eq!(r.stamp(), None, "a failed walk is not the end of first run");

    r.advance(30);
    r.start_scan();
    r.pump();
    r.held.release(HeldOutcome::Cancelled).unwrap();
    r.pump();
    assert_eq!(r.stamp(), Some(NOW + 30), "a cancelled walk ended");
}

/// The stamp re-publishes `core/snapshot`, so an open window reads it without a restart.
#[test]
fn the_stamp_republishes_the_core_snapshot() {
    let mut r = rig();
    r.parts.events.with(|p| p.subscribe(Topic::Core));
    r.start_scan();
    r.answer();
    r.pump();
    assert!(
        r.core_snapshots().is_empty(),
        "nothing to re-publish before the stamp"
    );

    r.held.release(HeldOutcome::Finished).unwrap();
    r.pump();
    let stamp = r.stamp().expect("stamped");
    let snapshots = r.core_snapshots();
    assert_eq!(snapshots.len(), 1, "{snapshots:?}");
    assert_eq!(snapshots[0]["firstRunCompletedAt"], stamp);
}

/// §37.8's row: the residency answer is not the end of first run. An `autostart` patch — `true`
/// or `false`, first or later — leaves the stamp NULL.
#[test]
fn ac_p4_43_15_an_autostart_patch_no_longer_stamps() {
    let mut r = rig();
    for on in [false, true, false] {
        r.parts
            .handler
            .handle(
                "settings.set",
                serde_json::json!({"patch": {"autostart": on}}),
            )
            .expect("settings.set is answerable");
        r.pump();
        assert_eq!(r.stamp(), None, "autostart {on} is not an ending");
    }
}
