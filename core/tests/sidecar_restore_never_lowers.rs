//! §48.8.3: a restore never lowers what is present. A write-once value restores only where none
//! is, so a row the index already holds is left exactly as it was. Every section whose rule is
//! not `Replace` has its case here.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use codotheca_core::index::pending::resolve_subject_unique;
use codotheca_core::index::sidecar::{
    RestoreCtx, RestoreRule, SectionSpec, SidecarLocationKey, SECTIONS,
};
use codotheca_core::index::subject::ProjectSubject;
use codotheca_core::index::Index;
use codotheca_core::protocol::{LocationId, ProjectId};
use codotheca_core::testing::sidecar::{FixtureIds, SECTION_FIXTURES};
use rusqlite::types::Value;
use rusqlite::{Connection, OptionalExtension as _};

const NOW: i64 = 1_760_000_000;

/// One section's case: the statements that make the present rows differ from the exported ones,
/// and the tables whose rows must come through the restore unchanged.
struct Case {
    section: &'static str,
    differ: &'static str,
    tables: &'static [&'static str],
}

const CASES: &[Case] = &[
    Case {
        section: "location_trust",
        differ: "UPDATE location SET trusted_at = trusted_at + 1000 WHERE trusted_at IS NOT NULL",
        tables: &["location"],
    },
    Case {
        section: "readme_consent",
        differ: "UPDATE project SET readme_remote_at = readme_remote_at + 1000
                 WHERE readme_remote_at IS NOT NULL",
        tables: &["project"],
    },
    Case {
        section: "accounts",
        differ: "UPDATE account SET display_name = 'present', connected_at = connected_at + 1000;
                 UPDATE account_org SET is_enabled = 1 - is_enabled",
        tables: &["account", "account_org"],
    },
    Case {
        section: "parcels",
        differ: "UPDATE parcel SET check_result = 'missing', checked_at = 999;
                 UPDATE parcel_ref SET oid = 'present'",
        tables: &["parcel", "parcel_ref"],
    },
    Case {
        section: "removal_records",
        differ: "UPDATE removal_record SET ended_at = 999, disposal = 'gone_unconfirmed';
                 UPDATE removal_log SET log = 'present'",
        tables: &["removal_record", "removal_log"],
    },
];

fn section(name: &str) -> &'static SectionSpec {
    SECTIONS
        .iter()
        .find(|s| s.name == name)
        .unwrap_or_else(|| panic!("{name} is not registered"))
}

/// One table's rows, every column of each.
type Rows = Vec<Vec<Value>>;

/// Every row of `table`, every column, in key order.
fn rows_of(conn: &Connection, table: &str) -> Rows {
    let mut st = conn
        .prepare(&format!("SELECT * FROM {table} ORDER BY 1, 2"))
        .unwrap();
    let width = st.column_count();
    st.query_map([], |r| (0..width).map(|i| r.get::<_, Value>(i)).collect())
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

fn unhex(text: &str) -> Vec<u8> {
    text.as_bytes()
        .chunks(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect()
}

/// The location `key` names in `conn`, and the project holding it.
fn resolve(conn: &Connection, key: &SidecarLocationKey) -> Option<(LocationId, ProjectId)> {
    conn.query_row(
        "SELECT id, project_id FROM location WHERE kind = ?1 AND distro = ?2 AND path_key = ?3",
        rusqlite::params![key.kind, key.distro, unhex(&key.path_key)],
        |r| Ok((LocationId(r.get(0)?), ProjectId(r.get(1)?))),
    )
    .optional()
    .unwrap()
}

/// Plant the section's fixture, export it, make the present rows differ, restore the export over
/// them, and answer each table's rows before and after the restore.
fn restore_over_present(case: &Case) -> Vec<(Rows, Rows)> {
    let spec = section(case.section);
    let fixture = SECTION_FIXTURES
        .iter()
        .find(|(name, _)| *name == case.section)
        .map(|(_, fixture)| *fixture)
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mut index = Index::open_at(dir.path(), NOW).unwrap();
    index
        .with_tx(|tx| fixture(tx, &FixtureIds::default()))
        .unwrap();
    let exported = (spec.export)(index.conn()).unwrap();
    assert_ne!(
        exported.len(),
        0,
        "{}'s fixture exports nothing",
        case.section
    );
    index.conn().execute_batch(case.differ).unwrap();
    let before: Vec<_> = case
        .tables
        .iter()
        .map(|table| rows_of(index.conn(), table))
        .collect();

    index
        .with_tx(|tx| {
            for row in &exported {
                let resolved: Vec<_> = row
                    .location_keys
                    .iter()
                    .filter_map(|key| resolve(tx, key).map(|found| (key.clone(), found)))
                    .collect();
                let locations: Vec<_> = resolved
                    .iter()
                    .map(|(key, (location, _))| (key.clone(), *location))
                    .collect();
                // A row naming no location reaches its project through its subject alone.
                let by_subject = row
                    .subject
                    .as_deref()
                    .and_then(ProjectSubject::parse)
                    .and_then(|subject| resolve_subject_unique(tx, &subject).unwrap());
                let ctx = RestoreCtx {
                    now: NOW,
                    project: resolved
                        .first()
                        .map(|(_, (_, project))| *project)
                        .or(by_subject),
                    locations: &locations,
                    source_generation: 1,
                };
                (spec.restore)(tx, row, &ctx)?;
            }
            Ok(())
        })
        .unwrap();
    let after = case.tables.iter().map(|table| rows_of(index.conn(), table));
    before.into_iter().zip(after).collect()
}

/// **A write-once restore never overwrites a present row.** Each case restores its section's
/// export over an index that already holds a different value for the same subject, and every row
/// of the section's tables must come through unchanged.
#[test]
fn a_write_once_restore_never_overwrites_a_present_row() {
    let unlowered: Vec<&str> = SECTIONS
        .iter()
        .filter(|s| s.rule != RestoreRule::Replace)
        .map(|s| s.name)
        .collect();
    let cased: Vec<&str> = CASES.iter().map(|c| c.section).collect();
    assert_eq!(cased, unlowered, "a non-Replace section has no case here");

    let mut exercised = Vec::new();
    for case in CASES {
        for (table, (before, after)) in case.tables.iter().zip(restore_over_present(case)) {
            assert_ne!(before.len(), 0, "{}: {table} holds no row", case.section);
            assert_eq!(
                after, before,
                "{} lowered a present {table} row",
                case.section
            );
        }
        exercised.push(case.section);
    }
    eprintln!("sections exercised: {exercised:?}");
    assert_ne!(exercised.len(), 0, "no section was exercised");
}
