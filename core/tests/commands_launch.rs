#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! §4bis.5 through the command surface: resolved and spawned core-side, argv only.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use codotheca_core::commands::launch::{dispatch_launch_command, handle_launch, LaunchCtx};
use codotheca_core::launch::catalogue::CATALOGUE;
use codotheca_core::launch::spawn::RecordingSpawner;
use codotheca_core::mount::{MountFacts, StoreClass};
use codotheca_core::protocol::{ErrorCode, LocationId, ProjectId, SessionId, TargetId};
use codotheca_core::session::activity::FakeIgnoreCheck;
use codotheca_core::session::manager::SessionManager;
use codotheca_core::session::store;
use codotheca_core::session::watch::FakeActivitySource;
use codotheca_core::testing::{FakeClock, FakeMountResolver};
use serde_json::{json, Value};

const T0: i64 = 1_700_000_000;

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
    location: i64,
    target: i64,
    editor: PathBuf,
    work_dir: PathBuf,
}

/// A worktree `RepoHandle::resolve` accepts: a directory with a `.git` directory in it.
fn worktree(root: &std::path::Path, name: &str) -> PathBuf {
    let dir = root.join(name);
    std::fs::create_dir_all(dir.join(".git")).expect("worktree");
    dir
}

/// An executable file: what `verify_path` calls `ok`.
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

/// The first catalogued tool that declares a `{path}`-substituted argv, chosen at runtime so
/// no product name is written into this file.
fn a_catalogued_editor() -> &'static codotheca_core::launch::catalogue::CatalogueEntry {
    CATALOGUE
        .iter()
        .find(|e| e.wsl_args.is_some() && e.args.contains(&"{path}"))
        .expect("a catalogued tool with a WSL form")
}

fn fixture_with(kind: &str, distro: &str) -> Fixture {
    let dir = tempfile::tempdir().expect("tempdir");
    let index = codotheca_core::index::Index::open(dir.path()).expect("open");
    let work_dir = worktree(dir.path(), "copy");
    let entry = a_catalogued_editor();
    // The stored executable's stem must be a catalogue binary, or `argv::build` finds no
    // WSL form for a distro site. `verify` needs the file to exist and to be executable.
    let editor = executable(dir.path(), entry.binaries[0]);

    let conn = index.conn();
    conn.execute(
        "INSERT INTO project (name, seed_basename, created_at, updated_at) VALUES ('p','p',0,0)",
        [],
    )
    .expect("project");
    let project = conn.last_insert_rowid();
    let stored_path = work_dir.display().to_string();
    conn.execute(
        "INSERT INTO location (project_id, kind, distro, path_bytes, path_key, path_display,
                               volume_key, store_key, presence, repo_kind)
         VALUES (?1, ?2, ?3, ?4, ?4, ?5, 'v', 's', 'present', 'worktree')",
        rusqlite::params![
            project,
            kind,
            distro,
            stored_path.as_bytes(),
            stored_path.as_str()
        ],
    )
    .expect("location");
    let location = conn.last_insert_rowid();
    let args_json = serde_json::to_string(entry.args).expect("args");
    conn.execute(
        "INSERT INTO launch_target (kind, name, exec_bytes, args_json, cwd_mode, env_json,
                                    sort_index, detected, verify_state)
         VALUES ('editor', 'an editor', ?1, ?2, 'location', '{}', 0, 1, 'unverified')",
        rusqlite::params![editor.display().to_string().as_bytes(), args_json],
    )
    .expect("target");
    let target = conn.last_insert_rowid();

    let clock = Arc::new(FakeClock::new(T0));
    let events = Arc::new(RecordingSink::default());
    let sessions = SessionManager::new(
        Arc::clone(&clock) as Arc<dyn codotheca_core::clock::Clock>,
        Arc::clone(&events) as Arc<dyn codotheca_core::proto::EventSink>,
        Box::new(FakeActivitySource::default()),
        Arc::new(FakeIgnoreCheck::new(&["dist/"])),
    );
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
        project,
        location,
        target,
        editor,
        work_dir,
    }
}

fn fixture() -> Fixture {
    fixture_with(if cfg!(windows) { "win" } else { "linux" }, "")
}

impl Fixture {
    fn ctx(&mut self) -> LaunchCtx<'_> {
        LaunchCtx {
            index: &mut self.index,
            sessions: &mut self.sessions,
            spawner: self.spawner.as_ref(),
            events: self.events.as_ref(),
            mounts: &self.mounts,
            now: codotheca_core::clock::Clock::now_unix(self.clock.as_ref()),
        }
    }

    fn launch_args(&self) -> Value {
        json!({
            "projectId": self.project,
            "locationId": self.location,
            "targetId": self.target,
        })
    }

    fn set_presence(&self, location: i64, presence: &str) {
        self.index
            .conn()
            .execute(
                "UPDATE location SET presence = ?2 WHERE id = ?1",
                rusqlite::params![location, presence],
            )
            .expect("presence");
    }

    fn verify_state_of(&self, target: i64) -> String {
        self.index
            .conn()
            .query_row(
                "SELECT verify_state FROM launch_target WHERE id = ?1",
                rusqlite::params![target],
                |r| r.get(0),
            )
            .expect("verify_state")
    }

    fn table_rows(&self, table: &str) -> i64 {
        self.index
            .conn()
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
            .expect("count")
    }

    /// Every user table's row count, so a test can assert what a launch did *not* touch.
    fn row_counts(&self) -> BTreeMap<String, i64> {
        let mut names = Vec::new();
        {
            let mut stmt = self
                .index
                .conn()
                .prepare(
                    "SELECT name FROM sqlite_master
                      WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
                )
                .expect("tables");
            let rows = stmt
                .query_map([], |r| r.get::<_, String>(0))
                .expect("names");
            for name in rows {
                names.push(name.expect("name"));
            }
        }
        names
            .into_iter()
            .map(|name| {
                let count = self.table_rows(&name);
                (name, count)
            })
            .collect()
    }

    fn new_project_with_location(&self) -> (i64, i64) {
        let conn = self.index.conn();
        conn.execute(
            "INSERT INTO project (name, seed_basename, created_at, updated_at)
             VALUES ('q','q',0,0)",
            [],
        )
        .expect("project");
        let project = conn.last_insert_rowid();
        let other = worktree(self.dir.path(), "other");
        let path = other.display().to_string();
        conn.execute(
            "INSERT INTO location (project_id, kind, distro, path_bytes, path_key, path_display,
                                   volume_key, store_key, presence, repo_kind)
             VALUES (?1, ?2, '', ?3, ?3, ?4, 'v', 's', 'present', 'worktree')",
            rusqlite::params![
                project,
                if cfg!(windows) { "win" } else { "linux" },
                path.as_bytes(),
                path.as_str()
            ],
        )
        .expect("location");
        (project, conn.last_insert_rowid())
    }

    fn merge_project_into(&self, survivor: i64) -> i64 {
        let conn = self.index.conn();
        conn.execute(
            "INSERT INTO project (name, seed_basename, created_at, updated_at, merged_into)
             VALUES ('gone','gone',0,0,?1)",
            rusqlite::params![survivor],
        )
        .expect("tombstone");
        let gone = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO project_redirect (old_project_id, new_project_id, merged_at)
             VALUES (?1, ?2, 0)",
            rusqlite::params![gone, survivor],
        )
        .expect("redirect");
        gone
    }

    fn project_of(&self, session: SessionId) -> i64 {
        store::session_ref(self.index.conn(), session)
            .expect("session")
            .project_id
            .0
    }
}

#[test]
fn a_launch_spawns_the_stored_program_with_the_stored_argv_and_no_shell() {
    let mut h = fixture();
    let args = h.launch_args();
    handle_launch(&mut h.ctx(), args).unwrap();

    let calls = h.spawner.calls();
    assert_eq!(calls.len(), 1);
    let inv = &calls[0];
    assert_eq!(inv.program, h.editor);
    assert_eq!(inv.cwd.as_deref(), Some(h.work_dir.as_path()));
    for shell in ["sh", "bash", "cmd.exe", "powershell.exe", "/bin/sh"] {
        assert!(!inv.program.ends_with(shell), "spawned through {shell}");
    }
    assert!(!inv.argv.iter().any(|a| a == "-c" || a == "/c"));
    assert!(
        inv.argv.iter().any(|a| a == h.work_dir.as_os_str()),
        "the copy's path is the argument, substituted whole"
    );
}

#[cfg(windows)]
#[test]
fn a_wsl_location_reaches_the_spawner_with_its_translated_argv_untouched() {
    // Criterion 8's command half. The translation is plan 11's; this layer must not re-derive,
    // re-encode or normalise the path on the way past. Windows only: a `wsl` site's stored
    // path is a Windows path, and `windows_to_wsl` has nothing to translate on a Linux host.
    let mut h = fixture_with("wsl", "distro-a");
    let expected = codotheca_core::launch::argv::build(
        &codotheca_core::launch::resolve::load_target(h.index.conn(), h.target).unwrap(),
        &codotheca_core::launch::argv::LaunchSite {
            kind: codotheca_core::derive::LocationKind::Wsl,
            distro: "distro-a".to_owned(),
            path_bytes: h.work_dir.display().to_string().into_bytes(),
        },
    )
    .expect("an invocation")
    .argv;

    let args = h.launch_args();
    handle_launch(&mut h.ctx(), args).unwrap();
    let calls = h.spawner.calls();
    assert_eq!(calls[0].argv, expected, "argv passed through verbatim");
    for arg in &calls[0].argv {
        let text = arg.to_string_lossy();
        assert!(
            !text.contains("wsl.localhost") && !text.contains("wsl$"),
            "the UNC form is display only and must never reach an argv: {text}"
        );
    }
}

#[test]
fn a_stale_target_path_is_detected_before_spawn_and_nothing_is_spawned() {
    // Criterion 7, final clause.
    let mut h = fixture();
    std::fs::remove_file(&h.editor).expect("delete the editor binary");

    let args = h.launch_args();
    let err = handle_launch(&mut h.ctx(), args).unwrap_err();
    assert_eq!(err.code, ErrorCode::PathGone);
    assert!(h.spawner.calls().is_empty(), "no process was started");
    assert_eq!(
        h.verify_state_of(h.target),
        "missing",
        "and the row now says so"
    );
    assert!(h.sessions.live().is_empty(), "no session was opened");
}

#[test]
fn an_offline_location_refuses_rather_than_opening_a_path_on_an_unmounted_drive() {
    let mut h = fixture();
    h.set_presence(h.location, "offline");
    let args = h.launch_args();
    let err = handle_launch(&mut h.ctx(), args).unwrap_err();
    assert_eq!(err.code, ErrorCode::StoreOffline);
    assert!(h.spawner.calls().is_empty());
}

#[test]
fn an_unscanned_location_is_not_treated_as_present() {
    // §6: absence of a recent observation is not permission to open a path.
    let mut h = fixture();
    h.set_presence(h.location, "unscanned");
    let args = h.launch_args();
    let err = handle_launch(&mut h.ctx(), args).unwrap_err();
    assert_eq!(err.code, ErrorCode::StoreOffline);
    assert!(h.spawner.calls().is_empty());
}

#[test]
fn a_location_belonging_to_another_project_is_refused() {
    let mut h = fixture();
    let (_other_project, other_location) = h.new_project_with_location();
    let mut args = h.launch_args();
    args["locationId"] = json!(other_location);
    assert_eq!(
        handle_launch(&mut h.ctx(), args).unwrap_err().code,
        ErrorCode::Protocol
    );
    assert!(h.spawner.calls().is_empty());
}

#[test]
fn a_merged_project_id_still_launches_the_survivor() {
    let mut h = fixture();
    let gone = h.merge_project_into(h.project);
    let mut args = h.launch_args();
    args["projectId"] = json!(gone);

    let session = handle_launch(&mut h.ctx(), args).unwrap();
    assert_eq!(h.project_of(session), h.project);
}

#[test]
fn a_fourth_argument_key_is_refused() {
    // §4bis.5: "{projectId, locationId, targetId} and nothing else."
    let mut h = fixture();
    let mut args = h.launch_args();
    args["execPath"] = json!("/bin/sh");
    assert_eq!(
        handle_launch(&mut h.ctx(), args).unwrap_err().code,
        ErrorCode::Protocol
    );
    assert!(h.spawner.calls().is_empty());
}

#[test]
fn a_failed_spawn_opens_no_session_and_leaves_the_ledger_empty() {
    let mut h = fixture();
    h.spawner.fail_next("permission denied");
    let args = h.launch_args();
    assert!(handle_launch(&mut h.ctx(), args).is_err());
    assert!(h.sessions.live().is_empty());
    assert_eq!(h.table_rows("session"), 0);
    assert_eq!(h.table_rows("session_segment"), 0);
}

#[test]
fn a_launch_writes_only_the_two_session_tables_and_the_row_it_verified() {
    // Two ledgers, never merged: playtime counts only launched sessions, and launching one
    // must not touch a git-derived row. `launch_target` is stamped by the before-spawn check,
    // which adds no row.
    let mut h = fixture();
    let before = h.row_counts();
    let args = h.launch_args();
    handle_launch(&mut h.ctx(), args).unwrap();
    let after = h.row_counts();

    for (table, count) in &after {
        let was = before[table];
        if table == "session" || table == "session_segment" {
            assert!(*count > was, "{table} should have grown");
        } else {
            assert_eq!(*count, was, "{table} changed");
        }
    }
}

#[test]
fn opening_a_session_publishes_started_and_no_condition_change() {
    // §5.1's term is *last session end*: opening one moves nothing, so publishing a change
    // here would report a movement that did not happen. `session/started` is 11b's.
    let mut h = fixture();
    let args = h.launch_args();
    handle_launch(&mut h.ctx(), args).unwrap();
    assert_eq!(h.events.of("session", "started").len(), 1);
    assert!(h.events.of("projects", "condition_changed").is_empty());
}

#[test]
fn the_dispatcher_declines_a_command_it_does_not_own() {
    let mut h = fixture();
    assert!(dispatch_launch_command(&mut h.ctx(), "targets.list", json!({})).is_none());
    let args = h.launch_args();
    assert!(dispatch_launch_command(&mut h.ctx(), "projects.launch", args).is_some());
}

#[test]
fn an_unknown_target_is_refused_before_anything_is_spawned() {
    let mut h = fixture();
    let mut args = h.launch_args();
    args["targetId"] = json!(9_999);
    assert_eq!(
        handle_launch(&mut h.ctx(), args).unwrap_err().code,
        ErrorCode::Protocol
    );
    assert!(h.spawner.calls().is_empty());
}

#[test]
fn the_session_records_the_location_and_target_it_was_launched_from() {
    let mut h = fixture();
    let args = h.launch_args();
    let session = handle_launch(&mut h.ctx(), args).unwrap();
    let reference = store::session_ref(h.index.conn(), session).expect("session");
    assert_eq!(reference.project_id, ProjectId(h.project));
    assert_eq!(reference.location_id, LocationId(h.location));
    assert_eq!(reference.target_id, TargetId(h.target));
}
