#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! §32.8's verdict: **three values, two clocks, and an asymmetric expiry.**
//!
//! Every case here is **clock-driven, never sleep-driven**: the expiry is a parameter, and this
//! file contains no `sleep`.
//!
//! The failure it exists to catch is a **false clean** — a lit tick claiming *no known vulnerable
//! dependencies* over a read that never looked, never finished, or looked six months ago.

use std::path::Path;

use codotheca_core::advisories::lockfiles::read_lockfiles;
use codotheca_core::advisories::verdict::{verdict_for, CLEAN_VERDICT_EXPIRY_SECS};
use codotheca_core::index::migrate::{apply_all, MIGRATIONS};
use codotheca_core::index::{open_connection, Index};
use codotheca_core::protocol::{DependencyVerdict, ProjectId};

const NOW: i64 = 1_800_000_000;
const DAY: i64 = 24 * 60 * 60;

fn fresh() -> (tempfile::TempDir, rusqlite::Connection) {
    let dir = tempfile::tempdir().unwrap();
    let mut conn = open_connection(&Index::db_path(dir.path())).unwrap();
    apply_all(&mut conn, MIGRATIONS).unwrap();
    (dir, conn)
}

fn insert_project(conn: &rusqlite::Connection, name: &str) -> ProjectId {
    conn.execute(
        "INSERT INTO project (name, seed_basename, created_at, updated_at)
         VALUES (?1, ?1, 1, 1)",
        [name],
    )
    .unwrap();
    ProjectId(conn.last_insert_rowid())
}

fn write(root: &Path, rel: &str, body: &str) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, body).unwrap();
}

fn cargo_lock(name: &str, version: &str) -> String {
    format!("[[package]]\nname = \"{name}\"\nversion = \"{version}\"\n")
}

/// Run the real lockfile read over a real directory, at `at`.
fn scan(conn: &mut rusqlite::Connection, project: ProjectId, root: &Path, at: i64) {
    let tx = conn.transaction().unwrap();
    read_lockfiles(&tx, project, root, at).unwrap();
    tx.commit().unwrap();
}

/// Answer one triple, at `at`, with `matches` advisories against it.
fn answer(conn: &rusqlite::Connection, name: &str, version: &str, at: i64, matches: usize) {
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
    for i in 0..matches {
        let id = format!("GHSA-{name}-{i}");
        conn.execute(
            "INSERT INTO advisory (advisory_id, severity, summary, url, observed_at)
             VALUES (?1, 'high', 's', 'u', ?2) ON CONFLICT DO NOTHING",
            rusqlite::params![id, at],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO advisory_match
               (ecosystem, package_name, version, advisory_id, fix_available, fixed_version)
             VALUES ('rust', ?1, ?2, ?3, 1, '9.9.9') ON CONFLICT DO NOTHING",
            rusqlite::params![name, version, id],
        )
        .unwrap();
    }
}

fn verdict(conn: &rusqlite::Connection, project: ProjectId, now: i64) -> DependencyVerdict {
    verdict_for(conn, project, now).unwrap().verdict
}

/// **AC-P3-32-8.** `clean` expires at thirty days; `vulnerable` **never** does.
///
/// Expiring `vulnerable` would silently unopen real debt: both halves of what it asserts are
/// recorded facts and neither is refuted by time passing.
#[test]
fn clean_expires_at_thirty_days_and_vulnerable_never_does() {
    let (_d, mut conn) = fresh();
    let clean = insert_project(&conn, "clean");
    let vulnerable = insert_project(&conn, "vulnerable");
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "Cargo.lock", &cargo_lock("safe", "1.0.0"));
    let other = tempfile::tempdir().unwrap();
    write(other.path(), "Cargo.lock", &cargo_lock("risky", "1.0.0"));

    scan(&mut conn, clean, dir.path(), NOW);
    scan(&mut conn, vulnerable, other.path(), NOW);
    answer(&conn, "safe", "1.0.0", NOW, 0);
    answer(&conn, "risky", "1.0.0", NOW, 1);

    assert_eq!(verdict(&conn, clean, NOW), DependencyVerdict::Clean);
    assert_eq!(
        verdict(&conn, clean, NOW + CLEAN_VERDICT_EXPIRY_SECS - DAY),
        DependencyVerdict::Clean,
        "twenty-nine days old is still a claim this build stands behind"
    );
    assert_eq!(
        verdict(&conn, clean, NOW + CLEAN_VERDICT_EXPIRY_SECS + DAY),
        DependencyVerdict::Unknown,
        "thirty-one days means about thirty consecutive attempts failed"
    );

    for age in [0, CLEAN_VERDICT_EXPIRY_SECS + DAY, 365 * DAY] {
        assert_eq!(
            verdict(&conn, vulnerable, NOW + age),
            DependencyVerdict::Vulnerable,
            "a vulnerable verdict of any age is still vulnerable"
        );
    }
    assert_eq!(CLEAN_VERDICT_EXPIRY_SECS, 30 * DAY);
}

/// **AC-P3-32-9.** The composite clock is **the older of the two**.
///
/// A sweep that ran an hour ago, joined against a triple set read six months ago, otherwise
/// produces a fresh-looking answer about a lockfile nobody has looked at since — which is the
/// currency claim the invariant forbids.
#[test]
fn the_rendered_clock_is_the_older_of_the_two() {
    let (_d, mut conn) = fresh();
    let stale_read = insert_project(&conn, "stale-read");
    let stale_sweep = insert_project(&conn, "stale-sweep");
    let a = tempfile::tempdir().unwrap();
    write(a.path(), "Cargo.lock", &cargo_lock("left", "1.0.0"));
    let b = tempfile::tempdir().unwrap();
    write(b.path(), "Cargo.lock", &cargo_lock("right", "1.0.0"));

    // A lockfile read six months ago, swept an hour ago.
    scan(&mut conn, stale_read, a.path(), NOW - 180 * DAY);
    answer(&conn, "left", "1.0.0", NOW - 3600, 0);
    // The converse: read an hour ago, swept six months ago.
    scan(&mut conn, stale_sweep, b.path(), NOW - 3600);
    answer(&conn, "right", "1.0.0", NOW - 180 * DAY, 0);

    let older = verdict_for(&conn, stale_read, NOW).unwrap();
    assert_eq!(
        older.observed_at,
        Some(NOW - 180 * DAY),
        "the six-month-old lockfile is what the answer is really about"
    );
    let converse = verdict_for(&conn, stale_sweep, NOW).unwrap();
    assert_eq!(converse.observed_at, Some(NOW - 180 * DAY));
    // Both are older than the clean expiry, so both read `unknown` rather than claiming currency.
    assert_eq!(older.verdict, DependencyVerdict::Unknown);
    assert_eq!(converse.verdict, DependencyVerdict::Unknown);
}

/// **AC-P3-32-15.** An unparsed ecosystem, no lockfile at all, and a scan that has not run are
/// **three different answers** — all asserted in one test, because the whole point is that they
/// are distinguishable.
#[test]
fn unparsed_no_lockfile_and_never_scanned_are_three_states() {
    let (_d, mut conn) = fresh();
    let unshipped = insert_project(&conn, "unshipped");
    let empty = insert_project(&conn, "empty");
    let never = insert_project(&conn, "never");
    let unresolved = insert_project(&conn, "unresolved");

    // An ecosystem this build ships no parser for: it declares dependencies that can never
    // resolve to triples, so `clean` would be a claim over a set nobody assembled.
    let go = tempfile::tempdir().unwrap();
    write(go.path(), "go.mod", "module example\n");
    scan(&mut conn, unshipped, go.path(), NOW);
    assert_eq!(verdict(&conn, unshipped, NOW), DependencyVerdict::Unknown);

    // Ran, and there is genuinely nothing: no lockfile, no manifest.
    let bare = tempfile::tempdir().unwrap();
    write(bare.path(), "README.md", "nothing here");
    scan(&mut conn, empty, bare.path(), NOW);
    assert_eq!(verdict(&conn, empty, NOW), DependencyVerdict::Clean);

    // Never scanned: the **absence** of the scan row, and it is unknown.
    assert_eq!(verdict(&conn, never, NOW), DependencyVerdict::Unknown);

    // A manifest in a **shipped** ecosystem with no lockfile: unresolved versions, and version
    // matching is server-side and needs a version.
    let manifest = tempfile::tempdir().unwrap();
    write(manifest.path(), "package.json", "{\"name\":\"x\"}");
    scan(&mut conn, unresolved, manifest.path(), NOW);
    assert_eq!(verdict(&conn, unresolved, NOW), DependencyVerdict::Unknown);
}

/// **AC-P3-32-20.** An **uninstalled** project still produces a verdict, with the triples' own age
/// attached, and **no byte is read from that project's disk**.
///
/// The root is pointed at a path that does not exist; the call still answers, because the join is
/// a pure recompute over stored facts.
#[test]
fn an_uninstalled_project_still_gets_a_verdict_without_touching_its_disk() {
    let (_d, mut conn) = fresh();
    let project = insert_project(&conn, "gone");
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "Cargo.lock", &cargo_lock("left", "1.0.0"));
    scan(&mut conn, project, dir.path(), NOW);
    answer(&conn, "left", "1.0.0", NOW, 1);
    conn.execute(
        "INSERT INTO location (project_id, path_display, path_key, kind, presence, removed_at,
                               first_seen_at, last_seen_at)
         VALUES (?1, '/gone', 'gone', 'native', 'missing', ?2, ?2, ?2)",
        rusqlite::params![project.0, NOW],
    )
    .ok();
    drop(dir);

    let reading = verdict_for(&conn, project, NOW + DAY).unwrap();
    assert_eq!(reading.verdict, DependencyVerdict::Vulnerable);
    assert_eq!(
        reading.observed_at,
        Some(NOW),
        "the triples' own age, not the moment it was asked"
    );

    // And a path that never existed answers just the same: the reading is frozen at the last
    // computed state, carrying the time it was computed.
    let reading = verdict_for(&conn, project, NOW + 365 * DAY).unwrap();
    assert_eq!(reading.verdict, DependencyVerdict::Vulnerable);
    assert_eq!(reading.observed_at, Some(NOW));
}

/// **A reading that was never computed is not frozen — it is absent.** An uninstalled project
/// nobody ever scanned reads `unknown` with **no** clock, which is what stops a surface rendering
/// an age nobody measured.
#[test]
fn a_reading_that_was_never_computed_carries_no_clock() {
    let (_d, conn) = fresh();
    let project = insert_project(&conn, "never");
    let reading = verdict_for(&conn, project, NOW).unwrap();
    assert_eq!(reading.verdict, DependencyVerdict::Unknown);
    assert_eq!(
        reading.observed_at, None,
        "absent, never a zero nobody made"
    );
}

/// **An unanswered triple makes the whole verdict unknown.** A project is vulnerable through the
/// dependency nobody asked about exactly as easily as through the one that was.
#[test]
fn one_unasked_triple_is_enough_to_make_the_verdict_unknown() {
    let (_d, mut conn) = fresh();
    let project = insert_project(&conn, "half-swept");
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "Cargo.lock",
        &format!(
            "{}{}",
            cargo_lock("asked", "1.0.0"),
            cargo_lock("unasked", "2.0.0")
        ),
    );
    scan(&mut conn, project, dir.path(), NOW);
    answer(&conn, "asked", "1.0.0", NOW, 0);

    assert_eq!(verdict(&conn, project, NOW), DependencyVerdict::Unknown);
    answer(&conn, "unasked", "2.0.0", NOW, 0);
    assert_eq!(verdict(&conn, project, NOW), DependencyVerdict::Clean);
}

/// A **withdrawn** advisory stops making its project vulnerable: the source changed its mind, and
/// the verdict follows the source.
#[test]
fn a_withdrawn_advisory_no_longer_makes_a_project_vulnerable() {
    let (_d, mut conn) = fresh();
    let project = insert_project(&conn, "withdrawn");
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "Cargo.lock", &cargo_lock("left", "1.0.0"));
    scan(&mut conn, project, dir.path(), NOW);
    answer(&conn, "left", "1.0.0", NOW, 1);
    assert_eq!(verdict(&conn, project, NOW), DependencyVerdict::Vulnerable);

    conn.execute(
        "UPDATE advisory SET withdrawn_at = ?1 WHERE advisory_id = 'GHSA-left-0'",
        [NOW + 60],
    )
    .unwrap();
    assert_eq!(verdict(&conn, project, NOW + 120), DependencyVerdict::Clean);
}

/// A lockfile the read could not take is `unknown`, **whatever the other lockfiles said**.
#[test]
fn one_unreadable_lockfile_makes_the_whole_project_unknown() {
    let (_d, mut conn) = fresh();
    let project = insert_project(&conn, "mixed");
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "Cargo.lock", &cargo_lock("left", "1.0.0"));
    // A yarn.lock whose entry has no version line: a construct the reader does not understand.
    write(dir.path(), "yarn.lock", "left@^1.0.0:\n  resolved \"x\"\n");
    scan(&mut conn, project, dir.path(), NOW);
    answer(&conn, "left", "1.0.0", NOW, 0);

    let not_read: i64 = conn
        .query_row(
            "SELECT count(*) FROM project_lockfile WHERE read_state = 'notRead'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(not_read, 1);
    assert_eq!(verdict(&conn, project, NOW), DependencyVerdict::Unknown);
}
