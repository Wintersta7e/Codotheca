//! `projects.setCheckNa` — the command half of §31.4's two stored facts.
//!
//! The rule it writes is asserted in `acceptance_completion.rs` against the store; what is
//! asserted here is the **command**: that it refuses an unknown project, that it announces an
//! accepted write, and that it announces nothing when the value is already what was asked for.

#![cfg(feature = "testkit")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

#[path = "support/detail_rig.rs"]
mod detail_rig;

use codotheca_core::detail::dispatch_detail_command;
use codotheca_core::protocol::ProjectId;
use detail_rig::{Rig, NOW};

fn call(rig: &Rig, project: i64, check: &str, na: &serde_json::Value) -> Result<(), String> {
    let args = serde_json::json!({ "id": project, "check": check, "na": na });
    dispatch_detail_command(&rig.ctx(), "projects.setCheckNa", args)
        .expect("the detail module owns this command")
        .map(|_| ())
        .map_err(|e| e.message)
}

fn scored(rig: &Rig) -> i64 {
    rig.project(1, "alpha");
    rig.location(1, 1, "/a", "present", Some("main"), Some(0), Some(NOW));
    let conn = rig.conn();
    conn.execute(
        "UPDATE project SET authored_by_user = 1, archetype = 'library' WHERE id = 1",
        [],
    )
    .unwrap();
    conn.execute(
        "UPDATE location SET refstate_observed_at = ?1 WHERE id = 1",
        [NOW],
    )
    .unwrap();
    // One evaluation, so there are ten rows for the command to rule on.
    let tx = conn.unchecked_transaction().unwrap();
    codotheca_core::completion::evaluate_and_write(&tx, ProjectId(1), NOW).unwrap();
    tx.commit().unwrap();
    1
}

#[test]
fn an_unknown_project_is_refused_rather_than_silently_written() {
    let rig = Rig::new();
    let _ = scored(&rig);
    let err = call(&rig, 404, "tests", &serde_json::Value::Bool(true)).expect_err("refused");
    assert!(err.contains("404"), "{err}");
}

/// **One `projects.upserted` per accepted write, and none when nothing changed.**
///
/// The tier frame on the shelf reads the projection, so a ruling that moved the denominator moved
/// a rendered figure and the open page must follow its own write. A no-op that emitted would
/// refresh every tile on a click that changed nothing.
#[test]
fn an_accepted_write_announces_once_and_a_no_op_announces_nothing() {
    let rig = Rig::new();
    let project = scored(&rig);
    let before = rig.sink.events().len();

    call(&rig, project, "tests", &serde_json::Value::Bool(true)).expect("accepted");
    let after_first = rig.sink.events().len();
    assert_eq!(
        after_first - before,
        1,
        "an accepted write announces exactly once"
    );

    // The same value again: already what was asked for, so nothing is written and nothing is
    // announced.
    call(&rig, project, "tests", &serde_json::Value::Bool(true)).expect("accepted");
    assert_eq!(
        rig.sink.events().len(),
        after_first,
        "a no-op announced a change"
    );

    // Clearing it back to the proposal is a real change again.
    call(&rig, project, "tests", &serde_json::Value::Null).expect("accepted");
    assert_eq!(rig.sink.events().len(), after_first + 1);
}
