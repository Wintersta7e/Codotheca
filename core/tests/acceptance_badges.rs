//! §40's acceptance suite: the badge types on the wire, read from the schema itself.
//!
//! Every test here prints what it read and fails when it read nothing, so a renamed type or an
//! emptied field set cannot pass by scanning zero fields.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::collections::BTreeSet;

const SCHEMA: &str = include_str!("../../protocol/schema/protocol.json");

/// The field names of one declared struct, in declaration order.
fn fields_of(name: &str) -> Vec<String> {
    let doc: serde_json::Value = serde_json::from_str(SCHEMA).unwrap();
    let Some(decl) = doc["types"].get(name) else {
        panic!("{name} is not a declared type");
    };
    let fields = decl["fields"]
        .as_object()
        .unwrap_or_else(|| panic!("{name} declares no fields"));
    let names: Vec<String> = fields.keys().cloned().collect();
    eprintln!("{name}: {} fields {names:?}", names.len());
    assert!(!names.is_empty(), "{name} declares zero fields");
    names
}

/// Field names any of `words` occurs in, case-insensitively.
fn naming_any<'a>(fields: &'a [String], words: &[&str]) -> Vec<&'a str> {
    fields
        .iter()
        .filter(|f| {
            let lower = f.to_lowercase();
            words.iter().any(|w| lower.contains(w))
        })
        .map(String::as_str)
        .collect()
}

/// §40.10: a badge is awarded or it is absent. No type on the wire can carry how near an
/// unearned badge is, so the sets are pinned exactly and a later field fails here for a human
/// to read.
#[test]
fn ac_p4_40_6_the_badge_types_carry_no_progress() {
    const PROGRESS: &[&str] = &[
        "threshold",
        "progress",
        "remaining",
        "toward",
        "unearned",
        "locked",
        "length",
        "count",
        "total",
    ];
    let expected: [(&str, &[&str]); 4] = [
        (
            "BadgeAward",
            &["kind", "earnedOn", "projectId", "evidence", "landed"],
        ),
        (
            "BadgeEvidence",
            &[
                "projectName",
                "gapDays",
                "year",
                "languages",
                "creditedSeconds",
                "projects",
                "runStartOn",
            ],
        ),
        ("BadgeEvidenceProject", &["name", "evaluable"]),
        ("BadgesListed", &["awards", "due", "basis"]),
    ];
    let mut read = 0;
    for (name, want) in expected {
        let fields = fields_of(name);
        read += fields.len();
        let named = naming_any(&fields, PROGRESS);
        assert!(named.is_empty(), "{name} carries progress: {named:?}");
        let got: BTreeSet<&str> = fields.iter().map(String::as_str).collect();
        let want: BTreeSet<&str> = want.iter().copied().collect();
        assert_eq!(got, want, "{name}'s fields are not §40.10's set");
    }
    eprintln!("{read} fields read across 4 types");
    assert!(read > 0, "read zero fields");
}

/// §40.7: evidence names what was earned, never a volume of lines or commits.
#[test]
fn ac_p4_40_17_evidence_names_no_line_or_commit() {
    const VOLUME: &[&str] = &["line", "insertion", "deletion", "commit"];
    let mut read = 0;
    for name in ["BadgeEvidence", "BadgeEvidenceProject"] {
        let fields = fields_of(name);
        read += fields.len();
        let named = naming_any(&fields, VOLUME);
        assert!(named.is_empty(), "{name} names a volume: {named:?}");
    }
    eprintln!("{read} evidence fields read");
    assert!(read > 0, "read zero fields");
}
