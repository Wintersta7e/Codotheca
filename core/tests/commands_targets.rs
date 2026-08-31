#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! Criterion 7's command half: the five tiers as `targets.list` returns them.

use codotheca_core::commands::targets::{dispatch_targets_command, handle_list, TargetsCtx};
use codotheca_core::index::Index;
use codotheca_core::protocol::{ErrorCode, ProjectId, TargetTier, VerifyState};
use serde_json::{json, Value};

const SEED: &str = "
INSERT INTO project (id, name, seed_basename, created_at, updated_at) VALUES (1, 'p', 'p', 0, 0);
INSERT INTO location (id, project_id, kind, distro, path_bytes, path_key, path_display,
                      volume_key, store_key, presence, repo_kind)
     VALUES (1, 1, 'linux', '', X'2F61', X'2F61', '/a', 'v', 's', 'present', 'worktree');
";

/// A row's scope, as §4bis.2a's triple.
#[derive(Debug, Clone, Default)]
struct Scope {
    project: Option<i64>,
    location: Option<i64>,
    language: Option<String>,
}

impl Scope {
    fn global() -> Scope {
        Scope::default()
    }
    fn project(id: i64) -> Scope {
        Scope {
            project: Some(id),
            ..Scope::default()
        }
    }
    fn location(id: i64) -> Scope {
        Scope {
            location: Some(id),
            ..Scope::default()
        }
    }
    fn language(name: &str) -> Scope {
        Scope {
            language: Some(name.to_owned()),
            ..Scope::default()
        }
    }
}

/// Records what the command layer publishes, so a test can assert an event it never sends.
#[derive(Debug, Default)]
struct RecordingSink;

impl codotheca_core::proto::EventSink for RecordingSink {
    fn emit(&self, _topic: &str, _event: &str, _payload: Value) {}
}

struct Fixture {
    _dir: tempfile::TempDir,
    index: Index,
    events: RecordingSink,
    project: i64,
    location: i64,
    now: i64,
    next_sort: i64,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().expect("tempdir");
    let index = Index::open(dir.path()).expect("open");
    index.conn().execute_batch(SEED).expect("seed");
    Fixture {
        _dir: dir,
        index,
        events: RecordingSink,
        project: 1,
        location: 1,
        now: 1_000,
        next_sort: 0,
    }
}

impl Fixture {
    fn ctx(&mut self) -> TargetsCtx<'_> {
        TargetsCtx {
            index: &mut self.index,
            events: &self.events,
            now: self.now,
        }
    }

    /// Inserts one `launch_target` row in `scope` and returns its id.
    fn target(&mut self, scope: &Scope, name: &str) -> i64 {
        let sort = self.next_sort;
        self.next_sort += 1;
        self.index
            .conn()
            .execute(
                "INSERT INTO launch_target
                     (project_id, location_id, language, kind, name, exec_bytes, args_json,
                      cwd_mode, env_json, sort_index, detected, verify_state)
                 VALUES (?1, ?2, ?3, 'editor', ?4, X'2F62696E2F65', '[\"{path}\"]', 'location',
                         '{}', ?5, 1, 'ok')",
                rusqlite::params![scope.project, scope.location, scope.language, name, sort],
            )
            .expect("insert target");
        self.index.conn().last_insert_rowid()
    }

    fn set_primary_language(&self, project: i64, language: &str) {
        self.index
            .conn()
            .execute(
                "UPDATE project SET primary_language = ?2 WHERE id = ?1",
                rusqlite::params![project, language],
            )
            .expect("set language");
    }

    fn set_verify_state(&self, target: i64, state: &str) {
        self.index
            .conn()
            .execute(
                "UPDATE launch_target SET verify_state = ?2 WHERE id = ?1",
                rusqlite::params![target, state],
            )
            .expect("set verify_state");
    }
}

fn json_args(project: i64, location: Option<i64>) -> Value {
    json!({ "projectId": project, "locationId": location })
}

#[test]
fn a_project_with_no_language_resolves_at_the_global_tier_not_a_language_row() {
    // §4bis.2a: "an implementation that coalesces the NULL into the language IS NULL predicate,
    // or takes the first language row, is wrong."
    let mut h = fixture();
    let global = h.target(&Scope::global(), "any-editor");
    h.target(&Scope::language("Rust"), "a-rust-ide");
    // project.primary_language stays NULL.

    let project = h.project;
    let list = handle_list(&mut h.ctx(), json_args(project, None)).unwrap();
    let resolved = list.resolved.expect("tier 4 resolves");
    assert_eq!(resolved.tier, TargetTier::Global);
    assert_eq!(resolved.target.id.0, global);
}

#[test]
fn setting_the_language_moves_the_same_project_from_tier_four_to_tier_three() {
    let mut h = fixture();
    h.target(&Scope::global(), "any-editor");
    let rust = h.target(&Scope::language("Rust"), "a-rust-ide");
    let project = h.project;
    h.set_primary_language(project, "Rust");

    let resolved = handle_list(&mut h.ctx(), json_args(project, None))
        .unwrap()
        .resolved
        .unwrap();
    assert_eq!(resolved.tier, TargetTier::Language);
    assert_eq!(resolved.target.id.0, rust);
}

#[test]
fn a_project_override_outranks_a_location_override() {
    // §4bis.2a, the paragraph headed "A project outranks its own copies".
    let mut h = fixture();
    let (project, location) = (h.project, h.location);
    h.target(&Scope::global(), "any-editor");
    let proj = h.target(&Scope::project(project), "chosen-on-the-page");
    h.target(&Scope::location(location), "beside-this-copy");

    let resolved = handle_list(&mut h.ctx(), json_args(project, Some(location)))
        .unwrap()
        .resolved
        .unwrap();
    assert_eq!(resolved.tier, TargetTier::Project);
    assert_eq!(resolved.target.id.0, proj);
}

#[test]
fn no_row_in_any_scope_is_the_ask_tier_and_is_a_null_resolution() {
    // Tier 5 is not an enum member; it is the absence of one.
    let mut h = fixture();
    let project = h.project;
    let list = handle_list(&mut h.ctx(), json_args(project, None)).unwrap();
    assert!(list.resolved.is_none());
    assert!(list.rows.is_empty());
}

#[test]
fn a_row_that_failed_verification_still_resolves() {
    // §4bis.2a: "Resolution never consults verify_state ... Falling through to the next tier
    // would silently open an application the user did not choose."
    let mut h = fixture();
    let broken = h.target(&Scope::global(), "moved-by-an-update");
    h.set_verify_state(broken, "missing");

    let project = h.project;
    let resolved = handle_list(&mut h.ctx(), json_args(project, None))
        .unwrap()
        .resolved
        .unwrap();
    assert_eq!(resolved.target.id.0, broken);
    assert_eq!(resolved.target.verify_state, VerifyState::Missing);
}

#[test]
fn no_row_this_command_returns_carries_an_executable_or_an_argv() {
    // §2.4's trust rule, from the outbound side. The wire struct has no such field, so this
    // asserts against the serialised form — the one a renderer would actually see.
    let mut h = fixture();
    h.target(&Scope::global(), "any-editor");
    let project = h.project;
    let list = handle_list(&mut h.ctx(), json_args(project, None)).unwrap();
    let wire = serde_json::to_value(&list).unwrap();
    let text = wire.to_string();
    for banned in ["execBytes", "\"argv\"", "\"env\"", "exec_bytes"] {
        assert!(
            !text.contains(banned),
            "{banned} reached the wire in {text}"
        );
    }
    assert!(text.contains("execDisplay"));
}

#[test]
fn the_dispatcher_declines_a_command_it_does_not_own() {
    let mut h = fixture();
    let project = h.project;
    assert!(dispatch_targets_command(&mut h.ctx(), "projects.launch", json!({})).is_none());
    assert!(
        dispatch_targets_command(&mut h.ctx(), "targets.list", json_args(project, None)).is_some()
    );
}

#[test]
fn an_unknown_argument_key_is_refused_rather_than_ignored() {
    // §2.4: the renderer originates no path. A key this endpoint does not know is a protocol
    // error, not something to drop silently — dropping it is how a path arrives unnoticed.
    let mut h = fixture();
    let project = h.project;
    let err = handle_list(
        &mut h.ctx(),
        json!({ "projectId": project, "locationId": null, "path": "/etc" }),
    )
    .unwrap_err();
    assert_eq!(err.code, ErrorCode::Protocol);
}

#[test]
fn a_corrupt_verify_state_column_is_an_error_and_never_a_silent_default() {
    // The column is a closed set with a DDL CHECK. If a value outside it ever reaches the
    // reader, reporting `unverified` would turn a corrupt row into a plausible one.
    let mut h = fixture();
    h.target(&Scope::global(), "any-editor");
    h.index
        .conn()
        .execute_batch(
            "PRAGMA ignore_check_constraints = ON;
             UPDATE launch_target SET verify_state = 'nonsense';
             PRAGMA ignore_check_constraints = OFF;",
        )
        .expect("bypass the CHECK");
    let project = h.project;
    let err = handle_list(&mut h.ctx(), json_args(project, None)).unwrap_err();
    assert_eq!(err.code, ErrorCode::Internal);
}

#[test]
fn primary_language_reads_null_as_absent_rather_than_empty() {
    let h = fixture();
    let project = h.project;
    assert_eq!(
        codotheca_core::commands::targets::primary_language(h.index.conn(), ProjectId(project))
            .unwrap(),
        None
    );
    h.set_primary_language(project, "Rust");
    assert_eq!(
        codotheca_core::commands::targets::primary_language(h.index.conn(), ProjectId(project))
            .unwrap()
            .as_deref(),
        Some("Rust")
    );
}
