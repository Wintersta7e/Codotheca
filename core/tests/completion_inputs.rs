//! §31.8's four gates and the shapes `gather` hands the evaluator.
//!
//! **These live outside `core/src` on purpose.** `completion_evaluator.rs` walks
//! `core/src/completion` for the six predicates R124 gave §28, and a fixture that writes a
//! `project_content_scan` row has to name three of them. Keeping the fixtures here is what lets
//! that walk stay a plain substring scan over the module rather than one with a carve-out for
//! test blocks — a carve-out is where the next re-derivation would hide.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use codotheca_core::completion::evaluate::CompletionInputs;
use codotheca_core::completion::inputs::{gather, NotScorable};
use codotheca_core::index::migrate::{apply_all, MIGRATIONS};
use codotheca_core::index::{open_connection, Index};
use codotheca_core::jobs::presence::PresenceState;
use codotheca_core::protocol::{CompletionCheck, DebtSweepOutcome, ProjectId};
use rusqlite::Connection;

fn fresh() -> (tempfile::TempDir, Connection) {
    let dir = tempfile::tempdir().unwrap();
    let mut conn = open_connection(&Index::db_path(dir.path())).unwrap();
    apply_all(&mut conn, MIGRATIONS).unwrap();
    (dir, conn)
}

/// A project past every gate: authored, not a reference, one present copy.
fn scorable(conn: &Connection, name: &str) -> ProjectId {
    conn.execute(
        "INSERT INTO project (name, seed_basename, lineage_key, authored_by_user,
                              created_at, updated_at)
         VALUES (?1, ?1, 'abc123', 1, 1, 100)",
        [name],
    )
    .unwrap();
    let id = ProjectId(conn.last_insert_rowid());
    copy(conn, id, "present", Some(100));
    id
}

fn copy(conn: &Connection, project: ProjectId, presence: &str, observed: Option<i64>) -> i64 {
    let n: i64 = conn
        .query_row("SELECT count(*) FROM location", [], |r| r.get(0))
        .unwrap();
    let path = format!("/copy-{n}");
    conn.execute(
        "INSERT INTO location (project_id, kind, path_bytes, path_key, path_display,
                               store_key, presence, repo_kind, worktree_newest_mtime,
                               refstate_observed_at)
         VALUES (?1, 'linux', ?2, ?2, ?3, 'store', ?4, 'worktree', ?5, ?6)",
        rusqlite::params![project.0, path.as_bytes(), path, presence, n, observed],
    )
    .unwrap();
    conn.last_insert_rowid()
}

fn content_scan(conn: &Connection, project: ProjectId, ci: &str) {
    conn.execute(
        "INSERT INTO project_content_scan
            (project_id, head_oid, complete_head_oid, blobs_total, blobs_pending,
             predicate_version, has_readme, has_license, has_tests, has_ci,
             presence_observed_at, enumerated_at, completed_at)
         VALUES (?1, 'head0', 'head0', 1, 0, 1, 'present', 'present', 'present', ?2, 1, 1, 1)",
        rusqlite::params![project.0, ci],
    )
    .unwrap();
}

fn got(conn: &Connection, project: ProjectId) -> CompletionInputs {
    gather(conn, project).unwrap().expect("scorable")
}

#[test]
fn a_missing_content_scan_row_is_never_observed_and_not_absent() {
    let (_d, conn) = fresh();
    let p = scorable(&conn, "unswept");
    assert_eq!(got(&conn, p).has_ci, None, "row-absent is not `absent`");

    let q = scorable(&conn, "swept");
    content_scan(&conn, q, "not_read");
    assert_eq!(
        got(&conn, q).has_ci,
        Some(PresenceState::NotRead),
        "a budget exceedance survives as not_read, never as absent"
    );
}

/// A missing `debt_sweep` row is `None`, which is *never observed* — **never a zero**.
#[test]
fn a_missing_sweep_row_is_none_and_not_a_zero() {
    let (_d, conn) = fresh();
    let p = scorable(&conn, "unswept");
    let inputs = got(&conn, p);
    assert_eq!(inputs.readme.outcome, None);
    assert_eq!(inputs.readme.open_items, 0);

    conn.execute(
        "INSERT INTO debt_sweep (project_id, source, outcome, item_count, observed_at)
         VALUES (?1, 'missing_readme', 'complete', 0, 10)",
        [p.0],
    )
    .unwrap();
    assert_eq!(
        got(&conn, p).readme.outcome,
        Some(DebtSweepOutcome::Complete)
    );
}

/// §5.1's primary copy decides, and **presence outranks recency** in that rule — so a
/// project keeps being scored while any copy is present, and freezes only when the copy it
/// is measured by is not there.
///
/// The second half is the distinction §31.8 turns on: a stored reading **stands**, and a
/// reading that was never computed may not be frozen because there is nothing to freeze.
#[test]
fn an_absent_primary_copy_freezes_a_stored_reading_and_never_invents_one() {
    let (_d, conn) = fresh();
    let p = scorable(&conn, "one-copy");
    conn.execute(
        "UPDATE location SET presence = 'offline' WHERE project_id = ?1",
        [p.0],
    )
    .unwrap();
    // No rows stored yet, so it is *never computed* rather than frozen.
    assert_eq!(
        gather(&conn, p).unwrap().err(),
        Some(NotScorable::NotComputed)
    );

    conn.execute(
        "INSERT INTO project_check (project_id, check_key, state, observed_at)
         VALUES (?1, 'readme', 'pass', 5)",
        [p.0],
    )
    .unwrap();
    assert_eq!(
        gather(&conn, p).unwrap().err(),
        Some(NotScorable::Frozen),
        "a stored reading stands with its own clock; recomputing it would take a project's \
         gold because a drive was unplugged"
    );

    // A sibling copy that IS present outranks the offline one, and the project is scored
    // again — `pick_primary` puts presence above recency, so an unplugged drive cannot
    // freeze a project the user still has a working copy of.
    conn.execute(
        "INSERT INTO location (project_id, kind, path_bytes, path_key, path_display,
                               store_key, presence, repo_kind, worktree_newest_mtime,
                               refstate_observed_at)
         VALUES (?1, 'linux', CAST('/live' AS BLOB), CAST('/live' AS BLOB), '/live', 'store',
                 'present', 'worktree', 1, 7)",
        [p.0],
    )
    .unwrap();
    assert!(gather(&conn, p).unwrap().is_ok());
}

/// §31.8's four `None` gates, each for its own reason.
#[test]
fn the_four_gates_each_answer_for_their_own_reason() {
    let (_d, conn) = fresh();

    conn.execute(
        "INSERT INTO project (name, seed_basename, authored_by_user, is_reference,
                              created_at, updated_at)
         VALUES ('ref', 'ref', 0, 1, 1, 1)",
        [],
    )
    .unwrap();
    let reference = ProjectId(conn.last_insert_rowid());
    copy(&conn, reference, "present", Some(1));
    assert_eq!(
        gather(&conn, reference).unwrap().err(),
        Some(NotScorable::Reference)
    );

    conn.execute(
        "INSERT INTO project (name, seed_basename, created_at, updated_at)
         VALUES ('unauthored', 'unauthored', 1, 1)",
        [],
    )
    .unwrap();
    let unauthored = ProjectId(conn.last_insert_rowid());
    copy(&conn, unauthored, "present", Some(1));
    assert_eq!(
        gather(&conn, unauthored).unwrap().err(),
        Some(NotScorable::AuthorshipNotComputed),
        "completion runs after J1.5, or it lights ticks on a repository about to be excluded"
    );

    conn.execute(
        "INSERT INTO project (name, seed_basename, authored_by_user, created_at, updated_at)
         VALUES ('uncloned', 'uncloned', 1, 1, 1)",
        [],
    )
    .unwrap();
    let uncloned = ProjectId(conn.last_insert_rowid());
    assert_eq!(
        gather(&conn, uncloned).unwrap().err(),
        Some(NotScorable::NotCloned)
    );
}

/// The `remote` predicate: a copy whose refstate was never persisted has not been looked at.
#[test]
fn remote_is_unobserved_until_a_refstate_is_persisted() {
    let (_d, conn) = fresh();
    conn.execute(
        "INSERT INTO project (name, seed_basename, authored_by_user, created_at, updated_at)
         VALUES ('unlooked', 'unlooked', 1, 1, 1)",
        [],
    )
    .unwrap();
    let p = ProjectId(conn.last_insert_rowid());
    copy(&conn, p, "present", None);
    assert_eq!(got(&conn, p).remote_configured, None);

    let q = scorable(&conn, "looked");
    assert_eq!(got(&conn, q).remote_configured, Some(false));
    conn.execute(
        "UPDATE project SET remote_key = 'forge/owner/name' WHERE id = ?1",
        [q.0],
    )
    .unwrap();
    assert_eq!(got(&conn, q).remote_configured, Some(true));
}

/// The forge accessor hands back values and **no verdict over them**.
#[test]
fn the_remote_accessor_returns_the_description_and_the_topic_count() {
    let (_d, conn) = fresh();
    let p = scorable(&conn, "bound");
    conn.execute(
        "UPDATE project SET remote_key = 'forge/o/n', provider = 'forge',
                provider_repo_id = '7' WHERE id = ?1",
        [p.0],
    )
    .unwrap();
    // No row yet: not observed, and no description invented for it.
    let inputs = got(&conn, p);
    assert_eq!(inputs.forge_description, None);
    assert_eq!(inputs.topic_count, 0);
    assert!(inputs.has_remote);

    conn.execute(
        "INSERT INTO remote_repo (provider, provider_repo_id, description, observed_at)
         VALUES ('forge', '7', 'a shaped description', 50)",
        [],
    )
    .unwrap();
    for topic in ["alpha", "beta"] {
        conn.execute(
            "INSERT INTO remote_topic (provider, provider_repo_id, topic)
             VALUES ('forge', '7', ?1)",
            [topic],
        )
        .unwrap();
    }
    let inputs = got(&conn, p);
    assert_eq!(
        inputs.forge_description.as_deref(),
        Some("a shaped description")
    );
    assert_eq!(inputs.topic_count, 2);
}

/// The stored user ruling is three-valued and lands on the right key.
#[test]
fn a_stored_user_ruling_reaches_its_own_key() {
    let (_d, conn) = fresh();
    let p = scorable(&conn, "ruled");
    conn.execute(
        "INSERT INTO project_check (project_id, check_key, state, user_na, observed_at)
         VALUES (?1, 'tests', 'na', 1, 5), (?1, 'ci', 'pass', 0, 5)",
        [p.0],
    )
    .unwrap();
    let na = got(&conn, p).user_na;
    let index = |key: CompletionCheck| CompletionCheck::ALL.iter().position(|k| *k == key).unwrap();
    assert_eq!(na[index(CompletionCheck::Tests)], Some(true));
    assert_eq!(na[index(CompletionCheck::Ci)], Some(false));
    assert_eq!(na[index(CompletionCheck::Readme)], None);
}
