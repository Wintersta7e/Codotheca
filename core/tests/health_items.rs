//! §30's open-item list — **what the reading hands the page**, and so what §33 lights layers from.
//!
//! The exclusions are applied once, upstream, when the list is produced (§33.1), and every item
//! set aside here keeps its row: nothing is closed and nothing is paid.
#![cfg(feature = "testkit")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

#[path = "support/detail_rig.rs"]
mod detail_rig;

use codotheca_core::detail::get::handle_project_get;
use codotheca_core::health::read_for_project;
use codotheca_core::protocol::{HealthState, ProjectId};
use detail_rig::{Rig, NOW};
use serde_json::json;

/// A project that reads `live` once enrolled — authored, not Reference, one present copy — with
/// one open item on a check that is on.
fn project_with_item(rig: &Rig, id: i64) {
    rig.project(id, &format!("p{id}"));
    rig.location(
        id,
        id,
        &format!("/p{id}"),
        "present",
        Some("main"),
        Some(0),
        Some(NOW),
    );
    let conn = rig.conn();
    conn.execute(
        "UPDATE project SET authored_by_user = 1 WHERE id = ?1",
        [id],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO debt_sweep (project_id, source, outcome, item_count, observed_at)
         VALUES (?1, 'missing_readme', 'complete', 1, ?2)",
        rusqlite::params![id, NOW],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO debt_item (project_id, subject_key, source, fingerprint, state, scoring,
                                first_seen_at, last_seen_at)
         VALUES (?1, ?3, 'missing_readme', '', 'open', 'scored', ?2, ?2)",
        rusqlite::params![id, NOW, format!("lineage:p{id}|remote:")],
    )
    .unwrap();
}

fn item_rows(rig: &Rig) -> i64 {
    rig.conn()
        .query_row("SELECT count(*) FROM debt_item", [], |r| r.get(0))
        .unwrap()
}

/// §33.1 — **Reference and backlog-suppressed projects render as clean metal**, because a reading
/// that says nothing hands the page nothing: no list, and so no layer. An enrolled `live` project
/// beside them keeps its item, and every row stays where it was.
#[test]
fn an_absent_or_suppressed_reading_hands_the_page_no_items() {
    let rig = Rig::new();
    for id in 1..=4 {
        project_with_item(&rig, id);
    }
    let rows = item_rows(&rig);
    let conn = rig.conn();
    conn.execute("UPDATE project SET is_archived = 1 WHERE id = 2", [])
        .unwrap();
    conn.execute(
        "UPDATE project SET is_reference = 1, authored_by_user = 0 WHERE id = 3",
        [],
    )
    .unwrap();

    // `projects.get` enrols what it opens (§30.5), so an unenrolled project is only observable
    // before any get: through the producer the page's field is built from.
    let (unenrolled, unenrolled_items) = read_for_project(conn, ProjectId(4)).unwrap();
    assert_eq!(unenrolled.state, HealthState::Suppressed);

    let get = |id: i64| handle_project_get(&rig.ctx(), json!({ "id": id })).unwrap();
    let live = get(1);
    let archived = get(2);
    let reference = get(3);
    eprintln!(
        "items handed down — live: {}, archived ({:?}): {}, Reference ({:?}): {}, unenrolled \
         ({:?}): {}; item rows {rows} -> {}",
        live.debt.len(),
        archived.health.state,
        archived.debt.len(),
        reference.health.state,
        reference.debt.len(),
        unenrolled.state,
        unenrolled_items.len(),
        item_rows(&rig)
    );

    assert_eq!(live.health.state, HealthState::Live);
    assert_eq!(live.debt.len(), 1, "the control lost its item");

    assert_eq!(archived.health.state, HealthState::Suppressed);
    assert!(
        archived.debt.is_empty(),
        "an archived project's items reached the page"
    );
    assert_eq!(reference.health.state, HealthState::Absent);
    assert!(
        reference.debt.is_empty(),
        "a Reference project's items reached the page"
    );
    assert!(
        unenrolled_items.is_empty(),
        "an unenrolled project's items were handed down"
    );
    assert_eq!(item_rows(&rig), rows, "setting items aside closed one");
}
