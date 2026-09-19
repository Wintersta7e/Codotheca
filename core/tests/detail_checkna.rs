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

// ---------------------------------------------------------------------------------------------
// AC-P3-31-1's wire half, and AC-P3-31-3's — `ProjectDetail.completion`
// ---------------------------------------------------------------------------------------------

fn detail(rig: &Rig, project: i64) -> serde_json::Value {
    let args = serde_json::json!({ "id": project });
    dispatch_detail_command(&rig.ctx(), "projects.get", args)
        .expect("the detail module owns this command")
        .expect("a project")
}

/// Ten rows in `CompletionCheck` declaration order, each with `observedAt` set and
/// `unknownReason` non-NULL **iff** the state is `unknown` — and the three scalars recounted from
/// `checks` in the test, because a wire triple that disagrees with its own array is the defect
/// `AC-P3-31-1` exists to catch one level up from the store.
#[test]
fn ac_p3_31_1_the_wire_triple_equals_a_recount_of_its_own_array() {
    let rig = Rig::new();
    let project = scored(&rig);
    let value = detail(&rig, project);
    let completion = value
        .get("completion")
        .and_then(|c| c.as_object())
        .expect("a scored project carries its detail");

    let checks = completion["checks"].as_array().expect("ten rows");
    assert_eq!(checks.len(), 10);

    let order: Vec<&str> = checks
        .iter()
        .map(|c| c["key"].as_str().expect("a key"))
        .collect();
    assert_eq!(
        order,
        vec![
            "remote",
            "readme",
            "license",
            "description",
            "tests",
            "ci",
            "ciGreen",
            "pushed",
            "deps",
            "release",
        ],
        "one order for every consumer, so a checklist never re-sorts to be stable"
    );

    for row in checks {
        assert!(row["observedAt"].is_i64(), "{row}");
        let unknown = row["state"] == "unknown";
        assert_eq!(
            !row["unknownReason"].is_null(),
            unknown,
            "a reason is present exactly when the state is unknown: {row}"
        );
    }

    let count = |state: &str| {
        i64::try_from(checks.iter().filter(|c| c["state"] == state).count()).expect("small")
    };
    assert_eq!(completion["lit"].as_i64(), Some(count("pass")));
    assert_eq!(
        completion["evaluable"].as_i64(),
        Some(count("pass") + count("fail"))
    );
    assert_eq!(completion["unknown"].as_i64(), Some(count("unknown")));
}

/// **`AC-P3-31-3`'s wire half, and the pairing that is the whole of it.** The `evaluable == 0`
/// case returns a **present** detail with ten rows and zero scalars **while
/// `ProjectRow.completionLit` is NULL** — hiding it would hide the only surface that says why
/// nothing could be scored.
///
/// The two genuine NULL cases are here too, because one alone cannot tell them apart.
#[test]
fn ac_p3_31_3_zero_evaluable_is_a_present_detail_beside_a_null_row() {
    let rig = Rig::new();
    // Nothing swept, no content scan, no remote, no refstate observed: every check is unknown or
    // `na`, and the projection is NULL.
    rig.project(1, "bare");
    rig.location(1, 1, "/a", "present", None, None, Some(NOW));
    rig.conn()
        .execute("UPDATE project SET authored_by_user = 1 WHERE id = 1", [])
        .unwrap();
    {
        let conn = rig.conn();
        let tx = conn.unchecked_transaction().unwrap();
        codotheca_core::completion::evaluate_and_write(&tx, ProjectId(1), NOW).unwrap();
        tx.commit().unwrap();
    }

    let value = detail(&rig, 1);
    let completion = value["completion"]
        .as_object()
        .expect("present, because ten rows exist");
    assert_eq!(completion["evaluable"].as_i64(), Some(0));
    assert_eq!(completion["lit"].as_i64(), Some(0));
    assert_eq!(completion["checks"].as_array().map(Vec::len), Some(10));
    assert!(
        value["row"]["completionLit"].is_null(),
        "the rendered figure is decided from the row, which is NULL"
    );
    assert!(value["row"]["completionApplicable"].is_null());

    // Not cloned: no location at all, so the evaluator wrote nothing and there is no detail.
    rig.project(2, "uncloned");
    rig.conn()
        .execute("UPDATE project SET authored_by_user = 1 WHERE id = 2", [])
        .unwrap();
    assert!(detail(&rig, 2)["completion"].is_null());

    // A Reference project, excluded for ever.
    rig.project(3, "reference");
    rig.location(3, 3, "/c", "present", None, None, Some(NOW));
    rig.conn()
        .execute(
            "UPDATE project SET authored_by_user = 0, is_reference = 1 WHERE id = 3",
            [],
        )
        .unwrap();
    assert!(detail(&rig, 3)["completion"].is_null());
}
