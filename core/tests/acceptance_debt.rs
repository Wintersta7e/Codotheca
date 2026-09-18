//! The `AC-P3-28-*` suite that does not live inside an implementation task's own file.
//!
//! The rest are named in their own files with their tag in the test name, so `TAG_P3` finds
//! them: `core/tests/debt_migration.rs` (`AC-P3-28-9`, `-10`, `-16`),
//! `core/tests/debt_lifecycle.rs` (`-1`, `-2`, `-3`, `-12`, `-14`, `-17`),
//! `core/tests/debt_producers.rs` (`-2`, `-3`, `-4`, `-11`, `-13`),
//! `core/tests/debt_xp.rs` (`-5`, `-15`), `core/tests/debt_merge.rs` (`-6`, `-7`, `-18`),
//! `core/tests/uninstall_command.rs` (`-8`).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use codotheca_core::debt::read::{load_debt, load_debt_sweeps};
use codotheca_core::index::migrate::{apply_all, MIGRATIONS};
use codotheca_core::index::{open_connection, Index};
use codotheca_core::protocol::{DebtSource, DecayLayer, ProjectId};

const SCHEMA: &str = include_str!("../../protocol/schema/protocol.json");

fn fresh() -> (tempfile::TempDir, rusqlite::Connection) {
    let dir = tempfile::tempdir().unwrap();
    let mut conn = open_connection(&Index::db_path(dir.path())).unwrap();
    apply_all(&mut conn, MIGRATIONS).unwrap();
    (dir, conn)
}

fn insert_project(conn: &rusqlite::Connection, name: &str) -> i64 {
    conn.execute(
        "INSERT INTO project (name, seed_basename, lineage_key, created_at, updated_at)
         VALUES (?1, ?1, 'abc123', 1, 1)",
        [name],
    )
    .unwrap();
    conn.last_insert_rowid()
}

/// **`AC-P3-28-11`.** *Not computed* and *zero* differ **on the wire**, not only in the store.
///
/// A project with **no `debt_sweep` row** for a source and a project with a `complete` sweep and
/// `item_count = 0` produce different `ProjectDetail` payloads: `debtSweeps` missing the source
/// versus carrying it with `itemCount: 0`. Both have an **empty item list**, which is exactly why
/// the item list alone cannot carry the distinction.
#[test]
fn ac_p3_28_11_never_observed_and_zero_differ_on_the_wire() {
    let (_d, conn) = fresh();
    let never = insert_project(&conn, "never-looked");
    let looked = insert_project(&conn, "looked-and-found-nothing");

    conn.execute(
        "INSERT INTO debt_sweep (project_id, source, outcome, basis, item_count, observed_at)
         VALUES (?1, 'todo_marker', 'complete', 'head', 0, 10)",
        [looked],
    )
    .unwrap();

    let never_items = load_debt(&conn, ProjectId(never)).unwrap();
    let looked_items = load_debt(&conn, ProjectId(looked)).unwrap();
    assert!(never_items.is_empty());
    assert!(
        looked_items.is_empty(),
        "the item lists must be identical, or this criterion is testing the wrong thing"
    );

    let never_sweeps = load_debt_sweeps(&conn, ProjectId(never)).unwrap();
    let looked_sweeps = load_debt_sweeps(&conn, ProjectId(looked)).unwrap();

    assert!(
        never_sweeps
            .iter()
            .all(|s| s.source != DebtSource::TodoMarker),
        "a project nobody swept carried a sweep row"
    );
    let found = looked_sweeps
        .iter()
        .find(|s| s.source == DebtSource::TodoMarker)
        .expect("the swept project must carry its row");
    assert_eq!(found.item_count, Some(0), "a measured zero read as unknown");

    assert_ne!(
        never_sweeps, looked_sweeps,
        "two different facts produced one payload"
    );
}

/// A sweep whose outcome did not observe carries **no count**, so an `unobservable` row and a
/// `complete, 0` row are also distinguishable on the wire.
#[test]
fn an_unobservable_sweep_carries_no_count_on_the_wire() {
    let (_d, conn) = fresh();
    let p = insert_project(&conn, "thing");
    conn.execute(
        "INSERT INTO debt_sweep (project_id, source, outcome, item_count, observed_at)
         VALUES (?1, 'missing_readme', 'unobservable', NULL, 10)",
        [p],
    )
    .unwrap();

    let sweeps = load_debt_sweeps(&conn, ProjectId(p)).unwrap();
    assert_eq!(sweeps.len(), 1);
    assert_eq!(sweeps[0].item_count, None);
}

/// The layer a `DebtItem` carries is **a join over §28.2's registry, not a stored column**, so it
/// cannot disagree with the source it describes — and the list is ordered by it.
#[test]
fn the_wire_layer_is_the_registrys_and_the_list_is_ordered_by_it() {
    let (_d, conn) = fresh();
    let p = insert_project(&conn, "thing");
    // Planted out of order: `ci_red` is `cracks` (3rd) and `missing_readme` is `dust` (0th).
    for source in ["ci_red", "missing_readme", "todo_marker"] {
        conn.execute(
            "INSERT INTO debt_item (project_id, subject_key, source, fingerprint, state, scoring,
                                    first_seen_at, last_seen_at)
             VALUES (?1, 'lineage:abc123|remote:', ?2, 'f', 'open', 'scored', 1, 1)",
            rusqlite::params![p, source],
        )
        .unwrap();
    }

    let items = load_debt(&conn, ProjectId(p)).unwrap();
    assert_eq!(items.len(), 3);
    assert_eq!(
        items.iter().map(|i| i.layer).collect::<Vec<_>>(),
        vec![DecayLayer::Dust, DecayLayer::Cracks, DecayLayer::Overgrowth],
        "the list is not in the schema's declared layer order"
    );
}

/// **The one value stated twice, and the test that keeps it one.** `read.rs`'s `layer_order`
/// mirrors the schema's declaration order, which is the total order A3's tie-break needs. This
/// reads the schema and asserts the rendered order agrees, so the two cannot drift silently.
#[test]
fn the_render_order_agrees_with_the_schemas_declaration_order() {
    let doc: serde_json::Value = serde_json::from_str(SCHEMA).unwrap();
    let declared: Vec<String> = doc["types"]["DecayLayer"]["variants"]
        .as_array()
        .expect("DecayLayer declares variants")
        .iter()
        .map(|v| v.as_str().unwrap().to_owned())
        .collect();
    assert_eq!(declared.len(), 5, "the schema moved and this test did not");

    // Plant one item per layer, through the source that carries it, and read the order back.
    let (_d, conn) = fresh();
    let p = insert_project(&conn, "thing");
    // dust · cobwebs · rust · cracks · overgrowth, in reverse so the sort has work to do.
    for source in [
        "todo_marker",
        "ci_red",
        "dependency_advisory",
        "abandoned_with_debt",
        "missing_readme",
    ] {
        conn.execute(
            "INSERT INTO debt_item (project_id, subject_key, source, fingerprint, state, scoring,
                                    first_seen_at, last_seen_at)
             VALUES (?1, 'lineage:abc123|remote:', ?2, 'f', 'open', 'scored', 1, 1)",
            rusqlite::params![p, source],
        )
        .unwrap();
    }

    let rendered: Vec<String> = load_debt(&conn, ProjectId(p))
        .unwrap()
        .iter()
        .map(|i| match serde_json::to_value(i.layer).unwrap() {
            serde_json::Value::String(raw) => raw,
            other => panic!("a generated enum serialised as {other}"),
        })
        .collect();
    assert_eq!(
        rendered, declared,
        "read.rs's render order and the schema's declaration order disagree"
    );
}
