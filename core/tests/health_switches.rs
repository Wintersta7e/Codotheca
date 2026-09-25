//! §30.9 — one switch per check, defaulting **on**, and **off hides without ever closing**.
#![cfg(feature = "testkit")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

#[path = "support/detail_rig.rs"]
mod detail_rig;

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

    let identity = |db: &rusqlite::Connection| -> Vec<(i64, String, String, i64)> {
        let mut st = db
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

/// `AC-P3-30-1`'s core half, **on the shelf**: with every check off, every project's summary —
/// batched and single — is `absent` with every quantity NULL, and the page is handed no item to
/// list or to light a layer from. The page's reading is `acceptance_p3_health.rs`'s.
///
/// Each project would read `live` with its checks on, and carries an open item, so a gate that
/// answers `absent` for another reason, or a list that keeps the item, fails here.
#[test]
fn ac_p3_30_1_with_every_check_off_every_reading_is_absent_with_scored_open_and_basis_null() {
    use codotheca_core::health::read_for_project;
    use codotheca_core::health::summary::{summaries_for_all, summary_for};
    use codotheca_core::protocol::{HealthState, ProjectId};

    let (_dir, conn) = fresh();
    let projects: Vec<i64> = (0..3)
        .map(|n| {
            let project = insert_project(&conn);
            let path = format!("/p-{project}");
            conn.execute(
                "UPDATE project SET authored_by_user = 1, acknowledged_at = ?2 WHERE id = ?1",
                rusqlite::params![project, NOW],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO location (project_id, kind, path_bytes, path_key, path_display,
                                       store_key, presence, repo_kind, refstate_observed_at)
                 VALUES (?1, 'linux', ?3, ?3, ?4, 'store-a', 'present', 'worktree', ?2)",
                rusqlite::params![project, NOW, path.as_bytes(), path],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO debt_sweep (project_id, source, outcome, item_count, observed_at)
                 VALUES (?1, 'missing_readme', 'complete', 1, ?2)",
                rusqlite::params![project, NOW],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO debt_item (project_id, subject_key, source, fingerprint, state,
                                        scoring, first_seen_at, last_seen_at)
                 VALUES (?1, ?3, 'missing_readme', '', 'open', 'scored', ?2, ?2)",
                rusqlite::params![project, NOW, format!("lineage:p{n}|remote:")],
            )
            .unwrap();
            project
        })
        .collect();
    eprintln!("projects scanned with every check off: {}", projects.len());
    assert!(
        !projects.is_empty(),
        "a scan over no projects proves nothing"
    );

    // The control: with every check on, each is a `live` reading with a figure on the shelf.
    for project in &projects {
        let (summary, _) = summary_for(&conn, ProjectId(*project)).unwrap();
        assert_eq!(summary.state, HealthState::Live, "project {project}");
        assert_eq!(summary.scored_open, Some(1));
    }

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
    assert!(read_switches(&conn).unwrap().iter().all(|s| !s.enabled));

    let batched = summaries_for_all(&conn).unwrap();
    for project in &projects {
        let (single, _) = summary_for(&conn, ProjectId(*project)).unwrap();
        assert_eq!(
            batched.get(project).map(|s| &s.0),
            Some(&single),
            "project {project}: the two shelf paths disagree"
        );
        assert_eq!(
            single.state,
            HealthState::Absent,
            "project {project}: nothing eligible is not a reading"
        );
        assert_eq!(single.scored_open, None, "a zero was written for unknown");
        assert_eq!(single.unverified, None);
        assert_eq!(single.unknown_checks, None);
        assert_eq!(single.observed_at, None);

        let (reading, items) = read_for_project(&conn, ProjectId(*project)).unwrap();
        assert_eq!(reading.state, HealthState::Absent);
        assert!(reading.basis.is_none(), "a zeroed basis was written");
        assert!(
            items.is_empty(),
            "project {project}: a switched-off check's item was handed to the page"
        );
    }
}

/// §30.9 — **the check's items leave the list.** A switched-off check's items leave the page's
/// item list, which is also the list §33 lights layers from, and the shelf's `unverified`; they
/// come back with their identity when the check does. Hidden, never closed, never paid.
#[test]
fn a_switched_off_checks_items_leave_the_page_list_and_come_back_unchanged() {
    use codotheca_core::detail::get::handle_project_get;
    use codotheca_core::health::summary::{summaries_for_all, summary_for};
    use codotheca_core::protocol::{DebtItem, ProjectId};

    let rig = detail_rig::Rig::new();
    rig.project(1, "alpha");
    rig.location(1, 1, "/a", "present", Some("main"), Some(0), Some(NOW));
    let conn = rig.conn();
    conn.execute("UPDATE project SET authored_by_user = 1 WHERE id = 1", [])
        .unwrap();
    seed_sweep_and_item(conn, 1, "missing_readme");
    seed_sweep_and_item(conn, 1, "missing_tests");
    conn.execute(
        "INSERT INTO debt_item (project_id, subject_key, source, fingerprint, state, scoring,
                                first_seen_at, last_seen_at)
         VALUES (1, 'lineage:abc123|remote:', 'missing_tests', 'u', 'unverified', 'scored',
                 ?1, ?1)",
        [NOW],
    )
    .unwrap();

    let count = |table: &str| -> i64 {
        conn.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .unwrap()
    };
    let identity = |items: &[DebtItem]| -> Vec<(DebtSource, String, i64)> {
        let mut out: Vec<_> = items
            .iter()
            .map(|i| (i.source, i.fingerprint.clone(), i.first_seen_at))
            .collect();
        out.sort_by(|a, b| format!("{a:?}").cmp(&format!("{b:?}")));
        out
    };
    let page = || handle_project_get(&rig.ctx(), serde_json::json!({ "id": 1 })).unwrap();
    let unverified = || summary_for(conn, ProjectId(1)).unwrap().0.unverified;

    // The control: every check on, all three items listed and the unverified one counted.
    let on = page();
    assert_eq!(on.debt.len(), 3, "seeded items missing from the page");
    assert_eq!(unverified(), Some(1));
    let (items_before, xp_before) = (count("debt_item"), count("xp_events"));

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

    let off = page();
    let sources: Vec<DebtSource> = off.debt.iter().map(|i| i.source).collect();
    eprintln!(
        "page items switched on: {}, switched off: {} {sources:?}; unverified off: {:?}; \
         debt_item rows {items_before} -> {}, xp_events {xp_before} -> {}",
        on.debt.len(),
        off.debt.len(),
        unverified(),
        count("debt_item"),
        count("xp_events")
    );
    assert_eq!(
        sources,
        vec![DebtSource::MissingReadme],
        "a switched-off check's items are still on the page"
    );
    assert_eq!(
        unverified(),
        Some(0),
        "the shelf still counts a hidden item"
    );
    assert_eq!(
        summaries_for_all(conn)
            .unwrap()
            .get(&1)
            .map(|s| s.0.unverified),
        Some(Some(0))
    );
    assert_eq!(count("debt_item"), items_before, "off closed an item");
    assert_eq!(count("xp_events"), xp_before, "off paid");

    toggle(true);
    let back = page();
    assert_eq!(
        identity(&back.debt),
        identity(&on.debt),
        "the items came back with a different identity"
    );
}

// ---------------------------------------------------------------------------------------------
// §30.9 — **a switched-off source is not swept.** Switch-off deletes its `debt_sweep` row, and a
// settle while it is off must not write one back, or *"a check switched back on is `unknown`
// until it next runs"* holds only if nothing settled in between.
// ---------------------------------------------------------------------------------------------

fn presence_answers(conn: &rusqlite::Connection, project: i64, tests: &str) {
    conn.execute(
        "INSERT INTO project_content_scan
            (project_id, head_oid, complete_head_oid, blobs_total, blobs_pending,
             predicate_version, has_readme, has_license, has_tests, has_ci,
             presence_observed_at, enumerated_at, completed_at)
         VALUES (?1, 'head0000', 'head0000', 4, 0, 1, 'present', 'present', ?2, 'present',
                 1, 1, 1)
         ON CONFLICT(project_id) DO UPDATE SET has_tests = excluded.has_tests",
        rusqlite::params![project, tests],
    )
    .unwrap();
}

/// The settle both runners call, in its own transaction.
fn settle(conn: &mut rusqlite::Connection, project: i64, at: i64) {
    let tx = conn.transaction().unwrap();
    codotheca_core::debt::singletons::settle_singletons(
        &tx,
        codotheca_core::protocol::ProjectId(project),
        at,
        0,
        &codotheca_core::debt::store::SqliteDebtStore,
    )
    .unwrap();
    tx.commit().unwrap();
}

fn tests_sweep(conn: &rusqlite::Connection, project: i64) -> Option<(String, i64)> {
    use rusqlite::OptionalExtension as _;
    conn.query_row(
        "SELECT outcome, observed_at FROM debt_sweep
          WHERE project_id = ?1 AND source = 'missing_tests'",
        [project],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .optional()
    .unwrap()
}

fn tests_items(conn: &rusqlite::Connection, project: i64) -> i64 {
    conn.query_row(
        "SELECT count(*) FROM debt_item WHERE project_id = ?1 AND source = 'missing_tests'",
        [project],
        |r| r.get(0),
    )
    .unwrap()
}

#[test]
fn a_switched_off_source_is_not_swept_and_reads_not_run_yet_once_back_on() {
    use codotheca_core::health::read_for_project;
    use codotheca_core::protocol::{CheckOutcome, ProjectId, UnknownReason};

    let (_dir, mut conn) = fresh();
    let p = insert_project(&conn);
    let path = format!("/p-{p}");
    conn.execute(
        "UPDATE project SET authored_by_user = 1, acknowledged_at = ?2 WHERE id = ?1",
        rusqlite::params![p, NOW],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO location (project_id, kind, path_bytes, path_key, path_display, store_key,
                               presence, repo_kind, refstate_observed_at)
         VALUES (?1, 'linux', ?3, ?3, ?4, 'store-a', 'present', 'worktree', ?2)",
        rusqlite::params![p, NOW, path.as_bytes(), path],
    )
    .unwrap();
    let toggle = |db: &rusqlite::Connection, enabled: bool| {
        let tx = db.unchecked_transaction().unwrap();
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

    // The control: switched on, tests present, a settle sweeps the source and finds nothing.
    presence_answers(&conn, p, "present");
    settle(&mut conn, p, NOW);
    assert_eq!(
        tests_sweep(&conn, p).map(|s| s.0).as_deref(),
        Some("complete"),
        "the control swept nothing"
    );

    toggle(&conn, false);
    assert_eq!(tests_sweep(&conn, p), None, "switch-off kept the sweep row");
    settle(&mut conn, p, NOW + 60);
    eprintln!(
        "after a settle with missing_tests off: sweep row {:?}",
        tests_sweep(&conn, p)
    );
    assert_eq!(
        tests_sweep(&conn, p),
        None,
        "a settle swept a switched-off source"
    );

    // Back on: `unknown`, owed *this has not run yet* — never the `ok` a re-written row would
    // claim — until the next real sweep.
    toggle(&conn, true);
    let (reading, _) = read_for_project(&conn, ProjectId(p)).unwrap();
    let check = reading
        .checks
        .iter()
        .find(|c| c.id == DebtSource::MissingTests)
        .expect("a declared source");
    eprintln!(
        "switched back on: {:?} {:?}",
        check.outcome, check.unknown_reason
    );
    assert_eq!(check.outcome, CheckOutcome::Unknown);
    assert_eq!(check.unknown_reason, Some(UnknownReason::NotRunYet));
    settle(&mut conn, p, NOW + 120);
    assert_eq!(
        tests_sweep(&conn, p),
        Some(("complete".to_owned(), NOW + 120)),
        "the next real sweep did not run"
    );

    // With an item open: a settle while the check is off opens and closes nothing, even when
    // what it would observe would close the item.
    presence_answers(&conn, p, "absent");
    settle(&mut conn, p, NOW + 180);
    assert_eq!(tests_items(&conn, p), 1, "the control opened nothing");
    toggle(&conn, false);
    presence_answers(&conn, p, "present");
    settle(&mut conn, p, NOW + 240);
    eprintln!(
        "item case, after a settle with missing_tests off: sweep row {:?}, items {}",
        tests_sweep(&conn, p),
        tests_items(&conn, p)
    );
    assert_eq!(tests_sweep(&conn, p), None);
    assert_eq!(
        tests_items(&conn, p),
        1,
        "a settle closed a switched-off source's item"
    );

    // Back on with the item surviving: §30.3 wins — *an item observed is an item*, so the check
    // is `failed` without waiting on a sweep. `failed` is neither the `ok` nor the `0` §30.9
    // guards against; `notRunYet` is the case above, where no item survived.
    toggle(&conn, true);
    let (item_reading, _) = read_for_project(&conn, ProjectId(p)).unwrap();
    let item_check = item_reading
        .checks
        .iter()
        .find(|c| c.id == DebtSource::MissingTests)
        .expect("a declared source");
    eprintln!(
        "item case, switched back on: {:?} {:?}, sweep row {:?}",
        item_check.outcome,
        item_check.unknown_reason,
        tests_sweep(&conn, p)
    );
    assert_eq!(tests_sweep(&conn, p), None, "nothing has swept it yet");
    assert_eq!(item_check.outcome, CheckOutcome::Failed);
    assert_eq!(item_check.unknown_reason, None);
}
