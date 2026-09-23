#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
#![cfg(feature = "testkit")]
//! Health and completion must read the same standing scored evidence the same way.

#[path = "support/detail_rig.rs"]
mod detail_rig;

use codotheca_core::completion::evaluate_and_write;
use codotheca_core::debt::store::SqliteDebtStore;
use codotheca_core::detail::get::handle_project_get;
use codotheca_core::protocol::{
    CheckOutcome, CheckState, CompletionCheck, DebtSource, ProjectDetail, ProjectId,
};
use codotheca_core::surfaces::{dispatch_surface_command, SurfaceCtx};
use detail_rig::{Rig, NOW};
use serde_json::json;

fn live_project(rig: &Rig, id: i64) {
    rig.project(id, &format!("project-{id}"));
    rig.location(
        id,
        id,
        &format!("/fixture-{id}"),
        "present",
        Some("main"),
        Some(0),
        Some(NOW),
    );
    rig.conn()
        .execute(
            "UPDATE project SET authored_by_user = 1 WHERE id = ?1",
            [id],
        )
        .unwrap();
}

fn seed_presence_answers(rig: &Rig, project: i64) {
    rig.conn()
        .execute(
            "INSERT INTO project_content_scan
                (project_id, head_oid, complete_head_oid, blobs_total, blobs_pending,
                 predicate_version, has_readme, has_license, has_tests, has_ci,
                 presence_observed_at, enumerated_at, completed_at)
             VALUES (?1, 'head0000', 'head0000', 4, 0, 1,
                     'present', 'absent', 'present', 'present', ?2, ?2, ?2)",
            rusqlite::params![project, NOW],
        )
        .unwrap();
}

fn settle(rig: &Rig, project: i64, now: i64) {
    let tx = rig.conn().unchecked_transaction().unwrap();
    codotheca_core::debt::singletons::settle_singletons(
        &tx,
        ProjectId(project),
        now,
        0,
        &SqliteDebtStore,
    )
    .unwrap();
    evaluate_and_write(&tx, ProjectId(project), now).unwrap();
    tx.commit().unwrap();
}

fn set_missing_license(rig: &Rig, enabled: bool, now: i64) {
    let ctx = SurfaceCtx {
        index: &rig.index,
        now,
    };
    dispatch_surface_command(
        &ctx,
        "settings.set",
        json!({
            "patch": {
                "healthChecks": [{
                    "check": "missing_license",
                    "enabled": enabled
                }]
            }
        }),
    )
    .expect("settings.set is routed")
    .expect("settings.set succeeds");
}

fn missing_license_sweep_count(rig: &Rig, project: i64) -> i64 {
    rig.conn()
        .query_row(
            "SELECT count(*) FROM debt_sweep
              WHERE project_id = ?1 AND source = 'missing_license'",
            [project],
            |row| row.get(0),
        )
        .unwrap()
}

fn detail(rig: &Rig, project: i64) -> ProjectDetail {
    handle_project_get(&rig.ctx(), json!({ "id": project })).unwrap()
}

fn agreement_states(detail: &ProjectDetail) -> (CheckOutcome, CheckState) {
    let health = detail
        .health
        .checks
        .iter()
        .find(|check| check.id == DebtSource::MissingLicense)
        .expect("missing_license health row")
        .outcome;
    let completion = detail
        .completion
        .as_ref()
        .expect("completion detail")
        .checks
        .iter()
        .find(|check| check.key == CompletionCheck::License)
        .expect("license completion row")
        .state;
    (health, completion)
}

fn seed_partial(rig: &Rig, project: i64, with_item: bool) {
    rig.conn()
        .execute(
            "INSERT INTO debt_sweep
                (project_id, source, outcome, location_id, basis, item_count, observed_at)
             VALUES (?1, 'missing_license', 'partial', ?1, 'head', ?2, ?3)",
            rusqlite::params![project, i64::from(with_item), NOW],
        )
        .unwrap();
    if with_item {
        rig.conn()
            .execute(
                "INSERT INTO debt_item
                    (project_id, subject_key, source, fingerprint, state, scoring,
                     last_seen_location_id, basis, first_seen_at, last_seen_at)
                 VALUES (?1, ?2, 'missing_license', '', 'open', 'scored',
                         ?1, 'head', ?3, ?3)",
                rusqlite::params![project, format!("subject-{project}"), NOW],
            )
            .unwrap();
    }
}

fn evaluate(rig: &Rig, project: i64) {
    let tx = rig.conn().unchecked_transaction().unwrap();
    evaluate_and_write(&tx, ProjectId(project), NOW).unwrap();
    tx.commit().unwrap();
}

#[test]
fn the_health_row_and_the_completion_check_agree_right_after_a_switch_on() {
    let rig = Rig::new();
    live_project(&rig, 1);
    seed_presence_answers(&rig, 1);

    settle(&rig, 1, NOW);
    set_missing_license(&rig, false, NOW + 1);
    settle(&rig, 1, NOW + 2);
    assert_eq!(
        missing_license_sweep_count(&rig, 1),
        0,
        "the off-settle restored the deleted sweep row"
    );
    set_missing_license(&rig, true, NOW + 3);

    let states = agreement_states(&detail(&rig, 1));
    eprintln!(
        "missing_license after switch-on: health {:?}, completion {:?}",
        states.0, states.1
    );
    assert_eq!(states.0, CheckOutcome::Failed);
    assert_eq!(
        states.1,
        CheckState::Fail,
        "completion discarded a scored open item after switch-off removed the sweep row"
    );
}

#[test]
fn a_partial_sweep_with_a_scored_open_item_fails_in_both_readings() {
    let rig = Rig::new();
    live_project(&rig, 1);
    seed_partial(&rig, 1, true);
    evaluate(&rig, 1);

    let states = agreement_states(&detail(&rig, 1));
    eprintln!(
        "missing_license after a partial sweep with an item: health {:?}, completion {:?}",
        states.0, states.1
    );
    assert_eq!(states.0, CheckOutcome::Failed);
    assert_eq!(
        states.1,
        CheckState::Fail,
        "completion discarded a scored open item from a partial sweep"
    );
}

#[test]
fn a_partial_sweep_without_an_item_stays_unknown_in_completion() {
    let rig = Rig::new();
    live_project(&rig, 1);
    seed_partial(&rig, 1, false);
    evaluate(&rig, 1);

    let states = agreement_states(&detail(&rig, 1));
    eprintln!(
        "missing_license after a partial sweep without an item: health {:?}, completion {:?}",
        states.0, states.1
    );
    assert_eq!(states.0, CheckOutcome::Unknown);
    assert_eq!(states.1, CheckState::Unknown);
}
