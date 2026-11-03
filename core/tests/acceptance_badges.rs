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
use std::path::Path;

use codotheca_core::index::rebuild::{rebuild_in_place, RebuildOutcome};
use codotheca_core::index::sidecar::{counts, read, RestoreCtx, RestoreOutcome, SECTIONS};
use codotheca_core::index::Index;
use rusqlite::types::Value;
use rusqlite::Connection;

const SCHEMA: &str = include_str!("../../protocol/schema/protocol.json");
const NOW: i64 = 1_760_000_000;

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

/// Every row of `badge_award`, every column, by kind.
fn awards(conn: &Connection) -> Vec<Vec<Value>> {
    let mut st = conn
        .prepare("SELECT * FROM badge_award ORDER BY kind")
        .unwrap();
    let width = st.column_count();
    st.query_map([], |r| (0..width).map(|i| r.get::<_, Value>(i)).collect())
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

/// The position of `project_id` in `SELECT *`.
fn project_column(conn: &Connection) -> usize {
    conn.prepare("SELECT * FROM badge_award")
        .unwrap()
        .column_names()
        .iter()
        .position(|name| *name == "project_id")
        .expect("badge_award has project_id")
}

/// Corrupt the index in `dir` and rebuild it through the production entry.
fn corrupt_and_rebuild(dir: &Path, now: i64) -> Index {
    std::fs::write(Index::db_path(dir), b"this is not a database").unwrap();
    match rebuild_in_place(dir, now) {
        Ok(RebuildOutcome::Rebuilt(report)) => {
            eprintln!("rebuilt: restored {:?}", report.restored);
        }
        other => panic!("expected a rebuild, got {other:?}"),
    }
    Index::open_at(dir, now + 1).unwrap()
}

/// §40.7, §48.8.3: awards travel as a global latch section. Every column comes back but
/// `project_id`, which waits for its subject; a second restore over held awards writes nothing.
#[test]
fn ac_p4_40_19_every_column_but_the_project_restores_and_a_second_restore_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let index = Index::open_at(dir.path(), NOW).unwrap();
    let conn = index.conn();
    conn.execute(
        "INSERT INTO project (name, seed_basename, lineage_key, created_at, updated_at)
         VALUES ('awarded', 'awarded', 'ee', 1, 1)",
        [],
    )
    .unwrap();
    let project = conn.last_insert_rowid();
    conn.execute_batch(&format!(
        "INSERT INTO badge_award (kind, earned_at, tz_offset_min, earned_on, project_id,
                                  subject_key, evidence, awarded_at, awarded_on)
         VALUES ('necromancer', 86400, 0, '1970-01-02', {project}, 'lineage:ee|remote:',
                 '{{\"projectName\":\"awarded\",\"gapDays\":200}}', 90000, '1970-01-02'),
                ('lifeline', 172800, 60, '1970-01-03', NULL, NULL,
                 '{{\"runStartOn\":\"1969-12-04\"}}', 180000, '1970-01-03'),
                ('polyglot', 0, 0, '1970-01-01', NULL, NULL,
                 '{{\"year\":1970}}', 90000, '1970-01-02');
         UPDATE badge_award SET landed_at = 190000, landed_on = '1970-01-03'
          WHERE kind = 'lifeline';"
    ))
    .unwrap();
    let before = awards(conn);
    let at = project_column(conn);
    assert_eq!(before.len(), 3);
    assert_eq!(
        before[1][at],
        Value::Integer(project),
        "necromancer names it"
    );

    let doc = read(&index.export_sidecar(NOW + 10).unwrap().path).unwrap();
    let exported = counts(&doc).get("badge_award").copied();
    eprintln!("rows exported: {exported:?}");
    assert_eq!(exported, Some(3));
    drop(index);

    let mut rebuilt = corrupt_and_rebuild(dir.path(), NOW + 20);
    let after = awards(rebuilt.conn());
    eprintln!("rows restored: {}", after.len());
    assert_eq!(after.len(), before.len(), "every award came back");
    for (was, now) in before.iter().zip(&after) {
        assert_eq!(now[at], Value::Null, "project_id waits for its subject");
        let mut expected = was.clone();
        expected[at] = Value::Null;
        assert_eq!(*now, expected, "every other column restores");
    }

    let spec = SECTIONS.iter().find(|s| s.name == "badge_award").unwrap();
    let rows = &doc.payload.sections["badge_award"];
    let written = rebuilt
        .with_tx(|tx| {
            let ctx = RestoreCtx {
                now: NOW + 30,
                project: None,
                locations: &[],
                source_generation: 1,
            };
            let mut written = 0;
            for row in rows {
                if let RestoreOutcome::Applied(n) = (spec.restore)(tx, row, &ctx)? {
                    written += n;
                } else {
                    panic!("a Global section answered Pending");
                }
            }
            Ok(written)
        })
        .unwrap();
    eprintln!("second restore wrote {written} rows");
    assert_eq!(written, 0);
    assert_eq!(
        awards(rebuilt.conn()),
        after,
        "a second restore changed a byte"
    );
}
