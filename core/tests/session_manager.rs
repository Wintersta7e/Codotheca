#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! Acceptance criterion 11, driven end to end against the fakes.

use std::sync::{Arc, Mutex};

use codotheca_core::git::{RepoHandle, StoreKey};
use codotheca_core::index::Index;
use codotheca_core::mount::StoreClass;
use codotheca_core::proto::EventSink;
use codotheca_core::protocol::{CloseReason, LocationId, ProjectId, SessionId, TargetId};
use codotheca_core::session::activity::FakeIgnoreCheck;
use codotheca_core::session::manager::{LaunchedSession, SessionManager};
use codotheca_core::session::store;
use codotheca_core::session::watch::{ActivityBatch, ActivitySource, FakeActivitySource};
use codotheca_core::session::SessionError;
use codotheca_core::testing::{FakeClock, TempIndex};

const T0: i64 = 1_700_000_000;
const TICK: i64 = 15;

#[derive(Debug, Default)]
struct RecordingSink {
    events: Mutex<Vec<(String, String, serde_json::Value)>>,
}

impl EventSink for RecordingSink {
    fn emit(&self, topic: &str, event: &str, payload: serde_json::Value) {
        self.events
            .lock()
            .unwrap()
            .push((topic.to_owned(), event.to_owned(), payload));
    }
}

impl RecordingSink {
    fn of(&self, topic: &str, event: &str) -> Vec<serde_json::Value> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter(|(t, e, _)| t == topic && e == event)
            .map(|(_, _, payload)| payload.clone())
            .collect()
    }
}

/// The manager takes `Box<dyn ActivitySource>`, so the test keeps a handle to the same buffer
/// in order to stand in for the OS.
#[derive(Debug, Clone, Default)]
struct SharedActivity(Arc<Mutex<FakeActivitySource>>);

impl SharedActivity {
    fn push(&self, session: SessionId, rel: &str) {
        self.0.lock().unwrap().push(session, rel);
    }
}

impl ActivitySource for SharedActivity {
    fn watch(&mut self, session: SessionId, root: &std::path::Path) -> Result<(), SessionError> {
        self.0.lock().unwrap().watch(session, root)
    }
    fn unwatch(&mut self, session: SessionId) {
        self.0.lock().unwrap().unwatch(session);
    }
    fn drain(&mut self) -> Vec<ActivityBatch> {
        self.0.lock().unwrap().drain()
    }
}

struct Harness {
    index: TempIndex,
    manager: SessionManager,
    clock: Arc<FakeClock>,
    events: Arc<RecordingSink>,
    activity: SharedActivity,
    alpha: ProjectId,
    beta: ProjectId,
    loc: LocationId,
    other_loc: LocationId,
    target: TargetId,
}

fn target(index: &TempIndex, project: ProjectId, location: LocationId) -> TargetId {
    index
        .index()
        .conn()
        .execute(
            "INSERT INTO launch_target (project_id, location_id, kind, name, exec_bytes,
                                        sort_index)
             VALUES (?1, ?2, 'editor', 'an editor', X'2F65', 0)",
            rusqlite::params![project.0, location.0],
        )
        .unwrap();
    TargetId(index.index().conn().last_insert_rowid())
}

fn harness() -> Harness {
    let index = TempIndex::new();
    let alpha = index.insert_project();
    let beta = index.insert_project();
    let loc = index.insert_location(alpha, "/w/alpha");
    let other_loc = index.insert_location(beta, "/w/beta");
    let target = target(&index, alpha, loc);

    let clock = Arc::new(FakeClock::new(T0));
    let events = Arc::new(RecordingSink::default());
    let activity = SharedActivity::default();
    // A build directory is ignored; everything else is in scope.
    let ignore = Arc::new(FakeIgnoreCheck::new(&["dist/"]));
    let manager = SessionManager::new(
        Arc::clone(&clock) as Arc<dyn codotheca_core::clock::Clock>,
        Arc::clone(&events) as Arc<dyn EventSink>,
        Box::new(activity.clone()),
        ignore,
    );
    Harness {
        index,
        manager,
        clock,
        events,
        activity,
        alpha,
        beta,
        loc,
        other_loc,
        target,
    }
}

impl Harness {
    fn launch(
        &mut self,
        project: ProjectId,
        location: LocationId,
        waiter: Option<std::thread::JoinHandle<Option<i32>>>,
    ) -> SessionId {
        let launched = LaunchedSession {
            project_id: project,
            location_id: Some(location),
            target_id: Some(self.target),
            repo: RepoHandle::bare(
                std::path::Path::new("/w/repo"),
                StoreKey::new("s"),
                StoreClass::Local,
            ),
            waiter,
        };
        self.manager
            .launch(self.index.index_mut(), launched)
            .unwrap()
    }

    fn tick(&mut self) {
        self.manager.tick(self.index.index_mut()).unwrap();
    }

    /// Let `count` ticks of wall time pass, pumping the manager on each.
    fn ticks(&mut self, count: usize) {
        for _ in 0..count {
            self.clock.advance(TICK);
            self.tick();
        }
    }

    fn conn(&self) -> &rusqlite::Connection {
        self.index.index().conn()
    }

    fn credited(&self, session: SessionId) -> i64 {
        store::credited_seconds(self.conn(), session).unwrap()
    }

    fn close_reason(&self, session: SessionId) -> Option<CloseReason> {
        store::session_ref(self.conn(), session)
            .unwrap()
            .close_reason
    }
}

#[test]
fn a_wait_mode_target_ends_the_session_on_process_exit() {
    // Criterion 11, clause 1.
    let mut h = harness();
    let child = std::thread::spawn(|| Some(0));
    // The manager detects exit through `JoinHandle::is_finished`, so the thread must actually
    // have run before any tick can observe it. The loop below advances a fake clock and blocks
    // on nothing, so on a two-core runner it can finish all forty iterations before this thread
    // is ever scheduled — which failed on CI while passing on every developer machine.
    while !child.is_finished() {
        std::thread::yield_now();
    }
    let s = h.launch(h.alpha, h.loc, Some(child));
    for _ in 0..40 {
        h.clock.advance(TICK);
        h.tick();
        if h.manager.live().is_empty() {
            break;
        }
    }
    assert!(h.manager.live().is_empty(), "the child exited");
    let session = store::session_ref(h.conn(), s).unwrap();
    assert_eq!(session.close_reason, Some(CloseReason::ProcessExit));
    assert!(session.ended_at.is_some());
}

#[test]
fn a_tree_that_never_changed_credits_at_most_twenty_minutes() {
    // Criterion 11, clause 2, through the store rather than the machine.
    let mut h = harness();
    let s = h.launch(h.alpha, h.loc, None);
    h.ticks(8 * 60 * 60 / 15);
    assert_eq!(h.credited(s), 1_200);
    assert_eq!(h.close_reason(s), Some(CloseReason::Idle));
}

#[test]
fn browsing_the_shelf_does_not_extend_another_projects_segment() {
    // Criterion 11, clause 3 -- §9's farming hole. Beta's page is focused for an hour; alpha's
    // session must idle out exactly as if nothing were focused.
    let mut h = harness();
    let s = h.launch(h.alpha, h.loc, None);
    for _ in 0..240 {
        h.manager.set_focus(Some(h.beta));
        h.clock.advance(TICK);
        h.tick();
    }
    assert_eq!(h.credited(s), 1_200);
}

#[test]
fn focusing_this_projects_own_view_does_extend_its_segment() {
    let mut h = harness();
    let s = h.launch(h.alpha, h.loc, None);
    for _ in 0..80 {
        h.manager.set_focus(Some(h.alpha));
        h.clock.advance(TICK);
        h.tick();
    }
    h.manager.set_focus(None);
    h.ticks(80);
    assert_eq!(h.credited(s), 1_200 + 1_200);
}

#[test]
fn a_dead_renderer_holding_focus_stops_extending_within_two_minutes() {
    // set_focus is never called again; the last report goes stale and the segment idles out.
    let mut h = harness();
    let s = h.launch(h.alpha, h.loc, None);
    h.manager.set_focus(Some(h.alpha));
    h.ticks(200);
    assert!(h.credited(s) <= 1_200 + 120, "credited {}", h.credited(s));
}

#[test]
fn a_build_directory_write_does_not_hold_the_session_open() {
    let mut h = harness();
    let s = h.launch(h.alpha, h.loc, None);
    for _ in 0..240 {
        h.activity.push(s, "dist/bundle.js");
        h.clock.advance(TICK);
        h.tick();
    }
    assert_eq!(h.credited(s), 1_200);
}

#[test]
fn a_source_save_does_hold_the_session_open() {
    // The mirror of the test above: the filter must not be inert. A gate that rejects everything
    // passes the build-directory case for the wrong reason.
    let mut h = harness();
    let s = h.launch(h.alpha, h.loc, None);
    for _ in 0..240 {
        h.activity.push(s, "src/main.rs");
        h.clock.advance(TICK);
        h.tick();
    }
    assert!(
        h.credited(s) >= 3_600,
        "an hour of saves credits an hour, not a window; got {}",
        h.credited(s)
    );
    assert_eq!(h.manager.live(), vec![s], "and the session is still open");
}

#[test]
fn a_save_after_an_idle_close_reopens_a_segment_and_the_events_say_so() {
    let mut h = harness();
    let s = h.launch(h.alpha, h.loc, None);
    h.ticks(90);
    h.activity.push(s, "src/main.rs");
    h.clock.advance(TICK);
    h.tick();
    assert_eq!(h.manager.live(), vec![s], "one session, not two");
    let closed = h.events.of("session", "segment_closed");
    assert_eq!(closed.len(), 1);
    assert_eq!(closed.first().unwrap()["creditedSeconds"], 1_200);
    assert_eq!(closed.first().unwrap()["sessionCreditedSeconds"], 1_200);
}

#[test]
fn stop_closes_the_ledger_and_kills_nothing() {
    let mut h = harness();
    let child = std::thread::spawn(|| {
        std::thread::sleep(std::time::Duration::from_secs(30));
        Some(0)
    });
    let s = h.launch(h.alpha, h.loc, Some(child));
    h.clock.advance(900);
    h.manager.stop(h.index.index_mut(), s).unwrap();

    let session = store::session_ref(h.conn(), s).unwrap();
    assert_eq!(session.close_reason, Some(CloseReason::Stop));
    assert_eq!(session.credited_seconds, 900);
    assert_eq!(h.events.of("session", "ended").len(), 1);
    // §7.8: Stop writes nothing to disk and is not a §17 destructive operation. Nothing here
    // may kill the child, so the waiter thread is still running.
}

#[test]
fn stopping_an_already_closed_session_is_not_a_second_close() {
    let mut h = harness();
    let s = h.launch(h.alpha, h.loc, None);
    h.clock.advance(600);
    h.manager.stop(h.index.index_mut(), s).unwrap();
    h.manager.stop(h.index.index_mut(), s).unwrap();
    assert_eq!(h.credited(s), 600);
    assert_eq!(h.events.of("session", "ended").len(), 1);
}

#[test]
fn app_exit_closes_every_live_session_as_app_exit() {
    // §12-15: "An update landing mid-session closes the session with close_reason='app_exit'."
    let mut h = harness();
    let a = h.launch(h.alpha, h.loc, None);
    let b = h.launch(h.beta, h.other_loc, None);
    h.clock.advance(600);
    h.manager.shutdown(h.index.index_mut()).unwrap();
    for s in [a, b] {
        assert_eq!(h.close_reason(s), Some(CloseReason::AppExit));
        assert_eq!(h.credited(s), 600);
    }
    assert!(h.manager.live().is_empty());
}

#[test]
fn a_location_going_offline_ends_the_session_it_was_launched_from() {
    let mut h = harness();
    let s = h.launch(h.alpha, h.loc, None);
    let other = h.launch(h.beta, h.other_loc, None);
    h.clock.advance(300);
    h.manager
        .location_offline(h.index.index_mut(), h.loc)
        .unwrap();
    assert_eq!(
        h.manager.live(),
        vec![other],
        "only that location's session"
    );
    // §1.6's enum carries no `offline` reason and the credit is identical either way.
    assert_eq!(h.close_reason(s), Some(CloseReason::Idle));
}

#[test]
fn opening_a_session_publishes_started_with_the_generated_ref() {
    let mut h = harness();
    h.launch(h.alpha, h.loc, None);
    let started = h.events.of("session", "started");
    assert_eq!(started.len(), 1);
    assert_eq!(started.first().unwrap()["session"]["creditedSeconds"], 0);
    assert_eq!(
        started.first().unwrap()["session"]["endedAt"],
        serde_json::Value::Null
    );
}

#[test]
fn closing_a_session_reports_the_project_whose_condition_moved() {
    // The command layer publishes `projects/condition_changed`; this module only accumulates.
    //
    // Never render unknown as zero, and its mirror here: a project no job has succeeded for has
    // an *uncomputed* condition, and moving from uncomputed to uncomputed is not a change. The
    // first half of this test would pass against a manager that reported every close.
    let mut h = harness();
    let s = h.launch(h.alpha, h.loc, None);
    h.clock.advance(600);
    h.manager.stop(h.index.index_mut(), s).unwrap();
    assert!(
        h.manager.take_condition_changes().is_empty(),
        "an uncomputed condition did not move"
    );

    h.index
        .mark_job_done(h.alpha, codotheca_core::jobs::JobKind::J4History);
    let s = h.launch(h.alpha, h.loc, None);
    h.clock.advance(600);
    h.manager.stop(h.index.index_mut(), s).unwrap();
    assert_eq!(h.manager.take_condition_changes(), vec![h.alpha]);
    assert!(
        h.manager.take_condition_changes().is_empty(),
        "taking empties the list"
    );
}

#[test]
fn two_live_sessions_keep_separate_ledgers() {
    // Alpha is worked on throughout; beta is opened and left. Beta must not ride on alpha's
    // activity, which is the same routing bug as the farming hole seen from the other side.
    let mut h = harness();
    let a = h.launch(h.alpha, h.loc, None);
    let b = h.launch(h.beta, h.other_loc, None);
    for _ in 0..240 {
        h.activity.push(a, "src/main.rs");
        h.clock.advance(TICK);
        h.tick();
    }
    assert!(h.credited(a) >= 3_600, "alpha was worked on");
    assert_eq!(h.credited(b), 1_200, "beta credits only its own window");
}

#[test]
fn a_session_with_no_watcher_still_credits_its_window() {
    // `launch` does not fail when the watch cannot be established. Such a session credits by
    // focus and by the 20-minute bound, which is honest; refusing to launch would not be.
    #[derive(Debug, Default)]
    struct RefusingWatcher;
    impl ActivitySource for RefusingWatcher {
        fn watch(
            &mut self,
            _session: SessionId,
            _root: &std::path::Path,
        ) -> Result<(), SessionError> {
            Err(SessionError::Watch("no watcher".to_owned()))
        }
        fn unwatch(&mut self, _session: SessionId) {}
        fn drain(&mut self) -> Vec<ActivityBatch> {
            Vec::new()
        }
    }

    let mut h = harness();
    h.manager = SessionManager::new(
        Arc::clone(&h.clock) as Arc<dyn codotheca_core::clock::Clock>,
        Arc::clone(&h.events) as Arc<dyn EventSink>,
        Box::new(RefusingWatcher),
        Arc::new(FakeIgnoreCheck::new(&[])),
    );

    let s = h.launch(h.alpha, h.loc, None);
    assert_eq!(h.events.of("session", "started").len(), 1, "it did launch");
    h.ticks(200);
    assert_eq!(h.credited(s), 1_200);
}

#[test]
fn the_index_is_the_only_writer_and_the_manager_holds_no_connection() {
    // §1.10: the manager takes `&mut Index` from its caller. This is a shape assertion -- it
    // compiles only while `tick` accepts a borrowed index rather than owning one.
    fn takes_borrowed(manager: &mut SessionManager, index: &mut Index) {
        manager.tick(index).unwrap();
    }
    let mut h = harness();
    h.launch(h.alpha, h.loc, None);
    let Harness {
        ref mut manager,
        ref mut index,
        ..
    } = h;
    takes_borrowed(manager, index.index_mut());
}
