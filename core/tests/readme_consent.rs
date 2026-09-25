#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! §25.5 — the consent that is not a flag.
//!
//! Two properties matter and both are about honesty rather than mechanism: a revoked consent is
//! **NULL**, which is the same value as never-granted and is correct — the user's decision is
//! *do not*, and a timestamp would date a grant that does not exist — and every change publishes
//! what the **column** now holds, so the renderer's optimistic flip reconciles against the row
//! rather than against the request it sent.

use codotheca_core::art::testsupport::CollectingSink;
use codotheca_core::protocol::{ErrorCode, ProjectId};
use codotheca_core::readme::consent::{readme_remote_at, set_readme_remote};
use codotheca_core::readme::{dispatch_readme_command, ReadmeCtx};
use codotheca_core::testing::TempIndex;

const NOW: i64 = 1_781_179_200;

#[test]
fn granting_writes_a_timestamp_and_publishes_it_revoking_writes_null() {
    let index = TempIndex::new();
    let project = index.insert_project();
    let sink = CollectingSink::default();
    let ctx = ReadmeCtx {
        index: index.index(),
        events: &sink,
        now: NOW,
    };

    assert_eq!(
        readme_remote_at(index.index().conn(), project).expect("read"),
        None,
        "never granted is the value every existing row starts at"
    );

    set_readme_remote(&ctx, project, true).expect("grant");
    assert_eq!(
        readme_remote_at(index.index().conn(), project).expect("read"),
        Some(NOW)
    );
    let events = sink.named("projects", "readme_remote_changed");
    assert_eq!(events.len(), 1, "one grant, one event");
    assert_eq!(events[0]["id"], project.0);
    assert_eq!(events[0]["allowedAt"], NOW);

    set_readme_remote(&ctx, project, false).expect("revoke");
    assert_eq!(
        readme_remote_at(index.index().conn(), project).expect("read"),
        None
    );
    let after_revoke = sink.named("projects", "readme_remote_changed");
    assert_eq!(after_revoke.len(), 2);
    assert_eq!(
        after_revoke[1]["allowedAt"],
        serde_json::Value::Null,
        "a revoked consent is NULL, not a dated denial"
    );
}

#[test]
fn granting_twice_emits_twice_and_leaves_one_row() {
    let index = TempIndex::new();
    let project = index.insert_project();
    let sink = CollectingSink::default();
    let ctx = ReadmeCtx {
        index: index.index(),
        events: &sink,
        now: NOW,
    };

    set_readme_remote(&ctx, project, true).expect("grant");
    set_readme_remote(&ctx, project, true).expect("grant again");
    assert_eq!(sink.named("projects", "readme_remote_changed").len(), 2);

    let rows: i64 = index
        .index()
        .conn()
        .query_row("SELECT COUNT(*) FROM project", [], |r| r.get(0))
        .expect("count");
    assert_eq!(rows, 1);
}

#[test]
fn an_id_that_names_no_project_is_a_protocol_error_and_writes_nothing() {
    let index = TempIndex::new();
    let sink = CollectingSink::default();
    let ctx = ReadmeCtx {
        index: index.index(),
        events: &sink,
        now: NOW,
    };

    let error = set_readme_remote(&ctx, ProjectId(9_999), true).expect_err("must refuse");
    assert_eq!(error.code(), ErrorCode::Protocol);
    assert!(
        sink.named("projects", "readme_remote_changed").is_empty(),
        "a refusal publishes nothing"
    );
}

#[test]
fn the_command_is_dispatched_by_name_and_returns_empty() {
    let index = TempIndex::new();
    let project = index.insert_project();
    let sink = CollectingSink::default();
    let ctx = ReadmeCtx {
        index: index.index(),
        events: &sink,
        now: NOW,
    };

    let answered = dispatch_readme_command(
        &ctx,
        "projects.setReadmeRemote",
        serde_json::json!({ "projectId": project.0, "allow": true }),
    )
    .expect("this module owns projects.setReadmeRemote")
    .expect("it answers");
    assert_eq!(answered, serde_json::json!({}), "the command returns Empty");
    assert_eq!(
        readme_remote_at(index.index().conn(), project).expect("read"),
        Some(NOW),
        "and the value travels on the event, not in the reply"
    );
}

/// §1.2's command may not grow a second effect, and this is the assertion that says so.
///
/// It is a source scan, which is the weaker form — but the thing being guarded against is a
/// later author *adding* the consent to a taxonomy command, and that is an edit to this file's
/// text. The count is printed because a gate whose passing run scanned nothing is a failing gate.
#[test]
fn the_flags_command_names_neither_the_column_nor_the_consent_command() {
    let flags = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/projects/flags.rs");
    let source = std::fs::read_to_string(&flags).expect("flags.rs is readable");
    assert!(
        source.len() > 500,
        "read real source, or every assertion below is vacuous: {} bytes",
        source.len()
    );

    let mut hits = Vec::new();
    for needle in [
        "readme_remote_at",
        "setReadmeRemote",
        "readme_remote_changed",
    ] {
        if source.contains(needle) {
            hits.push(needle);
        }
    }
    eprintln!(
        "flags.rs: scanned {} bytes for 3 names, {} hit(s)",
        source.len(),
        hits.len()
    );
    assert!(
        hits.is_empty(),
        "§1.2's three organisation primitives grew a security consent: {hits:?}"
    );
}
