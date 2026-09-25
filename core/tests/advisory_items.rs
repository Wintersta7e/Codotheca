#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! §32.10's items: **shown is wider than scored, and a withdrawal closes without paying.**
//!
//! Two failures this file exists to catch, and both are expensive:
//!
//! * A withdrawal closed as `fixed` puts an **unearned row in an append-only ledger** with a
//!   monotonic `level_floor`, for work the user did not do.
//! * An unreadable lockfile treated as evidence of a fix means **every uninstall silently pays
//!   out that project's whole open debt list**.

use std::collections::BTreeSet;
use std::path::Path;

use codotheca_core::advisories::items::{advisory_fingerprint, sync_advisory_items};
use codotheca_core::advisories::lockfiles::read_lockfiles;
use codotheca_core::debt::store::{DebtCloseReason, DebtStore, SqliteDebtStore};
use codotheca_core::debt::sweep::SweepObservation;
use codotheca_core::index::migrate::{apply_all, MIGRATIONS};
use codotheca_core::index::{open_connection, Index};
use codotheca_core::protocol::{
    DebtScoring, DebtSource, DebtSweepOutcome, Ecosystem, ObservationBasis, ProjectId,
};

const NOW: i64 = 1_800_000_000;

fn fresh() -> (tempfile::TempDir, rusqlite::Connection) {
    let dir = tempfile::tempdir().unwrap();
    let mut conn = open_connection(&Index::db_path(dir.path())).unwrap();
    apply_all(&mut conn, MIGRATIONS).unwrap();
    (dir, conn)
}

/// A project with a lineage key, because the ledger is keyed on the **subject** and not on the
/// project id — §1.7 records that v1 keyed it the other way and it broke on merges.
fn insert_project(conn: &rusqlite::Connection, name: &str) -> ProjectId {
    conn.execute(
        "INSERT INTO project (name, seed_basename, lineage_key, created_at, updated_at)
         VALUES (?1, ?1, ?1, 1, 1)",
        [name],
    )
    .unwrap();
    let project = ProjectId(conn.last_insert_rowid());
    // **A present copy**, because a project with none is deliberately unclosable: an uninstalled
    // copy's empty walk is not evidence that its dependencies were dropped.
    conn.execute(
        "INSERT INTO location (project_id, kind, path_bytes, path_key, path_display, store_key,
                               presence, repo_kind)
         VALUES (?1, 'linux', ?2, ?2, ?3, 'store', 'present', 'worktree')",
        rusqlite::params![project.0, name.as_bytes(), name],
    )
    .unwrap();
    project
}

fn write(root: &Path, rel: &str, body: &str) {
    std::fs::write(root.join(rel), body).unwrap();
}

fn cargo_lock(entries: &[(&str, &str)]) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    for (name, version) in entries {
        let _ = write!(
            out,
            "[[package]]\nname = \"{name}\"\nversion = \"{version}\"\n"
        );
    }
    out
}

fn scan(conn: &mut rusqlite::Connection, project: ProjectId, root: &Path, at: i64) {
    let tx = conn.transaction().unwrap();
    read_lockfiles(&tx, project, root, at).unwrap();
    tx.commit().unwrap();
}

/// Answer a triple, optionally matching an advisory with or without a fix.
fn answer(
    conn: &rusqlite::Connection,
    name: &str,
    version: &str,
    at: i64,
    advisory: Option<(&str, bool)>,
) {
    conn.execute(
        "INSERT INTO advisory_sweep (started_at, settled_at, outcome, complete)
         VALUES (?1, ?1, 'done', 1)",
        [at],
    )
    .unwrap();
    let sweep = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO advisory_triple
           (ecosystem, package_name, version, sweep_id, observed_at, answered)
         VALUES ('rust', ?1, ?2, ?3, ?4, 1)
         ON CONFLICT DO UPDATE SET sweep_id = excluded.sweep_id,
           observed_at = excluded.observed_at, answered = 1",
        rusqlite::params![name, version, sweep, at],
    )
    .unwrap();
    if let Some((id, fix)) = advisory {
        conn.execute(
            "INSERT INTO advisory (advisory_id, severity, summary, url, observed_at)
             VALUES (?1, 'critical', 's', 'u', ?2) ON CONFLICT DO NOTHING",
            rusqlite::params![id, at],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO advisory_match
               (ecosystem, package_name, version, advisory_id, fix_available, fixed_version)
             VALUES ('rust', ?1, ?2, ?3, ?4, ?5)
             ON CONFLICT DO UPDATE SET fix_available = excluded.fix_available,
               fixed_version = excluded.fixed_version",
            rusqlite::params![name, version, id, i64::from(fix), fix.then_some("9.9.9")],
        )
        .unwrap();
    }
}

fn sync(conn: &mut rusqlite::Connection, project: ProjectId, at: i64) -> ItemSweep {
    let tx = conn.transaction().unwrap();
    let got = sync_advisory_items(&tx, project, None, at, &SqliteDebtStore).unwrap();
    tx.commit().unwrap();
    ItemSweep {
        opened: got.opened,
        closed_fixed: got.closed_fixed,
        closed_invalidated: got.closed_invalidated,
        unverified: got.unverified,
        reasons: got.effect.closed.iter().map(|(_, r)| *r).collect(),
    }
}

struct ItemSweep {
    opened: usize,
    closed_fixed: usize,
    closed_invalidated: usize,
    unverified: usize,
    reasons: Vec<DebtCloseReason>,
}

/// Every `debt_item` row, by identity: `(rowid, fingerprint, state, scoring, first_seen_at)`.
fn items(conn: &rusqlite::Connection) -> Vec<(i64, String, String, String, i64)> {
    let mut stmt = conn
        .prepare(
            "SELECT id, fingerprint, state, scoring, first_seen_at FROM debt_item
              WHERE source = 'dependency_advisory' ORDER BY fingerprint",
        )
        .unwrap();
    stmt.query_map([], |r| {
        Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
    })
    .unwrap()
    .map(Result::unwrap)
    .collect()
}

/// The ledger's row set **by identity**, never by count: a count comparison passes against a
/// delete-plus-insert.
fn ledger(conn: &rusqlite::Connection) -> BTreeSet<String> {
    let mut stmt = conn
        .prepare("SELECT dedupe_key FROM xp_events ORDER BY dedupe_key")
        .unwrap();
    stmt.query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

/// **AC-P3-32-14.** A **no-fix** critical advisory opens a **visible** item that scores nothing,
/// and the same advisory gaining a fix flips it to `scored` **without a new item**.
///
/// Asserted on the item's `rowid` and `first_seen_at` being unchanged, which is what proves no
/// close-and-reopen happened — and therefore no second payout.
#[test]
fn ac_p3_32_14_a_no_fix_advisory_is_shown_only() {
    let (_d, mut conn) = fresh();
    let project = insert_project(&conn, "alpha");
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "Cargo.lock", &cargo_lock(&[("left", "1.0.0")]));
    scan(&mut conn, project, dir.path(), NOW);
    answer(&conn, "left", "1.0.0", NOW, Some(("GHSA-x", false)));

    let first = sync(&mut conn, project, NOW);
    assert_eq!(first.opened, 1);
    let opened = items(&conn);
    assert_eq!(opened.len(), 1);
    assert_eq!(
        opened[0].1,
        advisory_fingerprint(Ecosystem::Rust, "left", "GHSA-x")
    );
    assert_eq!(opened[0].2, "open", "it is visible debt");
    assert_eq!(
        opened[0].3, "shown_only",
        "an advisory with no fix scores nothing"
    );

    // The same advisory gains a fix.
    conn.execute(
        "UPDATE advisory_match SET fix_available = 1, fixed_version = '2.0.0'",
        [],
    )
    .unwrap();
    let second = sync(&mut conn, project, NOW + 60);
    assert_eq!(second.opened, 0, "no new item");
    assert_eq!(second.closed_fixed, 0);
    let after = items(&conn);
    eprintln!(
        "advisory_items: {} item(s); scoring {} -> {}",
        after.len(),
        opened[0].3,
        after[0].3
    );
    assert_eq!(after[0].3, "scored");
    assert_eq!(after[0].0, opened[0].0, "the same row: no close and reopen");
    assert_eq!(after[0].4, opened[0].4, "first_seen_at is untouched");
}

/// **AC-P3-32-19.** A **withdrawn** advisory closes its item as `invalidated`, writes **no**
/// `xp_events` row, and claws nothing back.
///
/// The ledger is compared **by identity**, not by count.
#[test]
fn ac_p3_32_19_a_withdrawal_closes_invalidated_and_pays_nothing() {
    let (_d, mut conn) = fresh();
    let project = insert_project(&conn, "alpha");
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "Cargo.lock", &cargo_lock(&[("left", "1.0.0")]));
    scan(&mut conn, project, dir.path(), NOW);
    answer(&conn, "left", "1.0.0", NOW, Some(("GHSA-x", true)));

    assert_eq!(sync(&mut conn, project, NOW).opened, 1);
    let before = ledger(&conn);

    conn.execute(
        "UPDATE advisory SET withdrawn_at = ?1 WHERE advisory_id = 'GHSA-x'",
        [NOW + 60],
    )
    .unwrap();
    let closed = sync(&mut conn, project, NOW + 120);

    eprintln!(
        "advisory_items: closed {} invalidated, {} fixed",
        closed.closed_invalidated, closed.closed_fixed
    );
    assert_eq!(closed.closed_invalidated, 1);
    assert_eq!(closed.closed_fixed, 0, "the user performed no fix");
    assert_eq!(
        closed.reasons,
        vec![DebtCloseReason::Invalidated],
        "the reason the XP writer reads, not only the count this test prints"
    );
    assert!(items(&conn).is_empty(), "closed is a deletion");
    assert_eq!(
        ledger(&conn),
        before,
        "the ledger's row set is unchanged, by identity and not by count"
    );
}

/// A version bump from one vulnerable version to another leaves the item **open with the same
/// fingerprint**: no close, no reopen, no second payout.
///
/// The version is deliberately absent from the key for exactly this.
#[test]
fn a_bump_between_two_vulnerable_versions_moves_no_item() {
    let (_d, mut conn) = fresh();
    let project = insert_project(&conn, "alpha");
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "Cargo.lock", &cargo_lock(&[("left", "1.0.0")]));
    scan(&mut conn, project, dir.path(), NOW);
    answer(&conn, "left", "1.0.0", NOW, Some(("GHSA-x", true)));
    sync(&mut conn, project, NOW);
    let before = items(&conn);
    assert_eq!(before.len(), 1);

    // The user upgrades — to a version the same advisory still names.
    write(dir.path(), "Cargo.lock", &cargo_lock(&[("left", "1.5.0")]));
    scan(&mut conn, project, dir.path(), NOW + 60);
    answer(&conn, "left", "1.5.0", NOW + 60, Some(("GHSA-x", true)));
    let after_sweep = sync(&mut conn, project, NOW + 120);

    let after = items(&conn);
    assert_eq!(after_sweep.opened, 0);
    assert_eq!(after_sweep.closed_fixed, 0);
    assert_eq!(after_sweep.closed_invalidated, 0);
    assert_eq!(after, before, "same row, same fingerprint, same clock");
}

/// **A project whose lockfiles cannot be re-read marks its items `unverified` and closes none.**
///
/// This is the single most expensive failure in the section: without it every uninstall pays out
/// the project's whole open debt list, silently, into an append-only ledger.
#[test]
fn an_unreadable_project_marks_unverified_and_pays_nothing() {
    let (_d, mut conn) = fresh();
    let project = insert_project(&conn, "alpha");
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "Cargo.lock", &cargo_lock(&[("left", "1.0.0")]));
    scan(&mut conn, project, dir.path(), NOW);
    answer(&conn, "left", "1.0.0", NOW, Some(("GHSA-x", true)));
    sync(&mut conn, project, NOW);
    let before = ledger(&conn);
    assert_eq!(items(&conn).len(), 1);

    // The copy is uninstalled: the lockfile cannot be re-read, and the walk finds nothing.
    conn.execute(
        "UPDATE location SET removed_at = ?2, presence = 'missing' WHERE project_id = ?1",
        rusqlite::params![project.0, NOW + 60],
    )
    .unwrap();
    std::fs::remove_file(dir.path().join("Cargo.lock")).unwrap();
    scan(&mut conn, project, dir.path(), NOW + 60);

    let swept = sync(&mut conn, project, NOW + 120);
    let rows = items(&conn);
    eprintln!(
        "advisory_items: {} unverified, {} closed, {} row(s) left",
        swept.unverified,
        swept.closed_fixed + swept.closed_invalidated,
        rows.len()
    );
    // **The walk ran and found nothing, so the verdict is `clean` — and that is exactly the
    // trap.** The lockfile is gone because the copy is gone, not because the dependency was
    // dropped, so the verdict alone would close every item as `fixed` and the day's payout would
    // pay the user for an uninstall.
    assert_eq!(
        swept.closed_fixed + swept.closed_invalidated,
        0,
        "an uninstall closed the project's open debt"
    );
    assert_eq!(swept.unverified, 1, "unverified, not closed");
    assert_eq!(rows.len(), 1, "the item still exists");
    assert_eq!(rows[0].2, "unverified");
    assert!(
        swept.reasons.is_empty(),
        "nothing reached the XP writer as a close at all"
    );
    assert_eq!(ledger(&conn), before);
}

/// **AC-P3-32-18, second half.** **No closure is computed by diffing a worktree observation
/// against a HEAD observation.** A complete HEAD-basis sweep of this source that saw nothing goes
/// through the same `DebtStore::observe` the advisory items close through, and closes nothing.
///
/// The observation shares the item's anchor, so the basis is the only thing that differs.
#[test]
fn ac_p3_32_18_a_head_basis_sweep_closes_nothing() {
    let (_d, mut conn) = fresh();
    let project = insert_project(&conn, "alpha");
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "Cargo.lock", &cargo_lock(&[("left", "1.0.0")]));
    scan(&mut conn, project, dir.path(), NOW);
    answer(&conn, "left", "1.0.0", NOW, Some(("GHSA-x", true)));
    sync(&mut conn, project, NOW);
    let before = items(&conn);
    assert_eq!(before.len(), 1);
    let ledger_before = ledger(&conn);

    // A HEAD-basis sweep for this very source, at the item's own anchor, claiming it observed
    // nothing.
    let head = SweepObservation {
        project,
        source: DebtSource::DependencyAdvisory,
        outcome: DebtSweepOutcome::Complete,
        location: None,
        generation: None,
        basis: Some(ObservationBasis::Head),
        item_count: Some(0),
        observed_at: NOW + 60,
    };
    let tx = conn.transaction().unwrap();
    let effect = SqliteDebtStore.observe(&tx, &head, &[]).unwrap();
    tx.commit().unwrap();

    let rows = items(&conn);
    eprintln!(
        "advisory_items: {} closed, {} row(s) after a HEAD-basis sweep, state {:?}",
        effect.closed.len(),
        rows.len(),
        rows.first().map(|r| r.2.as_str())
    );
    assert!(
        effect.closed.is_empty(),
        "a HEAD observation closed a worktree item: {:?}",
        effect.closed
    );
    assert_eq!(
        rows.len(),
        1,
        "a HEAD observation cannot close a worktree item"
    );
    assert_eq!(rows[0].0, before[0].0, "the same row: nothing was closed");
    assert_eq!(ledger(&conn), ledger_before, "nothing was paid");
}

/// The scored set is exactly the **fixable** set; the shown set is larger. The two reconcile by
/// construction rather than by choosing one wording.
#[test]
fn shown_is_wider_than_scored() {
    let (_d, mut conn) = fresh();
    let project = insert_project(&conn, "alpha");
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "Cargo.lock",
        &cargo_lock(&[("fixable", "1.0.0"), ("unfixable", "1.0.0")]),
    );
    scan(&mut conn, project, dir.path(), NOW);
    answer(&conn, "fixable", "1.0.0", NOW, Some(("GHSA-a", true)));
    answer(&conn, "unfixable", "1.0.0", NOW, Some(("GHSA-b", false)));

    assert_eq!(sync(&mut conn, project, NOW).opened, 2);
    let rows = items(&conn);
    let scored: Vec<&str> = rows
        .iter()
        .filter(|r| r.3 == "scored")
        .map(|r| r.1.as_str())
        .collect();
    let shown: Vec<&str> = rows.iter().map(|r| r.1.as_str()).collect();
    eprintln!(
        "advisory_items: {} shown, {} scored",
        shown.len(),
        scored.len()
    );
    assert_eq!(shown.len(), 2);
    assert_eq!(scored.len(), 1);
    assert_eq!(
        scored[0],
        advisory_fingerprint(Ecosystem::Rust, "fixable", "GHSA-a")
    );
    // The scoring vocabulary is §28's and is not restated here.
    assert_eq!(
        [DebtScoring::Scored, DebtScoring::ShownOnly].len(),
        2,
        "two values, orthogonal to the item's state"
    );
}

/// An index fault reading an advisory's detail is an **error**, never an absent detail: read as
/// absent, the page would show an advisory item with its severity silently missing.
#[test]
fn an_index_fault_reading_the_detail_is_an_error() {
    let (_d, conn) = fresh();
    let fingerprint = advisory_fingerprint(Ecosystem::Npm, "left", "GHSA-x");
    assert_eq!(
        codotheca_core::advisories::items::advisory_detail_for(&conn, &fingerprint).unwrap(),
        None,
        "no advisory row is no detail"
    );
    conn.execute_batch("DROP TABLE advisory_match").unwrap();
    let faulted = codotheca_core::advisories::items::advisory_detail_for(&conn, &fingerprint);
    eprintln!("advisory_items: a faulted detail read returned {faulted:?}");
    assert!(faulted.is_err(), "an index fault read as no detail");
}
