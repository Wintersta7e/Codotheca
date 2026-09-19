//! `AC-P3-35-3`, the Rust half. §35.7: the comparator exists twice by design, and until this file
//! **nothing compared the two orders** — one side pinned the hash function to a literal and the
//! other asserted only that its own hash was self-consistent.
//!
//! One machine-readable fixture, read by this test and by
//! `app/src/renderer/shelf/orderCorpus.test.ts`, in the shape `protocol/query/corpus.json` already
//! uses for the grammar. Each case states `expectedIds` **and** `expectedOrderKey`: two
//! comparators agreeing on ids while disagreeing on the cursor §8.2 windows on is a real
//! divergence, and the second field is what makes it visible.
#![cfg(feature = "testkit")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::collections::BTreeSet;

use codotheca_core::projects::list::{order_key_of, sort_rows};
use codotheca_core::projects::rows::{LoadedRow, RowFacts};
use codotheca_core::protocol::{HealthState, HealthSummary, ProjectRow, SortKey};
use serde_json::Value;

/// Every key a case row may carry. **An unknown key is rejected rather than ignored**: a field
/// added to one side's loader and not the other is exactly the divergence a shared fixture is
/// otherwise free to hide.
const ROW_KEYS: [&str; 6] = [
    "id",
    "name",
    "lastTouchedAt",
    "sizeTrackedBytes",
    "healthState",
    "scoredOpen",
];

fn read(name: &str) -> Value {
    let path = format!("{}/../{name}", env!("CARGO_MANIFEST_DIR"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("{path} is readable"));
    serde_json::from_str(&text).unwrap_or_else(|_| panic!("{path} is JSON"))
}

fn loaded(row: &Value) -> LoadedRow {
    let object = row.as_object().expect("a row is an object");
    for key in object.keys() {
        assert!(
            ROW_KEYS.contains(&key.as_str()),
            "unknown row key in order-corpus.json: {key}"
        );
    }
    let mut wire = ProjectRow::for_test(row["id"].as_i64().expect("id"));
    row["name"]
        .as_str()
        .expect("name")
        .clone_into(&mut wire.name);
    wire.last_touched_at = row["lastTouchedAt"].as_i64().expect("lastTouchedAt");
    wire.size_tracked_bytes = row["sizeTrackedBytes"].as_i64();
    wire.health_summary = HealthSummary {
        state: serde_json::from_value::<HealthState>(row["healthState"].clone())
            .expect("healthState is a HealthState"),
        scored_open: row["scoredOpen"]
            .as_u64()
            .map(|n| u32::try_from(n).expect("scoredOpen fits")),
        unverified: None,
        unknown_checks: None,
        observed_at: None,
    };
    LoadedRow {
        row: wire,
        facts: RowFacts {
            authored_by_user: None,
            location_kind: None,
            distro: None,
            has_remote: false,
            has_submodules: false,
            has_readme: None,
            content_presence: None,
        },
    }
}

#[test]
fn ac_p3_35_3_rust_comparator_matches_every_order_corpus_case() {
    let doc = read("protocol/shelf/order-corpus.json");
    let cases = doc["cases"].as_array().expect("cases is an array");
    eprintln!("sort_corpus: compared {} case(s)", cases.len());
    assert!(
        !cases.is_empty(),
        "a run that compared no case proves nothing"
    );

    let mut failures: Vec<String> = Vec::new();
    for case in cases {
        let name = case["name"].as_str().expect("name");
        let sort: SortKey =
            serde_json::from_value(case["sort"].clone()).expect("sort is a SortKey");
        assert!(
            case["now"].as_i64().is_some_and(|now| now > 0),
            "{name}: the case carries no clock"
        );
        let rows: Vec<LoadedRow> = case["rows"]
            .as_array()
            .expect("rows is an array")
            .iter()
            .map(loaded)
            .collect();
        let mut refs: Vec<&LoadedRow> = rows.iter().collect();
        sort_rows(&mut refs, sort);
        let ids: Vec<i64> = refs.iter().map(|r| r.row.id.0).collect();
        let expected: Vec<i64> = case["expectedIds"]
            .as_array()
            .expect("expectedIds")
            .iter()
            .map(|v| v.as_i64().expect("an id"))
            .collect();
        if ids != expected {
            failures.push(format!(
                "{name}\n  expected {expected:?}\n  produced {ids:?}"
            ));
            continue;
        }
        // The cursor §8.2 windows on. Agreeing on ids while disagreeing here is a divergence the
        // id assertion alone cannot see.
        let key = order_key_of(&ids);
        let expected_key = case["expectedOrderKey"].as_str().expect("expectedOrderKey");
        if key != expected_key {
            failures.push(format!(
                "{name}\n  expected orderKey {expected_key}\n  produced {key}"
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "order corpus divergence:\n{}",
        failures.join("\n")
    );
}

/// The coverage claim is **derived from the contract**, never from a literal: a count a human
/// maintains is a defect with a delay (§36.2 rule 9).
#[test]
fn ac_p3_35_3_the_corpus_covers_every_sort_key_the_schema_declares() {
    let schema = read("protocol/schema/protocol.json");
    let variants: BTreeSet<String> = schema["types"]["SortKey"]["variants"]
        .as_array()
        .expect("SortKey declares variants")
        .iter()
        .map(|v| v.as_str().expect("a variant").to_owned())
        .collect();
    eprintln!("sort_corpus: derived {} SortKey variant(s)", variants.len());
    assert!(
        !variants.is_empty(),
        "a run that derived no variant proves nothing"
    );

    let doc = read("protocol/shelf/order-corpus.json");
    let covered: BTreeSet<String> = doc["cases"]
        .as_array()
        .expect("cases")
        .iter()
        .map(|c| c["sort"].as_str().expect("sort").to_owned())
        .collect();
    assert_eq!(
        covered, variants,
        "every variant is exercised, and no other"
    );

    let names: Vec<&str> = doc["cases"]
        .as_array()
        .expect("cases")
        .iter()
        .map(|c| c["name"].as_str().expect("name"))
        .collect();
    let unique: BTreeSet<&&str> = names.iter().collect();
    assert_eq!(unique.len(), names.len(), "both sides report cases by name");
}

#[test]
fn ac_p3_35_3_the_loader_names_a_row_key_it_does_not_know() {
    let row = serde_json::json!({
        "id": 1, "name": "a", "lastTouchedAt": 0, "sizeTrackedBytes": null,
        "healthState": "absent", "scoredOpen": null, "weighting": 3
    });
    let panicked = std::panic::catch_unwind(|| loaded(&row));
    let message = *panicked
        .expect_err("an unknown key must be rejected")
        .downcast::<String>()
        .expect("a named panic");
    assert!(message.contains("weighting"), "{message}");
}
