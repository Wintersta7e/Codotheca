//! §30.9 — one switch per check, defaulting **on**, and **off hides without ever closing**.
#![cfg(feature = "testkit")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use codotheca_core::health::switches::{
    read_switches, switch_key, write_switches, SWITCH_KEY_PREFIX,
};
use codotheca_core::index::migrate::{apply_all, MIGRATIONS};
use codotheca_core::index::{open_connection, Index};
use codotheca_core::protocol::{DebtSource, HealthCheckSwitch};

const NOW: i64 = 1_781_179_200;

fn fresh() -> (tempfile::TempDir, rusqlite::Connection) {
    let dir = tempfile::tempdir().unwrap();
    let mut conn = open_connection(&Index::db_path(dir.path())).unwrap();
    apply_all(&mut conn, MIGRATIONS).unwrap();
    (dir, conn)
}

/// The variants the **schema** declares, read from the schema rather than restated — so a tenth
/// source fails here rather than being discovered by a user whose check has no switch.
fn declared_sources() -> Vec<String> {
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repo root");
    let schema: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(repo.join("protocol/schema/protocol.json")).unwrap(),
    )
    .unwrap();
    schema["types"]["DebtSource"]["variants"]
        .as_array()
        .expect("DebtSource is a declared enum")
        .iter()
        .map(|v| v.as_str().expect("a variant is a string").to_owned())
        .collect()
}

fn insert_project(conn: &rusqlite::Connection) -> i64 {
    conn.execute(
        "INSERT INTO project (name, seed_basename, lineage_key, created_at, updated_at)
         VALUES ('p', 'p', 'abc123', 1, 1)",
        [],
    )
    .unwrap();
    conn.last_insert_rowid()
}

fn seed_sweep_and_item(conn: &rusqlite::Connection, project: i64, source: &str) {
    conn.execute(
        "INSERT INTO debt_sweep (project_id, source, outcome, item_count, observed_at)
         VALUES (?1, ?2, 'complete', 1, ?3)",
        rusqlite::params![project, source, NOW],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO debt_item (project_id, subject_key, source, fingerprint, state, scoring,
                                first_seen_at, last_seen_at)
         VALUES (?1, 'lineage:abc123|remote:', ?2, '', 'open', 'scored', ?3, ?3)",
        rusqlite::params![project, source, NOW],
    )
    .unwrap();
}

/// §30.9's key set, **derived from the generated enum** and enumerable with the fixed prefix.
#[test]
fn ac_p3_30_12_app_meta_holds_one_health_check_key_per_debt_source_variant() {
    let declared = declared_sources();
    eprintln!(
        "DebtSource variants declared by the schema: {}",
        declared.len()
    );
    assert!(
        !declared.is_empty(),
        "a switch test over no variants proves nothing"
    );

    // The runtime list the core emits one entry from, against the schema's own list.
    assert_eq!(
        DebtSource::ALL.len(),
        declared.len(),
        "the generated ALL list is the schema's own"
    );

    let (_dir, conn) = fresh();
    let switches = read_switches(&conn).unwrap();
    assert_eq!(switches.len(), declared.len(), "one entry per variant");

    let mut keys: Vec<String> = Vec::new();
    for switch in &switches {
        let key = switch_key(switch.check).unwrap();
        assert!(key.starts_with(SWITCH_KEY_PREFIX), "{key}");
        keys.push(key);
    }
    let mut expected: Vec<String> = declared
        .iter()
        .map(|v| format!("{SWITCH_KEY_PREFIX}{v}"))
        .collect();
    expected.sort();
    let mut got = keys.clone();
    got.sort();
    assert_eq!(got, expected, "the key set is the variant set, spelled out");

    // Written keys enumerate with the prefix, which is what the criterion asks of the store.
    {
        let tx = conn.unchecked_transaction().unwrap();
        write_switches(&tx, &switches).unwrap();
        tx.commit().unwrap();
    }
    let stored: i64 = conn
        .query_row(
            "SELECT count(*) FROM app_meta WHERE k LIKE 'health_check.%'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    eprintln!("health_check.* keys enumerated from app_meta: {stored}");
    assert_eq!(usize::try_from(stored).unwrap(), declared.len());
}

/// An **absent key is on**, on `roast_enabled`'s existing precedent. A stored `0` is the only
/// thing that turns a check off.
#[test]
fn ac_p3_30_12_every_key_defaults_on() {
    let (_dir, conn) = fresh();
    let switches = read_switches(&conn).unwrap();
    assert!(
        switches.iter().all(|s| s.enabled),
        "an absent key must read as on"
    );

    {
        let tx = conn.unchecked_transaction().unwrap();
        write_switches(
            &tx,
            &[HealthCheckSwitch {
                check: DebtSource::MissingTests,
                enabled: false,
            }],
        )
        .unwrap();
        tx.commit().unwrap();
    }
    let after = read_switches(&conn).unwrap();
    for switch in &after {
        assert_eq!(
            switch.enabled,
            switch.check != DebtSource::MissingTests,
            "{:?} moved and should not have",
            switch.check
        );
    }
}

/// **Off hides; it never closes, and it never pays.** A switch that closed items would make
/// turning a check off an XP source, and closing an item because the user stopped looking is not
/// a state transition of the project.
#[test]
fn ac_p3_30_12_switching_a_check_off_closes_no_debt_item_and_writes_no_xp_events_row() {
    let (_dir, conn) = fresh();
    let project = insert_project(&conn);
    seed_sweep_and_item(&conn, project, "missing_tests");

    let count = |table: &str| -> i64 {
        conn.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .unwrap()
    };
    let items_before = count("debt_item");
    let xp_before = count("xp_events");
    assert!(
        items_before > 0,
        "seeded nothing: the test would prove none"
    );

    {
        let tx = conn.unchecked_transaction().unwrap();
        write_switches(
            &tx,
            &[HealthCheckSwitch {
                check: DebtSource::MissingTests,
                enabled: false,
            }],
        )
        .unwrap();
        tx.commit().unwrap();
    }

    let items_after = count("debt_item");
    let xp_after = count("xp_events");
    eprintln!("debt_item {items_before} -> {items_after}, xp_events {xp_before} -> {xp_after}");
    assert_eq!(items_after, items_before, "an off switch closed an item");
    assert_eq!(xp_after, xp_before, "an off switch paid");
}

/// §30.9 — **a check switched back on is `unknown` until it next runs, not `ok` and not `0`**,
/// and the item it was about keeps its identity across the whole toggle.
#[test]
fn ac_p3_30_12_switching_a_check_back_on_is_unknown_not_ok_and_not_zero_and_the_item_keeps_its_identity(
) {
    let (_dir, conn) = fresh();
    let project = insert_project(&conn);
    seed_sweep_and_item(&conn, project, "missing_tests");

    let identity = |conn: &rusqlite::Connection| -> Vec<(i64, String, String, i64)> {
        let mut st = conn
            .prepare("SELECT id, source, fingerprint, first_seen_at FROM debt_item ORDER BY id")
            .unwrap();
        let mapped = st
            .query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, i64>(3)?,
                ))
            })
            .unwrap();
        mapped.map(Result::unwrap).collect()
    };
    let before = identity(&conn);

    let toggle = |enabled: bool| {
        let tx = conn.unchecked_transaction().unwrap();
        write_switches(
            &tx,
            &[HealthCheckSwitch {
                check: DebtSource::MissingTests,
                enabled,
            }],
        )
        .unwrap();
        tx.commit().unwrap();
    };
    toggle(false);
    toggle(true);

    assert_eq!(identity(&conn), before, "the item's identity moved");

    // No sweep row, so the check reads as `unknown` with reason `notRunYet` — never `ok`, and
    // never a zero, which is what a surviving `complete` row with `item_count = 0` would produce.
    let sweeps: i64 = conn
        .query_row(
            "SELECT count(*) FROM debt_sweep WHERE source = 'missing_tests'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        sweeps, 0,
        "an observation claim survived a period the check was off"
    );
    assert!(read_switches(&conn)
        .unwrap()
        .iter()
        .any(|s| s.check == DebtSource::MissingTests && s.enabled));
}

/// `AC-P3-30-1`'s core half: with **every** check off, no check is eligible, so the reading has
/// no basis and no `scoredOpen` — not a zeroed one.
#[test]
fn ac_p3_30_1_with_every_check_off_every_reading_is_absent_with_scored_open_and_basis_null() {
    use codotheca_core::health::outcome::{
        basis_over, outcome_for, CheckObservation, SweepFacts, SwitchState,
    };
    use codotheca_core::health::reason::{check_for, GrantState};
    use codotheca_core::protocol::{CheckOutcome, Presence};

    let (_dir, conn) = fresh();
    let projects: Vec<i64> = (0..3).map(|_| insert_project(&conn)).collect();
    eprintln!("projects scanned with every check off: {}", projects.len());
    assert!(
        !projects.is_empty(),
        "a scan over no projects proves nothing"
    );

    let all_off: Vec<HealthCheckSwitch> = DebtSource::ALL
        .into_iter()
        .map(|check| HealthCheckSwitch {
            check,
            enabled: false,
        })
        .collect();
    {
        let tx = conn.unchecked_transaction().unwrap();
        write_switches(&tx, &all_off).unwrap();
        tx.commit().unwrap();
    }
    let switches = read_switches(&conn).unwrap();
    assert!(switches.iter().all(|s| !s.enabled));

    for _project in &projects {
        let mut entries = Vec::new();
        for switch in &switches {
            let facts = SweepFacts {
                outcome: None,
                scored_open: 0,
                unverified: 0,
                observed_at: None,
            };
            let state = SwitchState {
                enabled: switch.enabled,
                grant_missing: false,
                not_applicable: false,
            };
            assert_eq!(outcome_for(&facts, &state), CheckOutcome::Off);
            entries.push(CheckObservation {
                check: check_for(
                    switch.check,
                    &facts,
                    &state,
                    Presence::Present,
                    &GrantState::default(),
                ),
                observed_at: None,
            });
        }
        assert!(
            basis_over(&entries).is_none(),
            "every check off must produce no basis, never a zeroed one"
        );
    }
}
