//! [p3] §30's acceptance criteria whose evidence is the **core's** — the schema half of
//! `AC-P3-30-2` and the core half of `AC-P3-30-1`.
//!
//! Every check here prints what it scanned and **fails at zero**, and every count it can derive
//! it derives: none is written into a test.
#![cfg(feature = "testkit")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use codotheca_core::health::read_for_project;
use codotheca_core::health::switches::{read_switches, write_switches};
use codotheca_core::index::migrate::{apply_all, MIGRATIONS};
use codotheca_core::index::{open_connection, Index};
use codotheca_core::protocol::{
    CheckOutcome, DebtSource, HealthCheckSwitch, HealthState, ProjectId,
};

const NOW: i64 = 1_781_179_200;

fn store() -> (tempfile::TempDir, rusqlite::Connection) {
    let dir = tempfile::tempdir().unwrap();
    let mut conn = open_connection(&Index::db_path(dir.path())).unwrap();
    apply_all(&mut conn, MIGRATIONS).unwrap();
    (dir, conn)
}

fn seed_live(conn: &rusqlite::Connection) -> i64 {
    conn.execute(
        "INSERT INTO project (name, seed_basename, lineage_key, created_at, updated_at,
                              authored_by_user, is_reference, acknowledged_at)
         VALUES ('p', 'p', 'abc123', 1, 1, 1, 0, ?1)",
        [NOW],
    )
    .unwrap();
    let project = conn.last_insert_rowid();
    let path = format!("/p-{project}");
    conn.execute(
        "INSERT INTO location (project_id, kind, path_bytes, path_key, path_display, store_key,
                               presence, repo_kind, refstate_observed_at)
         VALUES (?1, 'linux', ?3, ?3, ?4, 'store-a', 'present', 'worktree', ?2)",
        rusqlite::params![project, NOW, path.as_bytes(), path],
    )
    .unwrap();
    project
}

fn schema() -> serde_json::Value {
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repo root");
    serde_json::from_str(
        &std::fs::read_to_string(repo.join("protocol/schema/protocol.json")).unwrap(),
    )
    .unwrap()
}

/// **`AC-P3-30-2`, the schema half.** Two units, never one: `scoredOpen` counts items and every
/// basis field counts checks. A type carrying one *value* over the two is what A12b forbids, and
/// a basis holding an item count is how that arrives.
#[test]
fn ac_p3_30_2_the_health_types_keep_items_and_checks_in_separate_fields() {
    let schema = schema();
    let basis = schema["types"]["HealthBasis"]["fields"]
        .as_object()
        .expect("HealthBasis is a declared struct");
    eprintln!(
        "AC-P3-30-2 scanned {} HealthBasis field(s): {:?}",
        basis.len(),
        basis.keys().collect::<Vec<_>>()
    );
    assert!(
        !basis.is_empty(),
        "a field audit over nothing proves nothing"
    );
    for name in basis.keys() {
        let lower = name.to_lowercase();
        for banned in ["open", "item", "unverified"] {
            assert!(
                !lower.contains(banned),
                "HealthBasis.{name} names {banned}: a basis counts checks, never items"
            );
        }
    }

    // And the two item counts live on the summary, where each carries its unit in its name.
    let summary = schema["types"]["HealthSummary"]["fields"]
        .as_object()
        .expect("HealthSummary is a declared struct");
    assert_eq!(summary["scoredOpen"], "u32?");
    assert_eq!(summary["unverified"], "u32?");
    assert_eq!(summary["unknownChecks"], "u32?");
    // A ratio, a percentage or a total would be a third field standing over the two units. There
    // is none. **`scoredOpen` is not one**: `scored` is A7's property of an item, so the field
    // names *what it counts*, which is the shape this rule is protecting rather than banning.
    for name in summary.keys() {
        let lower = name.to_lowercase();
        for banned in ["ratio", "percent", "pct", "total", "overall"] {
            assert!(
                !lower.contains(banned),
                "HealthSummary.{name} reads as a combined figure"
            );
        }
    }
}

/// **`AC-P3-30-1`, the core half.** With every check off, nothing is eligible, so the reading
/// carries no basis and no `scoredOpen` — **not a zeroed one**.
#[test]
fn ac_p3_30_1_with_every_check_off_a_reading_carries_no_figure() {
    let (_dir, conn) = store();
    let projects: Vec<i64> = (0..3).map(|_| seed_live(&conn)).collect();
    eprintln!("AC-P3-30-1 projects scanned: {}", projects.len());
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
    assert!(read_switches(&conn).unwrap().iter().all(|s| !s.enabled));

    for project in &projects {
        let (reading, _items) = read_for_project(&conn, ProjectId(*project)).unwrap();
        assert_eq!(reading.state, HealthState::Live);
        assert_eq!(
            reading.scored_open, None,
            "a zero was written for a reading with nothing eligible"
        );
        assert!(reading.basis.is_none(), "a zeroed basis was written");
        // Every check is still named, and every one of them is `off` — the tab renders the
        // switch rather than going silent.
        assert_eq!(reading.checks.len(), DebtSource::ALL.len());
        for check in &reading.checks {
            assert_eq!(check.outcome, CheckOutcome::Off, "{check:?}");
            assert_eq!(check.unknown_reason, None);
        }
    }
}

/// **`AC-P3-30-15`, the schema half.** §30 adds no command, no event and no topic — it moves the
/// type row alone.
#[test]
fn ac_p3_30_15_the_health_reading_adds_no_command_event_or_topic() {
    let schema = schema();
    let commands = schema["commands"].as_array().expect("commands").len();
    let topics = schema["topics"].as_object().expect("topics").len();
    let events: usize = schema["topics"]
        .as_object()
        .expect("topics")
        .values()
        .map(|t| t.as_object().map_or(0, serde_json::Map::len))
        .sum();
    eprintln!("AC-P3-30-15 surface: {commands} commands, {topics} topics, {events} events");
    assert!(commands > 0 && topics > 0 && events > 0);

    // No health-shaped name among them, which is what *"adds none"* means in practice.
    //
    // **`health.weathering` is the one exemption and it is §33.8's, not §30's.** §30.11 rules
    // that §30 owns the `health.*` prefix and occupies none of it, and §33 spends the first name
    // under it — `p3-00-index.md`'s delta table records that +1 against Δ33. The substring is a
    // *proxy* for the claim; the claim is *§30 declared no command*, and it is still asserted for
    // every other name. Exempting the one ruled declaration keeps the criterion; treating the
    // proxy as the claim would fail a tree that is correct.
    for command in schema["commands"].as_array().expect("commands") {
        let name = command["name"].as_str().unwrap_or_default().to_lowercase();
        if name == "health.weathering" {
            continue;
        }
        assert!(!name.contains("health"), "§30 declared a command: {name}");
    }
    // **`projects/health_delta` is the same kind of exemption, and it is §34.4's.** The delta table
    // records it against Δ34; §30 declares no event, and that is still asserted for every other.
    for (topic, entries) in schema["topics"].as_object().expect("topics") {
        assert!(!topic.to_lowercase().contains("health"));
        for event in entries.as_object().expect("events").keys() {
            if topic == "projects" && event == "health_delta" {
                continue;
            }
            assert!(
                !event.to_lowercase().contains("health"),
                "§30 declared an event: {topic}/{event}"
            );
        }
    }
}

/// **§36.1's one lettered id.** `AC-P3-30-11a` and `AC-P3-30-11` are different criteria and both
/// exist, so the tag grammar has to yield `P3-30-11a` for the first and not `P3-30-11`.
///
/// p3-36a's `tags.test.mjs` owns the grammar; this asserts the **names in this lane** resolve the
/// way §36.1 requires, because this plan is the one id §36.1 calls out by name.
#[test]
fn the_lettered_criterion_id_survives_this_lanes_test_names() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let mut lettered = Vec::new();
    let mut scanned = 0usize;
    for entry in std::fs::read_dir(&dir).expect("tests dir").flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "rs") {
            let text = std::fs::read_to_string(&path).unwrap();
            scanned += 1;
            for line in text.lines() {
                if let Some(rest) = line.trim().strip_prefix("fn ac_p3_30_11a") {
                    lettered.push(format!(
                        "{}::ac_p3_30_11a{}",
                        path.file_name().unwrap_or_default().to_string_lossy(),
                        rest.split('(').next().unwrap_or_default()
                    ));
                }
            }
        }
    }
    eprintln!("scanned {scanned} core test file(s); lettered names: {lettered:?}");
    assert!(scanned > 0, "a name audit over no files proves nothing");
    assert!(
        !lettered.is_empty(),
        "no test carries AC-P3-30-11a, so the lettered id is untested"
    );
    // The guard the grammar relies on: the character after the letter must not continue the id.
    for name in &lettered {
        let tail = name.split("ac_p3_30_11a").nth(1).unwrap_or_default();
        assert!(
            tail.is_empty() || tail.starts_with('_'),
            "{name} continues the id past the letter"
        );
    }
}
