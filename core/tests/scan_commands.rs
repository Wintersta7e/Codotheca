//! Compiled only under `testkit`: these link `codotheca_core::testing`, which the feature gates.
#![cfg(feature = "testkit")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

//! **R37** — §2.4's three `scan.*` commands. The walk, discovery, presence and the run all
//! existed and nothing exposed any of them to the protocol; `RefusingHandler` compiles and
//! answers everything, which is why eleven tasks of working scanner never produced the
//! observation that nothing could start it.
//!
//! **Deviation from plan 07 Task 12.** The plan gives `ScanCtx` an `index: &Index` and has
//! `scan.status` run its own SQL. It takes a `&dyn ScanStore` instead, for two reasons: the
//! scanner then has exactly one seam onto the database — which is what `ScanStore` is *for*,
//! stated in R1 and R40 — and the production store owns the connection behind a mutex (R39),
//! so a second `&Index` beside it would mean holding that lock across a whole command while the
//! scan worker waits on it.

use codotheca_core::index::Index;
use codotheca_core::protocol::{ScanMode, ScanRunId};
use codotheca_core::scan::commands::{handle_cancel, handle_start, handle_status, scan_status};
use codotheca_core::scan::presence::{ScanRunFinish, ScanRunStart, ScanStore};
use codotheca_core::scan::store::SqliteScanStore;
use codotheca_core::scan::ScanSupervisor;
use codotheca_core::scan::{dispatch_scan_command, dispatch_scan_command_names, ScanCtx};
use codotheca_core::testing::{ScanEventFake, ScanLauncherFake};
use std::sync::{Arc, Mutex};

const NOW: i64 = 1_700_000_000;

struct Harness {
    _dir: tempfile::TempDir,
    store: SqliteScanStore,
    events: ScanEventFake,
    scans: ScanSupervisor,
    launcher: Arc<ScanLauncherFake>,
}

fn harness() -> Harness {
    let dir = tempfile::tempdir().expect("tempdir");
    let index = Arc::new(Mutex::new(Index::open(dir.path()).expect("open")));
    let launcher = Arc::new(ScanLauncherFake::default());
    let scans = ScanSupervisor::new(Arc::clone(&launcher) as Arc<_>);
    Harness {
        _dir: dir,
        store: SqliteScanStore::new(index),
        events: ScanEventFake::default(),
        scans,
        launcher,
    }
}

fn ctx(h: &Harness) -> ScanCtx<'_> {
    ScanCtx {
        store: &h.store,
        events: &h.events,
        scans: &h.scans,
        now: NOW,
    }
}

/// One finished run, left the way `finish_scan_run` leaves it: counters final, `ended_at` set.
fn seed_finished_run(h: &Harness, walked: u64, found: u64) {
    let id = h
        .store
        .begin_scan_run(&ScanRunStart {
            generation: 1,
            started_at: NOW - 60,
            mode: "full",
            roots_json: "[{\"id\":1,\"enabled\":true}]".to_owned(),
        })
        .expect("begin");
    h.store
        .finish_scan_run(
            id,
            &ScanRunFinish {
                ended_at: NOW - 10,
                walked_dirs: walked,
                found_repos: found,
                cancelled: false,
            },
        )
        .expect("finish");
}

#[test]
fn dispatch_declines_a_command_this_module_does_not_own() {
    let h = harness();
    assert_eq!(
        dispatch_scan_command_names(),
        ["scan.start", "scan.cancel", "scan.status"]
    );
    assert!(dispatch_scan_command_names()
        .iter()
        .all(|n| n.starts_with("scan.")));
    // `None` is how plan 21's router learns this module is not the owner and moves on.
    assert!(dispatch_scan_command(&ctx(&h), "projects.list", serde_json::json!({})).is_none());
    assert!(dispatch_scan_command(&ctx(&h), "scan.status", serde_json::json!({})).is_some());
}

/// Two runs in flight would each take a `next_generation`, and the higher would mark every
/// location the other is still walking `missing`. A second click means "I want a scan", and that
/// is already true — refusing would teach the user the button is broken.
#[test]
fn a_second_start_returns_the_live_runs_id_and_launches_nothing() {
    let h = harness();
    let first = handle_start(&ctx(&h), serde_json::json!({ "full": true })).unwrap();
    let second = handle_start(&ctx(&h), serde_json::json!({ "full": true })).unwrap();
    assert_eq!(
        first, second,
        "the second click joins the live run, it does not refuse"
    );
    assert_eq!(
        h.launcher.launches(),
        1,
        "two generations racing would mark a live walk missing"
    );
    assert_eq!(
        h.events.named("scan", "run_started").len(),
        1,
        "one run started, one event"
    );
}

#[test]
fn cancelling_sets_the_token_cooperatively_and_a_stale_id_is_a_no_op_success() {
    let h = harness();
    let started = handle_start(&ctx(&h), serde_json::json!({ "full": false })).unwrap();
    let id: ScanRunId = serde_json::from_value(started).unwrap();
    let live = h.scans.live().expect("a run is live");
    assert!(!live.cancel.is_cancelled());

    handle_cancel(&ctx(&h), serde_json::json!({ "id": id })).unwrap();
    assert!(
        live.cancel.is_cancelled(),
        "§4.8 cancels through the token, not by killing"
    );
    assert!(
        h.scans.live().is_some(),
        "a cancel is a request; the run's own end clears the slot"
    );
    assert!(
        h.events.all().is_empty() || h.events.named("scan", "cancelled").is_empty(),
        "ScanCancelled carries endedAt, which is true only once the run has stopped"
    );

    // The run may have finished between the status poll that drew the control and the click.
    handle_cancel(&ctx(&h), serde_json::json!({ "id": ScanRunId(id.0 + 1) }))
        .expect("cancelling a run that is not live is not a failure");
}

/// The load-bearing property: a caller can tell *no scan has ever run* from *a scan ran and found
/// nothing*. Plan 16's first-run gate is exactly `status.runId === null`, so confusing the two
/// either replays the whole reveal on a configured machine or withholds it on an empty one.
#[test]
fn status_tells_a_library_that_never_scanned_from_one_that_scanned_and_found_nothing() {
    let h = harness();
    let idle = scan_status(&ctx(&h)).unwrap();
    assert_eq!(
        idle.run_id, None,
        "plan 16's first-run gate is exactly `runId === null`"
    );
    assert!(!idle.running);
    assert_eq!(
        idle.problem_count, None,
        "not computed here; problems.list owns that figure"
    );
    assert_eq!(idle.ambiguous_lineage_count, None);

    seed_finished_run(&h, 214_903, 0);
    let after = scan_status(&ctx(&h)).unwrap();
    assert_eq!(
        after.run_id,
        Some(ScanRunId(1)),
        "a scan happened, and found nothing"
    );
    assert_eq!(after.found_repos, 0);
    assert_eq!(after.walked_dirs, 214_903);
    assert_eq!(after.ended_at, Some(NOW - 10));
    assert_eq!(after.mode, Some(ScanMode::Full));
    assert!(!after.running);
    assert_ne!(
        serde_json::to_value(&idle).unwrap(),
        serde_json::to_value(&after).unwrap(),
        "an empty scan and an absent scan must not serialise the same"
    );
}

#[test]
fn a_live_run_reports_its_own_counters_and_never_the_rows_zeroes() {
    let h = harness();
    handle_start(&ctx(&h), serde_json::json!({ "full": true })).unwrap();
    let live = h.scans.live().expect("live");
    live.progress.observe(214_903, 147);

    let status = scan_status(&ctx(&h)).unwrap();
    assert!(status.running);
    assert_eq!(status.mode, Some(ScanMode::Full));
    assert_eq!(status.generation, Some(1));
    assert_eq!(status.started_at, Some(NOW));
    assert_eq!(status.ended_at, None);
    // The row's counters are written by finish_scan_run; reading them mid-run would report a
    // walk of 214,903 directories as a walk of zero.
    assert_eq!(status.walked_dirs, 214_903);
    assert_eq!(status.found_repos, 147);
    assert!(!status.cancelled);

    h.scans.cancel(status.run_id.unwrap());
    let cancelling = scan_status(&ctx(&h)).unwrap();
    assert!(cancelling.running, "still winding down");
    assert!(
        cancelling.cancelled,
        "a cancel was asked for, and that is true now"
    );
}

#[test]
fn the_status_handler_returns_the_same_document_the_typed_half_builds() {
    let h = harness();
    seed_finished_run(&h, 10, 3);
    let typed = serde_json::to_value(scan_status(&ctx(&h)).unwrap()).unwrap();
    assert_eq!(handle_status(&ctx(&h)).unwrap(), typed);
}

/// §2.4: the renderer may never originate a filesystem path, so `scan.start` takes exactly one
/// argument and the roots come from `scan_root`.
#[test]
fn scan_start_accepts_only_the_full_flag() {
    let h = harness();
    assert!(handle_start(&ctx(&h), serde_json::json!({ "full": true })).is_ok());
    let h = harness();
    let refused = handle_start(
        &ctx(&h),
        serde_json::json!({ "full": true, "root": "/etc" }),
    );
    assert!(
        refused.is_err(),
        "an unknown argument is a protocol error, not a path the core accepts"
    );
    let h = harness();
    assert!(handle_start(&ctx(&h), serde_json::json!({})).is_err());
}

/// A run that has stopped clears the slot on its own, and `scan.status` then reads the row.
#[test]
fn a_finished_run_stops_being_live_and_the_row_becomes_the_answer() {
    let h = harness();
    handle_start(&ctx(&h), serde_json::json!({ "full": true })).unwrap();
    let live = h.scans.live().expect("live");
    assert!(scan_status(&ctx(&h)).unwrap().running);

    live.progress.finish();
    assert!(
        h.scans.live().is_none(),
        "the run's own completion clears it"
    );
    let idle = scan_status(&ctx(&h)).unwrap();
    assert!(!idle.running);
    assert_eq!(
        idle.run_id, None,
        "the fake launcher wrote no scan_run row, so there is still nothing to report"
    );
}

/// The sequential test above passes even if `start` releases the lock before calling `launch` —
/// one caller never observes the gap. This is the test that gates the lock discipline: eight
/// threads racing on an empty slot must still produce one launch, because two generations racing
/// would have the higher one mark the lower's live walk `missing`.
#[test]
fn eight_concurrent_starts_launch_exactly_one_run() {
    let h = harness();
    std::thread::scope(|scope| {
        for _ in 0..8 {
            scope.spawn(|| {
                handle_start(&ctx(&h), serde_json::json!({ "full": true })).unwrap();
            });
        }
    });
    assert_eq!(h.launcher.launches(), 1);
    assert_eq!(h.events.named("scan", "run_started").len(), 1);
}
