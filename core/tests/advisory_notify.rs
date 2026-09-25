#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! §32.12's **one** notification: five conjuncts, a seeding ledger, and **no token naming an
//! absence**.
//!
//! The contract's footer states a ceiling — `THREE NOTIFICATIONS EXIST · NONE MENTIONS ABSENCE` —
//! and this is the first thing that may fire against it. A field naming an excluded count would
//! breach it by addition rather than by wording, which is why the payload's fields are asserted
//! against a forbidden list and not merely read.

use codotheca_core::advisories::notify::{notifiable, seed_notified};
use codotheca_core::index::migrate::{apply_all, MIGRATIONS};
use codotheca_core::index::{open_connection, Index};
use codotheca_core::protocol::{AdvisoryAlert, ProjectId};

const NOW: i64 = 1_800_000_000;

fn fresh() -> (tempfile::TempDir, rusqlite::Connection) {
    let dir = tempfile::tempdir().unwrap();
    let mut conn = open_connection(&Index::db_path(dir.path())).unwrap();
    apply_all(&mut conn, MIGRATIONS).unwrap();
    (dir, conn)
}

/// A project and one copy of it, in one of the five presence states this test needs.
fn project(conn: &rusqlite::Connection, name: &str, presence: &str, removed: bool) -> ProjectId {
    conn.execute(
        "INSERT INTO project (name, seed_basename, lineage_key, created_at, updated_at)
         VALUES (?1, ?1, ?1, 1, 1)",
        [name],
    )
    .unwrap();
    let id = ProjectId(conn.last_insert_rowid());
    conn.execute(
        "INSERT INTO location (project_id, kind, path_bytes, path_key, path_display, store_key,
                               presence, repo_kind, removed_at)
         VALUES (?1, 'linux', ?2, ?2, ?3, 'store', ?4, 'worktree', ?5)",
        rusqlite::params![
            id.0,
            name.as_bytes(),
            name,
            presence,
            removed.then_some(NOW)
        ],
    )
    .unwrap();
    id
}

/// A project that was never cloned: a row with no location at all.
fn never_cloned(conn: &rusqlite::Connection, name: &str) -> ProjectId {
    conn.execute(
        "INSERT INTO project (name, seed_basename, lineage_key, created_at, updated_at)
         VALUES (?1, ?1, ?1, 1, 1)",
        [name],
    )
    .unwrap();
    ProjectId(conn.last_insert_rowid())
}

struct Advisory<'a> {
    id: &'a str,
    severity: &'a str,
    fix: bool,
    cve: Option<&'a str>,
}

fn vulnerable(conn: &rusqlite::Connection, project: ProjectId, package: &str, adv: &Advisory<'_>) {
    conn.execute(
        "INSERT INTO project_dependency
           (project_id, ecosystem, package_name, version, source_path, observed_at)
         VALUES (?1, 'npm', ?2, '1.0.0', 'lock', ?3) ON CONFLICT DO NOTHING",
        rusqlite::params![project.0, package, NOW],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO advisory_sweep (started_at, settled_at, outcome, complete)
         VALUES (?1, ?1, 'done', 1)",
        [NOW],
    )
    .unwrap();
    let sweep = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO advisory_triple
           (ecosystem, package_name, version, sweep_id, observed_at, answered)
         VALUES ('npm', ?1, '1.0.0', ?2, ?3, 1) ON CONFLICT DO NOTHING",
        rusqlite::params![package, sweep, NOW],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO advisory (advisory_id, severity, summary, url, observed_at)
         VALUES (?1, ?2, 's', 'u', ?3) ON CONFLICT DO NOTHING",
        rusqlite::params![adv.id, adv.severity, NOW],
    )
    .unwrap();
    if let Some(cve) = adv.cve {
        conn.execute(
            "INSERT INTO advisory_cve (advisory_id, cve_id) VALUES (?1, ?2)
             ON CONFLICT DO NOTHING",
            rusqlite::params![adv.id, cve],
        )
        .unwrap();
    }
    conn.execute(
        "INSERT INTO advisory_match
           (ecosystem, package_name, version, advisory_id, fix_available, fixed_version)
         VALUES ('npm', ?1, '1.0.0', ?2, ?3, ?4) ON CONFLICT DO NOTHING",
        rusqlite::params![
            package,
            adv.id,
            i64::from(adv.fix),
            adv.fix.then_some("2.0.0")
        ],
    )
    .unwrap();
}

fn fire(conn: &mut rusqlite::Connection) -> Option<AdvisoryAlert> {
    let tx = conn.transaction().unwrap();
    // Nothing suppressed: these fixtures test the other four conjuncts.
    let alert = notifiable(&tx, NOW, &|_| false).unwrap();
    tx.commit().unwrap();
    alert
}

const fn critical(id: &str) -> Advisory<'_> {
    Advisory {
        id,
        severity: "critical",
        fix: true,
        cve: Some("CVE-2026-0001"),
    }
}

/// **AC-P3-32-12.** It fires **once and only once** per `(project, advisory)`, and **zero** times
/// on a project's first computation.
#[test]
fn ac_p3_32_12_it_fires_once_ever_and_never_on_a_first_computation() {
    let (_d, mut conn) = fresh();
    let alpha = project(&conn, "alpha", "present", false);
    vulnerable(&conn, alpha, "left", &critical("GHSA-a"));
    vulnerable(&conn, alpha, "right", &critical("GHSA-b"));

    // The seeding pass: it writes the ledger and fires nothing. Without it, first run toasts once
    // per critical advisory across the whole library.
    let seeded = {
        let tx = conn.transaction().unwrap();
        let n = seed_notified(&tx, alpha, NOW).unwrap();
        tx.commit().unwrap();
        n
    };
    eprintln!("advisory_notify: {seeded} pair(s) seeded, 0 fired");
    assert_eq!(seeded, 2);
    assert!(
        fire(&mut conn).is_none(),
        "a first computation fires nothing"
    );

    // A new advisory arrives afterwards: that one fires, once.
    vulnerable(&conn, alpha, "third", &critical("GHSA-c"));
    let first = fire(&mut conn).expect("a new critical advisory fires");
    assert_eq!(first.advisory_id.as_deref(), Some("GHSA-c"));
    assert_eq!(first.project_count, 1);
    assert!(fire(&mut conn).is_none(), "once ever, per pair");
}

/// **AC-P3-32-13.** **No** notification fires for a project in any of the four non-installed
/// states, and the payload carries **no token naming an absence**.
#[test]
fn ac_p3_32_13_the_four_non_installed_states_fire_nothing() {
    let (_d, mut conn) = fresh();
    let offline = project(&conn, "offline", "offline", false);
    let missing = project(&conn, "missing", "missing", false);
    let uninstalled = project(&conn, "uninstalled", "present", true);
    let not_cloned = never_cloned(&conn, "not-cloned");
    for (i, p) in [offline, missing, uninstalled, not_cloned]
        .into_iter()
        .enumerate()
    {
        vulnerable(
            &conn,
            p,
            &format!("pkg{i}"),
            &critical(&format!("GHSA-{i}")),
        );
    }
    assert!(
        fire(&mut conn).is_none(),
        "none of the four non-installed states may interrupt"
    );

    // **`removed_at` takes precedence over `presence` on every surface**: a copy marked present
    // and removed is uninstalled.
    let installed = project(&conn, "installed", "present", false);
    vulnerable(&conn, installed, "live", &critical("GHSA-live"));
    let alert = fire(&mut conn).expect("an installed project does fire");

    // The payload's own field names and string values, against the forbidden list. **A scan over
    // zero fields fails.**
    let json = serde_json::to_value(&alert).unwrap();
    let object = json.as_object().expect("the payload is an object");
    let mut scanned = 0usize;
    for (name, value) in object {
        scanned += 1;
        let haystack = format!("{name} {value}").to_lowercase();
        for banned in [
            "excluded",
            "suppressed",
            "skipped",
            "hidden",
            "uninstalled",
            "offline",
            "missing",
            "none",
            "no ",
        ] {
            assert!(
                !haystack.contains(banned),
                "the payload names an absence: {name} = {value}"
            );
        }
    }
    eprintln!("advisory_notify: {scanned} payload field(s) scanned for absence tokens");
    assert_eq!(scanned, 7, "a scan over zero fields proves nothing");
}

/// A critical advisory with **no** CVE id is still notifiable: the notifiable unit is the
/// advisory, and *"a critical CVE"* taken literally is a rule about identifier bookkeeping wearing
/// the costume of a rule about severity.
#[test]
fn an_advisory_with_no_cve_id_still_fires() {
    let (_d, mut conn) = fresh();
    let alpha = project(&conn, "alpha", "present", false);
    vulnerable(
        &conn,
        alpha,
        "left",
        &Advisory {
            id: "GHSA-nocve",
            severity: "critical",
            fix: true,
            cve: None,
        },
    );
    let alert = fire(&mut conn).expect("fires");
    assert_eq!(alert.advisory_id.as_deref(), Some("GHSA-nocve"));
    assert_eq!(alert.cve_id, None, "nullable display field, not a gate");
}

/// **Three projects firing in one settle produce ONE payload** with `projectCount: 3` and every
/// nullable field null; one project produces the five `Some` fields.
#[test]
fn several_projects_produce_one_payload_that_names_none_of_them() {
    let (_d, mut conn) = fresh();
    for i in 0..3 {
        let p = project(&conn, &format!("p{i}"), "present", false);
        vulnerable(
            &conn,
            p,
            &format!("pkg{i}"),
            &critical(&format!("GHSA-{i}")),
        );
    }
    let alert = fire(&mut conn).expect("fires");
    eprintln!(
        "advisory_notify: projectCount={} advisoryCount={}",
        alert.project_count, alert.advisory_count
    );
    assert_eq!(alert.project_count, 3);
    assert_eq!(alert.advisory_count, 3);
    assert_eq!(alert.project_id, None);
    assert_eq!(alert.advisory_id, None);
    assert_eq!(alert.cve_id, None);
    assert_eq!(alert.package_name, None);
    assert_eq!(alert.ecosystem, None);
}

/// A `surface_suppressed` project never fires. **The predicate is §30's and arrives as a
/// parameter**, so this section holds no second copy of it that could disagree with the first.
#[test]
fn a_suppressed_project_never_fires() {
    let (_d, mut conn) = fresh();
    let alpha = project(&conn, "alpha", "present", false);
    vulnerable(&conn, alpha, "left", &critical("GHSA-a"));

    let tx = conn.transaction().unwrap();
    let suppressed = notifiable(&tx, NOW, &|p: ProjectId| p == alpha).unwrap();
    tx.commit().unwrap();
    assert!(
        suppressed.is_none(),
        "suppression gates notification (A11.2)"
    );

    // And it is genuinely the predicate doing it: the same fixture fires when nothing suppresses.
    assert!(fire(&mut conn).is_some());
}

/// The **fix-available** qualifier is a narrowing, and a ceiling admits narrowing. A critical
/// advisory with no fix is real debt that renders on the project page and simply does not
/// interrupt: an interruption whose recommended action does not exist is the nag the contract was
/// written to prevent.
#[test]
fn a_critical_advisory_with_no_fix_does_not_interrupt() {
    let (_d, mut conn) = fresh();
    let alpha = project(&conn, "alpha", "present", false);
    vulnerable(
        &conn,
        alpha,
        "left",
        &Advisory {
            id: "GHSA-nofix",
            severity: "critical",
            fix: false,
            cve: None,
        },
    );
    assert!(fire(&mut conn).is_none());

    // And a non-critical advisory with a fix does not either: the severity is compared against the
    // source's own word and is never re-scored.
    vulnerable(
        &conn,
        alpha,
        "right",
        &Advisory {
            id: "GHSA-high",
            severity: "high",
            fix: true,
            cve: None,
        },
    );
    assert!(fire(&mut conn).is_none());
}

/// A **withdrawn** advisory never interrupts: the source retracted its own evidence.
#[test]
fn a_withdrawn_advisory_never_interrupts() {
    let (_d, mut conn) = fresh();
    let alpha = project(&conn, "alpha", "present", false);
    vulnerable(&conn, alpha, "left", &critical("GHSA-a"));
    conn.execute(
        "UPDATE advisory SET withdrawn_at = ?1 WHERE advisory_id = 'GHSA-a'",
        [NOW],
    )
    .unwrap();
    assert!(fire(&mut conn).is_none());
}
