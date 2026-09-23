//! R118: `DebtItem.advisory` carries an advisory's display attributes on the wire — severity, fix
//! version and **every** CVE id — for the items an advisory backs, and is absent for every other
//! source. Read through `projects.get`, the handler the project page calls.
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
use codotheca_core::protocol::{AdvisoryDetail, DebtSource, Ecosystem};
use detail_rig::{Rig, NOW};
use serde_json::json;

/// One open item of `source` with `fingerprint`, written as §28's store writes it.
fn item(rig: &Rig, source: &str, fingerprint: &str) {
    rig.conn()
        .execute(
            "INSERT INTO debt_item (project_id, subject_key, source, fingerprint, state, scoring,
                                    first_seen_at, last_seen_at)
             VALUES (1, 'lineage:alpha|remote:', ?1, ?2, 'open', 'scored', ?3, ?3)",
            rusqlite::params![source, fingerprint, NOW],
        )
        .unwrap();
}

#[test]
fn an_advisory_item_carries_its_detail_and_every_other_item_none() {
    let rig = Rig::new();
    rig.project(1, "alpha");
    // A project whose reading is `live` — authored, one present copy — and a granted content
    // scan, so neither item is set aside before it can reach the wire: §30 hands an `absent`
    // reading no items, and an ungranted `todo_marker` is `off`.
    rig.location(1, 1, "/a", "present", Some("main"), Some(0), Some(NOW));
    let conn = rig.conn();
    conn.execute_batch(
        "UPDATE project SET authored_by_user = 1 WHERE id = 1;
         INSERT INTO app_meta (k, v) VALUES ('content_scan_enabled', '1');",
    )
    .unwrap();
    conn.execute_batch(
        "INSERT INTO advisory_sweep (id, started_at, settled_at, outcome, complete)
           VALUES (1, 1, 1, 'done', 1);
         INSERT INTO advisory (advisory_id, severity, summary, url, observed_at)
           VALUES ('GHSA-aaaa', 'critical', 's', 'u', 1);
         INSERT INTO advisory_cve (advisory_id, cve_id)
           VALUES ('GHSA-aaaa', 'CVE-2026-0002'), ('GHSA-aaaa', 'CVE-2026-0001');
         INSERT INTO advisory_triple (ecosystem, package_name, version, sweep_id, observed_at,
                                      answered)
           VALUES ('npm', 'left', '1.0.0', 1, 1, 1);
         INSERT INTO advisory_match (ecosystem, package_name, version, advisory_id,
                                     fix_available, fixed_version)
           VALUES ('npm', 'left', '1.0.0', 'GHSA-aaaa', 1, '2.0.0');",
    )
    .unwrap();
    item(&rig, "dependency_advisory", "npm:left:GHSA-aaaa");
    // The same fingerprint on another source: the detail is keyed by the source, never by the
    // fingerprint's shape.
    item(&rig, "todo_marker", "npm:left:GHSA-aaaa");

    let detail = handle_project_get(&rig.ctx(), json!({ "id": 1 })).expect("get");
    eprintln!(
        "advisory_detail_wire: {} item(s): {:?}",
        detail.debt.len(),
        detail
            .debt
            .iter()
            .map(|i| (i.source, i.advisory.is_some()))
            .collect::<Vec<_>>()
    );
    assert_eq!(detail.debt.len(), 2, "both items reached the wire");

    let advisory = detail
        .debt
        .iter()
        .find(|i| i.source == DebtSource::DependencyAdvisory)
        .expect("the advisory item");
    assert_eq!(
        advisory.advisory,
        Some(AdvisoryDetail {
            ecosystem: Ecosystem::Npm,
            package_name: "left".to_owned(),
            advisory_id: "GHSA-aaaa".to_owned(),
            cve_ids: vec!["CVE-2026-0001".to_owned(), "CVE-2026-0002".to_owned()],
            severity: Some("critical".to_owned()),
            fixed_version: Some("2.0.0".to_owned()),
        }),
        "the advisory's detail did not reach the wire"
    );

    let marker = detail
        .debt
        .iter()
        .find(|i| i.source == DebtSource::TodoMarker)
        .expect("the marker item");
    assert_eq!(marker.advisory, None, "a marker item named an advisory");
}
