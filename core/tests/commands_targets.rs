#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! Criterion 7's command half: the five tiers as `targets.list` returns them.

use codotheca_core::commands::targets::{
    dispatch_targets_command, handle_list, handle_verify, TargetsCtx,
};
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
    dir: tempfile::TempDir,
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
        dir,
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

// ---------------------------------------------------------------------------------------------
// Task 2: `targets.setDefault` — re-heading a scope, and adopting a row into one.
// ---------------------------------------------------------------------------------------------

impl Fixture {
    fn set_default(&mut self, target: i64, scope: &Scope) {
        let project = self.project;
        self.set_default_for(target, project, scope);
    }

    fn set_default_for(&mut self, target: i64, project: i64, scope: &Scope) {
        let args = json!({
            "targetId": target,
            "projectId": scope.project.map(|_| project),
            "locationId": scope.location,
            "language": scope.language,
        });
        dispatch_targets_command(&mut self.ctx(), "targets.setDefault", args)
            .expect("targets.setDefault is owned here")
            .expect("set default");
    }

    fn new_project(&self) -> i64 {
        self.index
            .conn()
            .execute(
                "INSERT INTO project (name, seed_basename, created_at, updated_at)
                 VALUES ('q', 'q', 0, 0)",
                [],
            )
            .expect("insert project");
        self.index.conn().last_insert_rowid()
    }

    /// A tombstoned project that redirects to `survivor` — §1.5's merge, seeded directly.
    fn merge_project_into(&self, survivor: i64) -> i64 {
        let conn = self.index.conn();
        conn.execute(
            "INSERT INTO project (name, seed_basename, created_at, updated_at, merged_into)
             VALUES ('gone', 'gone', 0, 0, ?1)",
            rusqlite::params![survivor],
        )
        .expect("insert tombstone");
        let gone = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO project_redirect (old_project_id, new_project_id, merged_at)
             VALUES (?1, ?2, 0)",
            rusqlite::params![gone, survivor],
        )
        .expect("insert redirect");
        gone
    }

    fn row_count(&self) -> i64 {
        self.index
            .conn()
            .query_row("SELECT COUNT(*) FROM launch_target", [], |r| r.get(0))
            .expect("count")
    }
}

#[test]
fn the_tier_one_write_is_target_id_plus_project_id_and_the_other_two_null() {
    // Criterion 7 [v2.2]: "the bar drives that exact shape."
    let mut h = fixture();
    let global = h.target(&Scope::global(), "any-editor");
    let project = h.project;

    dispatch_targets_command(
        &mut h.ctx(),
        "targets.setDefault",
        json!({ "targetId": global, "projectId": project, "locationId": null, "language": null }),
    )
    .unwrap()
    .unwrap();

    let resolved = handle_list(&mut h.ctx(), json_args(project, None))
        .unwrap()
        .resolved
        .unwrap();
    assert_eq!(resolved.tier, TargetTier::Project);
    assert_eq!(resolved.target.name, "any-editor");
}

#[test]
fn adopting_a_global_row_into_a_project_leaves_the_global_scope_intact() {
    // L2: a row has one scope. Moving it would break every other project's default.
    let mut h = fixture();
    let global = h.target(&Scope::global(), "any-editor");
    let project = h.project;
    h.set_default(global, &Scope::project(project));

    let other = h.new_project();
    let still = handle_list(&mut h.ctx(), json_args(other, None))
        .unwrap()
        .resolved
        .unwrap();
    assert_eq!(still.tier, TargetTier::Global);
    assert_eq!(
        still.target.id.0, global,
        "the global row itself, not a copy"
    );
}

#[test]
fn an_adopted_row_is_not_marked_detected() {
    // L2: an override is a statement, not a detection.
    let mut h = fixture();
    let global = h.target(&Scope::global(), "any-editor");
    let project = h.project;
    h.set_default(global, &Scope::project(project));

    let row = handle_list(&mut h.ctx(), json_args(project, None))
        .unwrap()
        .resolved
        .unwrap();
    assert!(!row.target.detected);
    assert_ne!(row.target.id.0, global);
}

#[test]
fn re_heading_a_scope_the_row_is_already_in_reuses_the_row() {
    let mut h = fixture();
    let project = h.project;
    let first = h.target(&Scope::project(project), "first");
    let second = h.target(&Scope::project(project), "second");
    h.set_default(second, &Scope::project(project));

    let rows = handle_list(&mut h.ctx(), json_args(project, None)).unwrap();
    assert_eq!(rows.rows.len(), 2, "no copy was made");
    assert_eq!(rows.resolved.unwrap().target.id.0, second);
    assert!(rows.rows.iter().any(|r| r.id.0 == first));
}

#[test]
fn the_language_scope_is_written_with_the_canonical_string_not_the_drawer_tag() {
    // §4bis.2a: "`RS` is a display label and nothing stores it."
    let mut h = fixture();
    let global = h.target(&Scope::global(), "any-editor");
    let project = h.project;
    dispatch_targets_command(
        &mut h.ctx(),
        "targets.setDefault",
        json!({ "targetId": global, "projectId": null, "locationId": null, "language": "Rust" }),
    )
    .unwrap()
    .unwrap();
    h.set_primary_language(project, "Rust");

    let resolved = handle_list(&mut h.ctx(), json_args(project, None))
        .unwrap()
        .resolved
        .unwrap();
    assert_eq!(resolved.tier, TargetTier::Language);
    assert_eq!(resolved.target.language.as_deref(), Some("Rust"));
}

#[test]
fn set_default_follows_a_merge_redirect() {
    let mut h = fixture();
    let global = h.target(&Scope::global(), "any-editor");
    let project = h.project;
    let gone = h.merge_project_into(project); // `gone` now redirects to `project`

    h.set_default_for(global, gone, &Scope::project(gone));
    let resolved = handle_list(&mut h.ctx(), json_args(project, None))
        .unwrap()
        .resolved
        .unwrap();
    assert_eq!(resolved.tier, TargetTier::Project);
    assert_eq!(
        resolved.target.project_id.map(|p| p.0),
        Some(project),
        "the survivor's id was written, not the stale one"
    );
}

#[test]
fn set_default_on_an_unknown_target_reports_it() {
    let mut h = fixture();
    let err = dispatch_targets_command(
        &mut h.ctx(),
        "targets.setDefault",
        json!({ "targetId": 9_999, "projectId": null, "locationId": null, "language": null }),
    )
    .unwrap()
    .unwrap_err();
    assert_eq!(err.code, ErrorCode::Protocol);
}

#[test]
fn an_adopted_row_heads_its_new_scope_and_the_indices_stay_representable() {
    // The scope is renumbered densely from 0 rather than driven negative: `TargetRow` carries
    // `sortIndex: u32`, so a negative column value could not reach the wire intact.
    let mut h = fixture();
    let project = h.project;
    let sitting = h.target(&Scope::project(project), "already-here");
    let global = h.target(&Scope::global(), "adopted");
    h.set_default(global, &Scope::project(project));

    let list = handle_list(&mut h.ctx(), json_args(project, None)).unwrap();
    let adopted = list.resolved.unwrap().target;
    assert_eq!(adopted.name, "adopted");
    assert_eq!(h.row_count(), 3, "one copy, and only one");
    let sitting_row = list.rows.iter().find(|r| r.id.0 == sitting).unwrap();
    assert!(
        adopted.sort_index < sitting_row.sort_index,
        "the adopted row heads the scope: {} then {}",
        adopted.sort_index,
        sitting_row.sort_index
    );
}

// ---------------------------------------------------------------------------------------------
// Task 4: `targets.verify` — per row, language-blind.
// ---------------------------------------------------------------------------------------------

impl Fixture {
    /// A `launch_target` row whose executable is `exec`.
    fn target_at(&mut self, scope: &Scope, exec: &str) -> i64 {
        let id = self.target(scope, exec);
        self.index
            .conn()
            .execute(
                "UPDATE launch_target SET exec_bytes = ?2 WHERE id = ?1",
                rusqlite::params![id, exec.as_bytes()],
            )
            .expect("set exec");
        id
    }

    /// An executable file inside the fixture's tempdir.
    fn executable(&self, name: &str) -> String {
        let path = self.dir.path().join(name);
        std::fs::write(&path, b"#!/bin/sh\n").expect("write");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        }
        path.display().to_string()
    }

    /// A path nothing has ever created.
    fn missing_path(&self) -> String {
        self.dir.path().join("never-existed").display().to_string()
    }

    /// A file that exists and cannot be executed. Unix only: there is no execute bit to clear
    /// on the other target.
    #[cfg(unix)]
    fn non_executable_file(&self, name: &str) -> String {
        let path = self.dir.path().join(name);
        std::fs::write(&path, b"x").expect("write"); // fs::write leaves mode 0644
        path.display().to_string()
    }

    fn verified_at(&self, target: i64) -> Option<i64> {
        self.index
            .conn()
            .query_row(
                "SELECT verified_at FROM launch_target WHERE id = ?1",
                rusqlite::params![target],
                |r| r.get(0),
            )
            .expect("verified_at")
    }

    fn advance(&mut self, seconds: i64) {
        self.now += seconds;
    }
}

#[test]
fn verify_walks_every_scope_and_stamps_each_row_separately() {
    // §4bis.2a: "a language row is not covered by having verified the global row it was
    // copied from."
    let mut h = fixture();
    let project = h.project;
    let editor = h.executable("editor");
    let gone = h.missing_path();
    let global = h.target_at(&Scope::global(), &editor);
    let lang = h.target_at(&Scope::language("Rust"), &gone);
    let proj = h.target_at(&Scope::project(project), &editor);

    let rows = handle_verify(&mut h.ctx(), json!({ "targetId": null })).unwrap();
    assert_eq!(rows.len(), 3);
    let by_id = |id: i64| {
        rows.iter()
            .find(|r| r.target_id.0 == id)
            .unwrap_or_else(|| panic!("no verification for {id}"))
    };
    assert_eq!(by_id(global).verify_state, VerifyState::Ok);
    assert_eq!(by_id(lang).verify_state, VerifyState::Missing);
    assert_eq!(by_id(proj).verify_state, VerifyState::Ok);
    for row in &rows {
        assert_eq!(
            row.verified_at, h.now,
            "each row carries its own verified_at"
        );
    }
}

#[test]
fn verifying_one_row_leaves_the_others_stamps_alone() {
    let mut h = fixture();
    let editor = h.executable("editor");
    let a = h.target_at(&Scope::global(), &editor);
    let b = h.target_at(&Scope::language("Rust"), &editor);
    handle_verify(&mut h.ctx(), json!({ "targetId": null })).unwrap();
    let before = h.verified_at(b);

    h.advance(600);
    let rows = handle_verify(&mut h.ctx(), json!({ "targetId": a })).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(h.verified_at(b), before, "an untouched row keeps its stamp");
    assert!(h.verified_at(a) > before);
}

#[cfg(unix)]
#[test]
fn a_file_that_exists_but_is_not_executable_is_its_own_state() {
    // §4bis.5's four states, not three: `missing` and `not_executable` are different repairs.
    // Unix only — the other target has no execute bit to leave clear.
    let mut h = fixture();
    let blunt = h.non_executable_file("not-runnable");
    let id = h.target_at(&Scope::global(), &blunt);
    let rows = handle_verify(&mut h.ctx(), json!({ "targetId": id })).unwrap();
    assert_eq!(rows[0].verify_state, VerifyState::NotExecutable);
}

#[test]
fn verify_never_changes_which_row_resolves() {
    // §4bis.2a: "Resolution never consults verify_state."
    let mut h = fixture();
    let gone = h.missing_path();
    let global = h.target_at(&Scope::global(), &gone);
    handle_verify(&mut h.ctx(), json!({ "targetId": null })).unwrap();

    let project = h.project;
    let resolved = handle_list(&mut h.ctx(), json_args(project, None))
        .unwrap()
        .resolved
        .unwrap();
    assert_eq!(resolved.target.id.0, global);
    assert_eq!(resolved.target.verify_state, VerifyState::Missing);
}

#[test]
fn verify_covers_a_disabled_row_too() {
    // §4bis.2a: verification is unfiltered where resolution is filtered. A disabled row still
    // has an executable that can go missing.
    let mut h = fixture();
    let editor = h.executable("editor");
    let id = h.target_at(&Scope::global(), &editor);
    h.index
        .conn()
        .execute(
            "UPDATE launch_target SET disabled = 1 WHERE id = ?1",
            rusqlite::params![id],
        )
        .expect("disable");
    let rows = handle_verify(&mut h.ctx(), json!({ "targetId": null })).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].target_id.0, id);
}

#[test]
fn verify_on_an_unknown_target_reports_it_rather_than_returning_an_empty_list() {
    let mut h = fixture();
    let err = handle_verify(&mut h.ctx(), json!({ "targetId": 9_999 })).unwrap_err();
    assert_eq!(err.code, ErrorCode::Protocol);
}
