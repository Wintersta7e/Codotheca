#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! §2.4's trust rule, core-side. A launch target is an executable; this file is the reason
//! that fact is safe.

use std::path::{Path, PathBuf};

use codotheca_core::commands::targets::{
    check_dialog_origin, dispatch_targets_command, TargetsCtx,
};
use codotheca_core::derive::LocationKind;
use codotheca_core::index::Index;
use codotheca_core::launch::argv;
use codotheca_core::launch::catalogue::CwdMode;
use codotheca_core::launch::resolve::{load_target, StoredTarget};
use codotheca_core::protocol::{Bytes, TargetRow};
use serde_json::{json, Value};

const SEED: &str = "
INSERT INTO project (id, name, seed_basename, created_at, updated_at) VALUES (1, 'p', 'p', 0, 0);
INSERT INTO location (id, project_id, kind, distro, path_bytes, path_key, path_display,
                      volume_key, store_key, presence, repo_kind)
     VALUES (1, 1, 'linux', '', X'2F61', X'2F61', '/a', 'v', 's', 'present', 'worktree');
";

#[derive(Debug, Default)]
struct SilentSink;

impl codotheca_core::proto::EventSink for SilentSink {
    fn emit(&self, _topic: &str, _event: &str, _payload: Value) {}
}

struct Fixture {
    dir: tempfile::TempDir,
    index: Index,
    events: SilentSink,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().expect("tempdir");
    let index = Index::open(dir.path()).expect("open");
    index.conn().execute_batch(SEED).expect("seed");
    Fixture {
        dir,
        index,
        events: SilentSink,
    }
}

impl Fixture {
    fn ctx(&mut self) -> TargetsCtx<'_> {
        TargetsCtx {
            index: &mut self.index,
            events: &self.events,
            now: 1_000,
        }
    }

    /// An absolute path inside the fixture's tempdir — the shape a native dialog returns.
    fn abs(&self, name: &str) -> String {
        self.dir.path().join(name).display().to_string()
    }

    fn upsert(&mut self, args: Value) -> TargetRow {
        let value = dispatch_targets_command(&mut self.ctx(), "targets.upsert", args)
            .expect("targets.upsert is owned here")
            .expect("upsert");
        serde_json::from_value(value).expect("a TargetRow")
    }

    fn load(&self, id: i64) -> StoredTarget {
        load_target(self.index.conn(), id).expect("load")
    }

    fn row_count(&self) -> i64 {
        self.index
            .conn()
            .query_row("SELECT COUNT(*) FROM launch_target", [], |r| r.get(0))
            .expect("count")
    }
}

/// The `LaunchSite` the seeded location describes.
fn site() -> argv::LaunchSite {
    argv::LaunchSite {
        kind: LocationKind::Linux,
        distro: String::new(),
        path_bytes: b"/a".to_vec(),
    }
}

fn upsert_args(exec: &str, argv_json: &Value) -> Value {
    json!({
        "targetId": null, "kind": "editor", "name": "an editor",
        "execBytes": serde_json::to_value(Bytes(exec.as_bytes().to_vec())).unwrap(),
        "argv": argv_json.clone(),
        "projectId": null, "locationId": null, "language": null
    })
}

#[test]
fn a_relative_executable_is_refused() {
    // A native dialog never returns one; PATH resolution is not this process's to trust.
    for relative in ["sh", "code", "../payload", "bin/tool"] {
        assert!(
            check_dialog_origin(Path::new(relative)).is_err(),
            "{relative}"
        );
    }
}

#[test]
fn an_absolute_executable_is_accepted_on_either_host_form() {
    let accepted = if cfg!(windows) {
        "C:\\Program Files\\Ed\\ed.exe"
    } else {
        "/usr/bin/ed"
    };
    assert!(check_dialog_origin(Path::new(accepted)).is_ok());
}

#[test]
fn a_relative_executable_is_refused_through_the_command_and_stores_nothing() {
    // The guard is worth nothing if the command does not call it.
    let mut h = fixture();
    let err = dispatch_targets_command(
        &mut h.ctx(),
        "targets.upsert",
        upsert_args("sh", &json!([])),
    )
    .unwrap()
    .unwrap_err();
    assert_eq!(err.code, codotheca_core::protocol::ErrorCode::Protocol);
    assert_eq!(h.row_count(), 0, "nothing was stored");
}

#[test]
fn argv_zero_is_never_promoted_to_a_program() {
    // The single most valuable assertion in this file. `argv` is renderer-supplied; if any
    // layer ever treated argv[0] as the program, a renderer would have an executable.
    let mut h = fixture();
    let exec = h.abs("real-editor");
    let row = h.upsert(upsert_args(&exec, &json!(["/bin/sh", "-c", "whoami"])));
    let stored = h.load(row.id.0);
    let inv = argv::build(&stored, &site()).expect("an invocation");

    assert_eq!(inv.program, PathBuf::from(&exec));
    assert_ne!(inv.program, PathBuf::from("/bin/sh"));
    assert!(
        inv.argv.iter().any(|a| a == "-c"),
        "argv is stored, just never as the program"
    );
}

#[test]
fn nothing_in_the_argument_list_is_expanded_or_interpreted() {
    let mut h = fixture();
    let hostile = "$(id) `id` %PATH% ${HOME} && rm";
    let exec = h.abs("real-editor");
    let row = h.upsert(upsert_args(&exec, &json!([hostile])));
    let stored = h.load(row.id.0);
    assert_eq!(
        stored.args,
        vec![hostile.to_owned()],
        "stored byte for byte"
    );
}

#[test]
fn the_returned_row_carries_a_display_string_and_no_executable() {
    let mut h = fixture();
    let exec = h.abs("real-editor");
    let row = h.upsert(upsert_args(&exec, &json!([])));
    let text = serde_json::to_value(&row).unwrap().to_string();
    assert!(!text.contains("execBytes") && !text.contains("\"argv\""));
    assert!(row.exec_display.contains("real-editor"));
}

#[test]
fn a_hand_added_target_gets_cwd_mode_location() {
    // C3: the column is NOT NULL, phase 1 draws no control, and `none` would open the editor
    // at an unrelated directory.
    let mut h = fixture();
    let exec = h.abs("real-editor");
    let row = h.upsert(upsert_args(&exec, &json!([])));
    assert_eq!(h.load(row.id.0).cwd_mode, CwdMode::Location);
}

#[test]
fn upsert_is_privileged_in_the_schema_it_is_generated_from() {
    // Guard 1 of the three, asserted by name rather than restated: if the schema ever loses
    // `privileged`, the bridge stops refusing this command from the renderer.
    let schema = std::fs::read_to_string("../protocol/schema/protocol.json").unwrap();
    let schema: Value = serde_json::from_str(&schema).unwrap();
    let cmd = schema["commands"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "targets.upsert")
        .expect("targets.upsert");
    assert_eq!(cmd["privileged"], json!(true));
}

#[test]
fn upsert_with_a_target_id_rewrites_that_row_and_adds_none() {
    let mut h = fixture();
    let first_exec = h.abs("first");
    let first = h.upsert(upsert_args(&first_exec, &json!([])));
    let second_exec = h.abs("second");
    let mut again = upsert_args(&second_exec, &json!([]));
    again["targetId"] = json!(first.id.0);
    let second = h.upsert(again);

    assert_eq!(second.id, first.id);
    assert_eq!(h.row_count(), 1);
    assert!(second.exec_display.contains("second"));
}

#[test]
fn rewriting_a_rows_executable_drops_the_verification_it_no_longer_describes() {
    // `verify_state` describes a specific file. Keeping `ok` across a change of executable
    // would claim a currency the row does not have (§6, §4bis.5).
    let mut h = fixture();
    let first = h.upsert(upsert_args(&h.abs("first"), &json!([])));
    h.index
        .conn()
        .execute(
            "UPDATE launch_target SET verify_state = 'ok', verified_at = 5 WHERE id = ?1",
            rusqlite::params![first.id.0],
        )
        .unwrap();

    let mut again = upsert_args(&h.abs("second"), &json!([]));
    again["targetId"] = json!(first.id.0);
    let second = h.upsert(again);
    assert_eq!(
        second.verify_state,
        codotheca_core::protocol::VerifyState::Unverified
    );
    assert_eq!(second.verified_at, None);
}

#[test]
fn an_empty_name_is_refused_rather_than_stored_as_a_blank_menu_entry() {
    let mut h = fixture();
    let mut args = upsert_args(&h.abs("real-editor"), &json!([]));
    args["name"] = json!("");
    let err = dispatch_targets_command(&mut h.ctx(), "targets.upsert", args)
        .unwrap()
        .unwrap_err();
    assert_eq!(err.code, codotheca_core::protocol::ErrorCode::Protocol);
    assert_eq!(h.row_count(), 0);
}

#[test]
fn a_dialog_chosen_target_heads_its_scope() {
    // The user just picked it in a dialog; it is the one they meant to use.
    let mut h = fixture();
    h.index
        .conn()
        .execute(
            "INSERT INTO launch_target
                 (kind, name, exec_bytes, args_json, cwd_mode, env_json, sort_index, detected,
                  verify_state)
             VALUES ('editor', 'detected', X'2F62696E2F65', '[]', 'location', '{}', 0, 1, 'ok')",
            [],
        )
        .unwrap();
    let row = h.upsert(upsert_args(&h.abs("chosen"), &json!([])));
    assert!(row.sort_index < 1, "one below the scope's minimum");
    assert!(
        !row.detected,
        "a dialog choice is a statement, not a detection"
    );
}
