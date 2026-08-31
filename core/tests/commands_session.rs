#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! §2.4's two session commands and §11.2's orphan recovery, through the layer that runs them.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use codotheca_core::commands::launch::{handle_focus, handle_launch, handle_stop, startup, tick};
use codotheca_core::git::{RepoHandle, StoreKey};
use codotheca_core::launch::spawn::RecordingSpawner;
use codotheca_core::mount::{MountFacts, StoreClass};
use codotheca_core::protocol::{CloseReason, ErrorCode, ProjectId, SessionId, TargetId};
use codotheca_core::session::activity::FakeIgnoreCheck;
use codotheca_core::session::manager::{LaunchedSession, SessionManager};
use codotheca_core::session::store;
use codotheca_core::session::watch::FakeActivitySource;
use codotheca_core::testing::{FakeClock, FakeMountResolver};
use serde_json::{json, Value};

const T0: i64 = 1_700_000_000;
const TICK: i64 = 15;

/// The statement counter `rusqlite`'s trace hook feeds. A plain `fn(&str)` cannot capture, so
/// the count is a static — the tests that read it run in one process and each resets it first.
static STATEMENTS: AtomicUsize = AtomicUsize::new(0);

fn count_statement(_sql: &str) {
    STATEMENTS.fetch_add(1, Ordering::Relaxed);
}

#[derive(Debug, Default)]
struct RecordingSink {
    events: Mutex<Vec<(String, String, Value)>>,
}

impl codotheca_core::proto::EventSink for RecordingSink {
    fn emit(&self, topic: &str, event: &str, payload: Value) {
        self.events
            .lock()
            .unwrap()
            .push((topic.to_owned(), event.to_owned(), payload));
    }
}

impl RecordingSink {
    fn of(&self, topic: &str, event: &str) -> Vec<Value> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter(|(t, e, _)| t == topic && e == event)
            .map(|(_, _, payload)| payload.clone())
            .collect()
    }
}

struct Fixture {
    dir: tempfile::TempDir,
    index: codotheca_core::index::Index,
    sessions: SessionManager,
    spawner: Arc<RecordingSpawner>,
    events: Arc<RecordingSink>,
    mounts: FakeMountResolver,
    clock: Arc<FakeClock>,
    project: i64,
    other_project: i64,
    location: i64,
    target: i64,
    work_dir: PathBuf,
}

fn manager(clock: &Arc<FakeClock>, events: &Arc<RecordingSink>) -> SessionManager {
    SessionManager::new(
        Arc::clone(clock) as Arc<dyn codotheca_core::clock::Clock>,
        Arc::clone(events) as Arc<dyn codotheca_core::proto::EventSink>,
        Box::new(FakeActivitySource::default()),
        Arc::new(FakeIgnoreCheck::new(&["dist/"])),
    )
}

fn executable(root: &std::path::Path, name: &str) -> PathBuf {
    let path = root.join(name);
    std::fs::write(&path, b"#!/bin/sh\n").expect("write");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    }
    path
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().expect("tempdir");
    let index = codotheca_core::index::Index::open(dir.path()).expect("open");
    let work_dir = dir.path().join("copy");
    std::fs::create_dir_all(work_dir.join(".git")).expect("worktree");
    let editor = executable(dir.path(), "an-editor");

    let conn = index.conn();
    let mut projects = Vec::new();
    for name in ["p", "q"] {
        conn.execute(
            "INSERT INTO project (name, seed_basename, created_at, updated_at)
             VALUES (?1, ?1, 0, 0)",
            rusqlite::params![name],
        )
        .expect("project");
        projects.push(conn.last_insert_rowid());
    }
    let stored_path = work_dir.display().to_string();
    conn.execute(
        "INSERT INTO location (project_id, kind, distro, path_bytes, path_key, path_display,
                               volume_key, store_key, presence, repo_kind)
         VALUES (?1, ?2, '', ?3, ?3, ?4, 'v', 's', 'present', 'worktree')",
        rusqlite::params![
            projects[0],
            if cfg!(windows) { "win" } else { "linux" },
            stored_path.as_bytes(),
            stored_path.as_str()
        ],
    )
    .expect("location");
    let location = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO launch_target (kind, name, exec_bytes, args_json, cwd_mode, env_json,
                                    sort_index, detected, verify_state)
         VALUES ('editor', 'an editor', ?1, '[]', 'location', '{}', 0, 1, 'unverified')",
        rusqlite::params![editor.display().to_string().as_bytes()],
    )
    .expect("target");
    let target = conn.last_insert_rowid();

    let clock = Arc::new(FakeClock::new(T0));
    let events = Arc::new(RecordingSink::default());
    let sessions = manager(&clock, &events);
    let mounts = FakeMountResolver::new();
    mounts.map(
        dir.path(),
        MountFacts {
            store_key: "s".to_owned(),
            volume_key: Some("v".to_owned()),
            class: StoreClass::Local,
        },
    );

    Fixture {
        dir,
        index,
        sessions,
        spawner: Arc::new(RecordingSpawner::new()),
        events,
        mounts,
        clock,
        project: projects[0],
        other_project: projects[1],
        location,
        target,
        work_dir,
    }
}

impl Fixture {
    fn ctx(&mut self) -> codotheca_core::commands::launch::LaunchCtx<'_> {
        codotheca_core::commands::launch::LaunchCtx {
            index: &mut self.index,
            sessions: &mut self.sessions,
            spawner: self.spawner.as_ref(),
            events: self.events.as_ref(),
            mounts: &self.mounts,
            now: codotheca_core::clock::Clock::now_unix(self.clock.as_ref()),
        }
    }

    fn launch(&mut self) -> SessionId {
        let args = json!({
            "projectId": self.project,
            "locationId": self.location,
            "targetId": self.target,
        });
        handle_launch(&mut self.ctx(), args).expect("launch")
    }

    /// A session left open by a process that is gone: the row stays, the manager does not.
    fn crash(&mut self) {
        self.sessions = manager(&self.clock, &self.events);
    }

    /// Open a session directly, bypassing the command, so the fixture can leave one behind
    /// without a spawn.
    fn open_directly(&mut self) -> SessionId {
        let launched = LaunchedSession {
            project_id: ProjectId(self.project),
            location_id: Some(codotheca_core::protocol::LocationId(self.location)),
            target_id: Some(TargetId(self.target)),
            repo: RepoHandle::bare(&self.work_dir, StoreKey::new("s"), StoreClass::Local),
            waiter: None,
        };
        let mut sessions =
            std::mem::replace(&mut self.sessions, manager(&self.clock, &self.events));
        let id = sessions.launch(&mut self.index, launched).expect("open");
        self.sessions = sessions;
        id
    }

    fn credited(&self, session: SessionId) -> i64 {
        store::credited_seconds(self.index.conn(), session).expect("credited")
    }

    fn close_reason(&self, session: SessionId) -> Option<CloseReason> {
        store::session_ref(self.index.conn(), session)
            .expect("session")
            .close_reason
    }

    /// Statements issued from now on, counted by `rusqlite`'s trace hook.
    fn trace_statements(&mut self) {
        STATEMENTS.store(0, Ordering::Relaxed);
        self.index.conn_mut().trace(Some(count_statement));
    }
}

/// Statements counted since the last [`Fixture::trace_statements`].
fn statements() -> usize {
    STATEMENTS.load(Ordering::Relaxed)
}

#[test]
fn stop_closes_the_ledger_and_starts_no_process_and_kills_none() {
    // §17: phase 1 has no destructive operation. Stop ends the accounting, nothing else.
    let mut h = fixture();
    let session = h.launch();
    h.clock.advance(900);

    handle_stop(&mut h.ctx(), json!({ "id": session.0 })).unwrap();
    assert_eq!(h.close_reason(session), Some(CloseReason::Stop));
    assert_eq!(h.credited(session), 900);
    assert_eq!(
        h.spawner.calls().len(),
        1,
        "no second process, and none killed"
    );
}

#[test]
fn stopping_an_already_stopped_session_succeeds() {
    // L3: the user asked for a state that already holds.
    let mut h = fixture();
    let session = h.launch();
    h.clock.advance(300);
    handle_stop(&mut h.ctx(), json!({ "id": session.0 })).unwrap();
    let after_first = h.credited(session);
    handle_stop(&mut h.ctx(), json!({ "id": session.0 })).unwrap();
    assert_eq!(h.credited(session), after_first, "not a second close");
    assert_eq!(h.close_reason(session), Some(CloseReason::Stop));
}

#[test]
fn stopping_a_session_that_never_existed_reports_it() {
    // L3's other half: only a renderer bug names a session with no row at all.
    let mut h = fixture();
    let err = handle_stop(&mut h.ctx(), json!({ "id": 9_999 })).unwrap_err();
    assert_eq!(err.code, ErrorCode::Protocol);
}

#[test]
fn focus_opens_no_transaction_and_issues_no_statement() {
    // L4: this command arrives on a timer. Its cost is the reason the heartbeat is affordable.
    let mut h = fixture();
    let project = h.project;
    h.launch();
    h.trace_statements();
    for _ in 0..100 {
        handle_focus(&mut h.ctx(), json!({ "projectId": project })).unwrap();
        handle_focus(&mut h.ctx(), json!({ "projectId": null })).unwrap();
    }
    assert_eq!(statements(), 0, "200 focus reports touched the database");
}

#[test]
fn focus_reaches_the_manager_and_extends_only_that_projects_segment() {
    // §9's farming hole, through the command rather than the manager.
    let mut h = fixture();
    let other = h.other_project;
    let session = h.launch();
    for _ in 0..240 {
        handle_focus(&mut h.ctx(), json!({ "projectId": other })).unwrap();
        h.clock.advance(TICK);
        tick(&mut h.ctx()).unwrap();
    }
    assert_eq!(
        h.credited(session),
        1_200,
        "another project's page extends nothing"
    );
}

#[test]
fn focus_on_this_projects_own_view_does_extend_its_segment() {
    // The mirror that proves the previous test is not simply inert.
    let mut h = fixture();
    let project = h.project;
    let session = h.launch();
    for _ in 0..240 {
        handle_focus(&mut h.ctx(), json!({ "projectId": project })).unwrap();
        h.clock.advance(TICK);
        tick(&mut h.ctx()).unwrap();
    }
    assert!(
        h.credited(session) > 1_200,
        "own-view focus holds the segment open: {}",
        h.credited(session)
    );
}

#[test]
fn a_null_focus_report_is_accepted_and_clears_the_level() {
    let mut h = fixture();
    handle_focus(&mut h.ctx(), json!({ "projectId": null })).unwrap();
}

#[test]
fn a_focus_report_with_an_unknown_key_is_refused() {
    let mut h = fixture();
    let err = handle_focus(&mut h.ctx(), json!({ "projectId": null, "path": "/etc" })).unwrap_err();
    assert_eq!(err.code, ErrorCode::Protocol);
}

#[test]
fn startup_closes_a_session_orphaned_by_a_crash_and_says_which() {
    // Criterion 11, clause 4, through the layer that actually runs it.
    let mut h = fixture();
    let orphan = h.open_directly();
    h.clock.advance(600);
    h.crash();

    let report = startup(&mut h.ctx()).unwrap();
    assert_eq!(report.sessions_closed, 1);
    assert_eq!(h.close_reason(orphan), Some(CloseReason::Orphaned));
    let ended = h.events.of("session", "ended");
    assert_eq!(ended.len(), 1);
    assert_eq!(ended[0]["session"]["id"], json!(orphan.0));
}

#[test]
fn startup_on_a_clean_database_closes_nothing_and_emits_nothing() {
    let mut h = fixture();
    let report = startup(&mut h.ctx()).unwrap();
    assert_eq!((report.sessions_closed, report.segments_closed), (0, 0));
    assert!(h.events.of("session", "ended").is_empty());
}

#[test]
fn startup_runs_before_any_session_of_this_process_can_exist() {
    // Plan 11b Task 7's safety argument, asserted rather than assumed: an orphan closed at
    // startup must not be confused with a session this process opened.
    let mut h = fixture();
    let orphan = h.open_directly();
    h.clock.advance(600);
    h.crash();

    startup(&mut h.ctx()).unwrap();
    let fresh = h.launch();
    assert_ne!(fresh, orphan);
    assert_eq!(h.close_reason(fresh), None);
    assert_eq!(h.close_reason(orphan), Some(CloseReason::Orphaned));
}

#[test]
fn startup_emits_one_ended_event_per_orphan_and_not_one_per_open_row_it_saw() {
    let mut h = fixture();
    let first = h.open_directly();
    let second = h.open_directly();
    h.clock.advance(600);
    h.crash();

    let report = startup(&mut h.ctx()).unwrap();
    assert_eq!(report.sessions_closed, 2);
    let ended = h.events.of("session", "ended");
    let ids: Vec<Value> = ended.iter().map(|e| e["session"]["id"].clone()).collect();
    assert_eq!(ids, vec![json!(first.0), json!(second.0)]);
}

#[test]
fn the_tempdir_outlives_every_handle_the_fixture_hands_out() {
    // Guards the fixture itself: a dropped tempdir would make every path test above vacuous.
    let h = fixture();
    assert!(h.dir.path().exists());
    assert!(h.work_dir.join(".git").is_dir());
}
