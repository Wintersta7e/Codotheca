//! `projects.setNote` and `locations.relocate` — the project page's two writes, and the closest
//! phase 1 comes to a destructive control, which is not one. Compiled only under `testkit`.
#![cfg(feature = "testkit")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

#[path = "support/detail_rig.rs"]
mod detail_rig;

use codotheca_core::detail::note::handle_project_set_note;
use codotheca_core::detail::relocate::handle_location_relocate;
use codotheca_core::git::{RepoFacts, RootCommit};
use codotheca_core::protocol::ErrorCode;
use codotheca_core::testing::GitReply;
use detail_rig::Rig;
use serde_json::json;

const ROOT_OID: &str = "0123456789abcdef0123456789abcdef01234567";

fn rig() -> Rig {
    let rig = Rig::new();
    rig.project(1, "alpha");
    rig.location(1, 1, "/srv/work/thing", "present", Some("main"), None, None);
    rig
}

/// A rig whose relocate target really is this project: the fake git answers the same root set
/// the project's `lineage_key` was computed from.
fn rig_with_matching_lineage() -> (Rig, std::path::PathBuf) {
    let rig = rig();
    let key = codotheca_core::identity::lineage::lineage_key(&[ROOT_OID.to_owned()], false)
        .expect("a non-shallow single-root repo has a lineage key");
    rig.conn()
        .execute("UPDATE project SET lineage_key = ?1 WHERE id = 1", [&key])
        .expect("seed lineage");
    let moved = rig.make_repo_dir("moved");
    rig.git.always_repo_facts(GitReply::Ok(RepoFacts {
        is_bare: false,
        is_shallow: false,
        git_dir: moved.join(".git"),
        common_dir: moved.join(".git"),
    }));
    rig.git.always_root_commits(GitReply::Ok(vec![RootCommit {
        oid: ROOT_OID.to_owned(),
        committed_at: 1_700_000_000,
        tz_offset_min: 0,
    }]));
    (rig, moved)
}

/// The same rig, but the folder the user picked is a different repository.
fn rig_with_foreign_lineage() -> (Rig, std::path::PathBuf) {
    let (rig, moved) = rig_with_matching_lineage();
    rig.git.always_root_commits(GitReply::Ok(vec![RootCommit {
        oid: "ffffffffffffffffffffffffffffffffffffffff".to_owned(),
        committed_at: 1_700_000_000,
        tz_offset_min: 0,
    }]));
    (rig, moved)
}

#[test]
fn set_note_writes_the_scratchpad_and_no_other_column() {
    // §8.5.4: it writes `project.notes` through the command that has one caller in phase 1.
    let rig = rig();
    let before = rig.fingerprint("project");
    handle_project_set_note(
        &rig.ctx(),
        json!({ "id": 1, "note": "left the parser half done" }),
    )
    .expect("setNote");
    assert_eq!(rig.notes(1).as_deref(), Some("left the parser half done"));

    // Every other column — `updated_at` and `last_touched_at` included — is untouched. A
    // scratchpad edit is not a git fact, and writing them would age a project on the shelf for
    // typing a reminder to yourself.
    let after = rig.fingerprint("project");
    let strip = |f: &str| f.replace("'left the parser half done'", "NULL");
    assert_eq!(strip(&after), strip(&before));

    // The page follows its own write.
    let events = rig.sink.named("projects", "upserted");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["row"]["id"], json!(1));
}

#[test]
fn an_emptied_note_is_null_and_not_an_empty_string() {
    // An empty string would give §5.2's description chain an empty first line to pick up and
    // blank the shelf row.
    let rig = rig();
    handle_project_set_note(&rig.ctx(), json!({ "id": 1, "note": "x" })).expect("set");
    handle_project_set_note(&rig.ctx(), json!({ "id": 1, "note": null })).expect("clear");
    assert_eq!(rig.notes(1), None);
    handle_project_set_note(&rig.ctx(), json!({ "id": 1, "note": "   " })).expect("blank");
    assert_eq!(rig.notes(1), None);
}

#[test]
fn set_note_refuses_a_project_that_is_not_there() {
    let rig = rig();
    let err =
        handle_project_set_note(&rig.ctx(), json!({ "id": 77, "note": "x" })).expect_err("refusal");
    assert_eq!(err.code, ErrorCode::Protocol);
}

#[test]
fn relocate_rewrites_the_path_columns_and_keeps_everything_else() {
    // §2.4: rewrites `path_bytes` in place and keeps `scan_generation`. Criterion 44: the
    // location row's path, and nothing else.
    let (rig, moved) = rig_with_matching_lineage();
    let before = rig.fingerprint("location");
    let out = handle_location_relocate(
        &rig.ctx(),
        json!({ "locationId": 1, "pathBytes": Rig::dialog_bytes(&moved) }),
    )
    .expect("relocate");

    let after = rig.fingerprint("location");
    assert_ne!(after, before, "the path columns did change");
    assert_eq!(out.location.path_display, moved.to_string_lossy());

    // It observed nothing, so it claims nothing: presence, generation and every observed fact
    // are still the last scan's answer. The row has been *pointed* somewhere, not *seen* there.
    let (presence, generation, last_seen, branch, refstate): (
        String,
        i64,
        Option<i64>,
        Option<String>,
        Option<i64>,
    ) = rig
        .conn()
        .query_row(
            "SELECT presence, scan_generation, last_seen_at, branch, refstate_observed_at
               FROM location WHERE id = 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .expect("read back");
    assert_eq!(presence, "present");
    assert_eq!(generation, 0);
    assert_eq!(last_seen, None);
    assert_eq!(branch.as_deref(), Some("main"));
    assert_eq!(refstate, None);
    assert_eq!(out.last_seen_at, None);
}

#[test]
fn relocate_moves_no_file() {
    // Criterion 44's own method: hash the location's tree before and after.
    let (rig, moved) = rig_with_matching_lineage();
    let before = Rig::hash_tree(&moved);
    assert!(!before.is_empty(), "the fixture tree is not empty");
    handle_location_relocate(
        &rig.ctx(),
        json!({ "locationId": 1, "pathBytes": Rig::dialog_bytes(&moved) }),
    )
    .expect("relocate");
    assert_eq!(Rig::hash_tree(&moved), before);

    // And no git call that could have written: §17 gives phase 1 no destructive operation, and
    // the only two operations this reaches for are the two reads the lineage check needs.
    let ops: Vec<String> = rig
        .git
        .calls()
        .into_iter()
        .map(|c| c.op.to_owned())
        .collect();
    assert_eq!(ops, vec!["repo_facts", "root_commits"]);
}

#[test]
fn relocate_refuses_a_target_whose_lineage_disagrees() {
    // §8.5.2: relocating must never silently re-identify. The refusal is a diagnostic message
    // and a code; the shell owns the prose (§2.4).
    let (rig, moved) = rig_with_foreign_lineage();
    let err = handle_location_relocate(
        &rig.ctx(),
        json!({ "locationId": 1, "pathBytes": Rig::dialog_bytes(&moved) }),
    )
    .expect_err("refusal");
    assert_eq!(err.code, ErrorCode::RepoUnreadable);
    assert!(err.message.contains("lineage"));
    // Refused means unwritten.
    let display: String = rig
        .conn()
        .query_row("SELECT path_display FROM location WHERE id = 1", [], |r| {
            r.get(0)
        })
        .expect("read back");
    assert_eq!(display, "/srv/work/thing");
}

#[test]
fn a_renderer_typed_path_cannot_even_deserialise() {
    // §2.4's trust rule, at the last door. The dialog produces tagged bytes (§2.5); a string is
    // not that shape, so it fails before the handler runs and nothing is written.
    let (rig, _moved) = rig_with_matching_lineage();
    let err = handle_location_relocate(
        &rig.ctx(),
        json!({ "locationId": 1, "pathBytes": "/etc/passwd" }),
    )
    .expect_err("refusal");
    assert_eq!(err.code, ErrorCode::Protocol);
    let display: String = rig
        .conn()
        .query_row("SELECT path_display FROM location WHERE id = 1", [], |r| {
            r.get(0)
        })
        .expect("read back");
    assert_eq!(display, "/srv/work/thing");
    assert!(rig.git.calls().is_empty(), "it refused before touching git");
}

#[test]
fn the_relocate_door_is_privileged_in_the_schema() {
    // The other half of §2.4's rule is that the renderer must not be able to *reach* this
    // command, and that rests on one flag in the schema. Without this, losing the flag would
    // open the renderer door silently with every other test still green. Read from the schema
    // file rather than from the generated code, because the schema is what codegen believes.
    let schema: serde_json::Value =
        serde_json::from_str(include_str!("../../protocol/schema/protocol.json"))
            .expect("schema parses");
    let commands = schema["commands"].as_array().expect("commands");
    let relocate = commands
        .iter()
        .find(|c| c["name"] == json!("locations.relocate"))
        .expect("locations.relocate is declared");
    assert_eq!(relocate["privileged"], json!(true));
    assert_eq!(relocate["args"]["pathBytes"], json!("Bytes"));
}

#[test]
fn the_banned_token_appears_nowhere_in_this_module() {
    // Criterion 44: the token names no command, no message and no accessible name. Scanning the
    // source is what makes that true of a message nobody thought to test.
    for src in [
        include_str!("../src/detail/mod.rs"),
        include_str!("../src/detail/get.rs"),
        include_str!("../src/detail/note.rs"),
        include_str!("../src/detail/relocate.rs"),
    ] {
        assert!(
            !src.to_ascii_uppercase().contains("FORGET"),
            "the token is in detail/"
        );
    }
}
