//! §34.4's emit gate — **a withdrawal is recorded and never announced** (R135).
//!
//! §28.10 left §34 one question: does an `invalidated` decrease originate a surge. The row is
//! written, because the value genuinely moved and the history must say so; the event is not,
//! because the surge is a reward and nothing animates for work nobody did.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use codotheca_core::advisories::items::sync_advisory_items;
use codotheca_core::advisories::lockfiles::read_lockfiles;
use codotheca_core::debt::store::SqliteDebtStore;
use codotheca_core::index::migrate::{apply_all, MIGRATIONS};
use codotheca_core::index::{open_connection, Index};
use codotheca_core::protocol::{DecayLayer, HealthDetectedIn, ProjectHealthDelta, ProjectId};
use codotheca_core::restoration::{record_after_write, LayerValues};
use rusqlite::Connection;

const NOW: i64 = 1_800_000_000;
const ACK: i64 = NOW - 1_000;

fn rows(conn: &Connection) -> usize {
    let n: i64 = conn
        .query_row("SELECT count(*) FROM health_delta", [], |r| r.get(0))
        .unwrap();
    usize::try_from(n).unwrap()
}

fn only_row(conn: &Connection) -> (String, f64, f64, String) {
    conn.query_row(
        "SELECT layer, from_value, to_value, detected_in FROM health_delta",
        [],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
    )
    .unwrap()
}

// ---------------------------------------------------------------------------------------------
// The emit gate: a withdrawal is recorded and never announced.
// ---------------------------------------------------------------------------------------------

/// One enrolled project whose `rust` layer is fed by §32's advisory items.
fn advisory_db() -> (tempfile::TempDir, Connection, ProjectId, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let mut conn = open_connection(&Index::db_path(dir.path())).unwrap();
    apply_all(&mut conn, MIGRATIONS).unwrap();
    conn.execute(
        "INSERT INTO project (name, seed_basename, lineage_key, authored_by_user, is_reference,
                              acknowledged_at, created_at, updated_at)
         VALUES ('alpha', 'alpha', 'alpha', 1, 0, ?1, 1, 1)",
        [ACK],
    )
    .unwrap();
    let project = ProjectId(conn.last_insert_rowid());
    conn.execute(
        "INSERT INTO location (project_id, kind, path_bytes, path_key, path_display, store_key,
                               presence, repo_kind)
         VALUES (?1, 'linux', x'2f61', x'2f61', '/a', 'store', 'present', 'worktree')",
        [project.0],
    )
    .unwrap();
    let root = tempfile::tempdir().unwrap();
    (dir, conn, project, root)
}

fn lockfile(
    conn: &mut Connection,
    project: ProjectId,
    root: &std::path::Path,
    pkgs: &[(&str, &str)],
) {
    use std::fmt::Write as _;
    let mut body = String::new();
    for (name, version) in pkgs {
        let _ = write!(
            body,
            "[[package]]\nname = \"{name}\"\nversion = \"{version}\"\n"
        );
    }
    std::fs::write(root.join("Cargo.lock"), body).unwrap();
    let tx = conn.transaction().unwrap();
    read_lockfiles(&tx, project, root, NOW).unwrap();
    tx.commit().unwrap();
}

fn advise(conn: &Connection, name: &str, version: &str, advisory: Option<&str>) {
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
         VALUES ('rust', ?1, ?2, ?3, ?4, 1)
         ON CONFLICT DO UPDATE SET sweep_id = excluded.sweep_id,
           observed_at = excluded.observed_at, answered = 1",
        rusqlite::params![name, version, sweep, NOW],
    )
    .unwrap();
    if let Some(id) = advisory {
        conn.execute(
            "INSERT INTO advisory (advisory_id, severity, summary, url, observed_at)
             VALUES (?1, 'critical', 's', 'u', ?2) ON CONFLICT DO NOTHING",
            rusqlite::params![id, NOW],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO advisory_match
               (ecosystem, package_name, version, advisory_id, fix_available, fixed_version)
             VALUES ('rust', ?1, ?2, ?3, 1, '9.9.9')
             ON CONFLICT DO NOTHING",
            rusqlite::params![name, version, id],
        )
        .unwrap();
    }
}

/// §32's real item sweep, wrapped the way a production caller wraps a debt write.
fn advisory_settle(
    conn: &mut Connection,
    project: ProjectId,
    at: i64,
) -> Option<ProjectHealthDelta> {
    let tx = conn.transaction().unwrap();
    let before = LayerValues::read(&tx, project).unwrap();
    let swept = sync_advisory_items(&tx, project, None, at, &SqliteDebtStore).unwrap();
    let event = record_after_write(
        &tx,
        project,
        &before,
        &swept.effect.closed,
        HealthDetectedIn::Background,
        at,
    )
    .unwrap();
    tx.commit().unwrap();
    event
}

/// **§28.10's question, answered (R135).** A layer whose whole decrease is a withdrawn advisory
/// writes its row — the value genuinely moved — and announces nothing: nothing animates for work
/// nobody did.
#[test]
fn an_invalidated_only_decrease_is_written_and_never_announced() {
    let (_d, mut conn, project, root) = advisory_db();
    lockfile(
        &mut conn,
        project,
        root.path(),
        &[("left", "1.0.0"), ("right", "1.0.0")],
    );
    advise(&conn, "left", "1.0.0", Some("GHSA-left"));
    advise(&conn, "right", "1.0.0", Some("GHSA-right"));
    assert!(advisory_settle(&mut conn, project, NOW).is_none());
    assert_eq!(rows(&conn), 0);

    conn.execute(
        "UPDATE advisory SET withdrawn_at = ?1 WHERE advisory_id = 'GHSA-left'",
        [NOW + 60],
    )
    .unwrap();
    let event = advisory_settle(&mut conn, project, NOW + 120);
    eprintln!(
        "withdrawal only: {} row(s), event {:?}",
        rows(&conn),
        event.as_ref().map(|e| e.layers.len())
    );
    assert_eq!(rows(&conn), 1, "the history lost a real change of value");
    assert_eq!(
        only_row(&conn),
        ("rust".to_owned(), 2.0, 1.0, "background".to_owned())
    );
    assert!(
        event.is_none(),
        "a withdrawal was announced as a restoration"
    );
}

/// **The mixed case is announced with its true values**: one closure the user caused and one they
/// did not, on the same layer. Named as not closed — the surge may originate at a decrease that was
/// partly unearned, bounded by one item, and no value is fabricated to shave it off.
#[test]
fn a_mixed_decrease_is_announced_with_its_true_values() {
    let (_d, mut conn, project, root) = advisory_db();
    lockfile(
        &mut conn,
        project,
        root.path(),
        &[("left", "1.0.0"), ("right", "1.0.0"), ("third", "1.0.0")],
    );
    advise(&conn, "left", "1.0.0", Some("GHSA-left"));
    advise(&conn, "right", "1.0.0", Some("GHSA-right"));
    advise(&conn, "third", "1.0.0", Some("GHSA-third"));
    assert!(advisory_settle(&mut conn, project, NOW).is_none());

    // `left` is withdrawn; `right` is upgraded past its advisory — a fix.
    conn.execute(
        "UPDATE advisory SET withdrawn_at = ?1 WHERE advisory_id = 'GHSA-left'",
        [NOW + 60],
    )
    .unwrap();
    lockfile(
        &mut conn,
        project,
        root.path(),
        &[("left", "1.0.0"), ("right", "2.0.0"), ("third", "1.0.0")],
    );
    advise(&conn, "right", "2.0.0", None);
    let event =
        advisory_settle(&mut conn, project, NOW + 120).expect("the mixed case is announced");
    eprintln!("mixed: {} row(s), {:?}", rows(&conn), event.layers);
    assert_eq!(event.layers.len(), 1);
    assert_eq!(event.layers[0].layer, DecayLayer::Rust);
    assert_eq!(
        (event.layers[0].from_value, event.layers[0].to_value),
        (Some(3.0), Some(1.0))
    );
}
