#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

//! §11.1's two drawn controls, TRUST THIS REPOSITORY and TRY AGAIN.

use codotheca_core::index::Index;
use codotheca_core::protocol::{LocationId, ProjectId};
use codotheca_core::surfaces::repair;

fn seed(conn: &rusqlite::Connection) {
    conn.execute_batch(
        "INSERT INTO project (id, name, seed_basename, created_at, updated_at, last_touched_at,
                              error_kind, error_detail, error_at)
         VALUES (1, 'alpha', 'alpha', 1, 1, 1, 'PERMISSION_DENIED', 'EACCES', 10),
                (2, 'bravo', 'bravo', 1, 1, 1, 'PERMISSION_DENIED', 'EACCES', 10);
         INSERT INTO location (id, project_id, kind, distro, path_bytes, path_key,
                               path_display, volume_key, store_key, presence, repo_kind)
         VALUES (1, 1, 'linux', '', X'2f61', X'2f61', '<root>/alpha', 'v', 's', 'present',
                 'worktree');
         INSERT INTO project_job_state (project_id, job, state, fail_count, reason, at)
         VALUES (1, 'j2', 'failed', 3, 'permission denied', 10),
                (1, 'j3', 'deferred_slow', 1, 'over budget', 10),
                (1, 'j1', 'ok', 0, NULL, 10),
                (2, 'j2', 'failed', 1, 'permission denied', 10);",
    )
    .expect("seed");
}

fn seeded(dir: &std::path::Path) -> Index {
    let index = Index::open(dir).expect("open");
    seed(index.conn());
    index
}

#[test]
fn trust_writes_only_codothecas_own_row() {
    let dir = tempfile::tempdir().expect("tempdir");
    let index = seeded(dir.path());

    assert!(repair::set_trusted(index.conn(), LocationId(1), 555).expect("trust"));
    let at: Option<i64> = index
        .conn()
        .query_row("SELECT trusted_at FROM location WHERE id = 1", [], |r| {
            r.get(0)
        })
        .expect("read back");
    assert_eq!(at, Some(555));
    // §17: nothing else moves — and nothing anywhere writes the user's git config.
    let paths: i64 = index
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM location WHERE path_display <> '<root>/alpha'",
            [],
            |r| r.get(0),
        )
        .expect("count");
    assert_eq!(paths, 0);
}

#[test]
fn trusting_a_location_that_is_gone_reports_it_rather_than_inventing_a_row() {
    let dir = tempfile::tempdir().expect("tempdir");
    let index = seeded(dir.path());
    assert!(!repair::set_trusted(index.conn(), LocationId(99), 555).expect("no such location"));
    let rows: i64 = index
        .conn()
        .query_row("SELECT COUNT(*) FROM location", [], |r| r.get(0))
        .expect("count");
    assert_eq!(rows, 1, "a missing id creates nothing");
}

#[test]
fn a_missing_location_is_a_path_gone_failure_that_definitely_did_not_take_effect() {
    let dir = tempfile::tempdir().expect("tempdir");
    let index = seeded(dir.path());
    let ctx = codotheca_core::surfaces::SurfaceCtx {
        index: &index,
        now: 555,
    };
    let failure = repair::handle_set_trusted(&ctx, serde_json::json!({ "locationId": 99 }))
        .expect_err("gone");
    assert_eq!(failure.code, codotheca_core::protocol::ErrorCode::PathGone);
    assert_eq!(
        failure.outcome, None,
        "None is `definitely did not take effect`; there is no `failed` wire value"
    );
}

/// **R111.** TRY AGAIN revives the sync ledger too, and a deferred account is not a dead end.
///
/// §21.4 defers a sync task after three transient failures and leaves it only through a revival
/// cause. Every one of those causes had test-only callers, so a listing that failed three times was
/// left with no user-reachable way out — while a deferred *job* had this very button. The same
/// guarantee, written twice and wired once.
///
/// **The account's two tasks as well as the project's one**, because a project's remote facts are
/// unreachable while the listing that binds it is deferred: reviving `project_remote` alone would
/// be a button that reports success and changes nothing.
///
/// **And nobody else's.** The fixture is built so that the owning account's id **collides with
/// another project's id** — `key` is polymorphic, so a revival without a task filter moves
/// whichever row happens to share that integer, which is the defect `delete_account_tasks` was
/// written against. A first version of this test put the account and the project at the same id
/// and the collision was unobservable: dropping the filter stayed green.
#[test]
fn try_again_revives_this_accounts_deferred_sync_and_no_others() {
    let dir = tempfile::tempdir().expect("tempdir");
    let index = seeded(dir.path());
    index
        .conn()
        .execute_batch(
            // Account 1 is on another host, so `account_for_project` — which matches provider and
            // host and takes the lowest enabled id — resolves project 1 to account **2**.
            "INSERT INTO account (id, provider, host, login, auth_kind, scope_tier,
                                  granted_scopes, token_ref, is_enabled, connected_at)
             VALUES (1, 'github', 'other.example.invalid', 'owner', 'device', 'private',
                     '[\"repo\"]', 'github:other.example.invalid:owner', 1, 5),
                    (2, 'github', 'forge.example.invalid', 'other', 'device', 'private',
                     '[\"repo\"]', 'github:forge.example.invalid:other', 1, 5);
             UPDATE project SET remote_key = 'forge.example.invalid/other/alpha',
                                provider = 'github', provider_repo_id = '7',
                                remote_link_basis = 'provider_id'
              WHERE id = 1;
             INSERT INTO sync_task_state (task, key, state, fail_count, reason, at, not_before)
             VALUES ('account_repos',  2, 'deferred', 3, 'offline', 10, 0),
                    ('rename_probe',   2, 'deferred', 3, 'offline', 10, 0),
                    ('project_remote', 1, 'deferred', 3, 'offline', 10, 0),
                    ('account_repos',  1, 'deferred', 3, 'offline', 10, 0),
                    ('project_remote', 2, 'deferred', 3, 'offline', 10, 0),
                    ('account_repos',  9, 'blocked',  0, 'token_invalid', 10, 0);",
        )
        .expect("seed sync rows");

    repair::requeue(index.conn(), ProjectId(1), 777).expect("requeue");

    let state_of = |task: &str, key: i64| -> String {
        index
            .conn()
            .query_row(
                "SELECT state FROM sync_task_state WHERE task = ?1 AND key = ?2",
                rusqlite::params![task, key],
                |r| r.get(0),
            )
            .expect("row")
    };
    for (task, key) in [
        ("account_repos", 2),
        ("rename_probe", 2),
        ("project_remote", 1),
    ] {
        assert_eq!(
            state_of(task, key),
            "queued",
            "{task} for this project's own account is still a dead end"
        );
    }
    assert_eq!(
        state_of("project_remote", 2),
        "deferred",
        "the account's id is 2 and so is another project's — a revival with no task filter \
         reaches a project this button was not pressed on"
    );
    assert_eq!(
        state_of("account_repos", 1),
        "deferred",
        "TRY AGAIN on one tile means one account, never every forge connected"
    );
    assert_eq!(
        state_of("account_repos", 9),
        "blocked",
        "a blocked row is left only through an account change, never by a button"
    );

    let (reason, fails): (String, i64) = index
        .conn()
        .query_row(
            "SELECT reason, fail_count FROM sync_task_state
              WHERE task = 'account_repos' AND key = 2",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("row");
    assert_eq!(reason, "user_requested", "§21.4 names the cause");
    assert_eq!(fails, 0, "the count that deferred it is cleared");
}

#[test]
fn requeue_touches_one_project_and_leaves_every_other_alone() {
    let dir = tempfile::tempdir().expect("tempdir");
    let index = seeded(dir.path());

    let moved = repair::requeue(index.conn(), ProjectId(1), 777).expect("requeue");
    assert_eq!(
        moved, 2,
        "the failed row and the deferred row, not the ok one"
    );

    let queued: Vec<String> = index
        .conn()
        .prepare(
            "SELECT job FROM project_job_state
             WHERE project_id = 1 AND state = 'queued' ORDER BY job",
        )
        .expect("prepare")
        .query_map([], |r| r.get(0))
        .expect("query")
        .collect::<Result<_, _>>()
        .expect("rows");
    assert_eq!(queued, vec!["j2".to_owned(), "j3".to_owned()]);

    let other: String = index
        .conn()
        .query_row(
            "SELECT state FROM project_job_state WHERE project_id = 2",
            [],
            |r| r.get(0),
        )
        .expect("other project");
    assert_eq!(
        other, "failed",
        "TRY AGAIN on one tile means one repository, never scan.start"
    );
    let other_error: Option<String> = index
        .conn()
        .query_row("SELECT error_kind FROM project WHERE id = 2", [], |r| {
            r.get(0)
        })
        .expect("other project error");
    assert_eq!(
        other_error.as_deref(),
        Some("PERMISSION_DENIED"),
        "the neighbour's never-succeeded state is not cleared either"
    );
}

#[test]
fn requeue_clears_the_fail_count_so_the_backoff_does_not_defeat_the_button() {
    let dir = tempfile::tempdir().expect("tempdir");
    let index = seeded(dir.path());
    repair::requeue(index.conn(), ProjectId(1), 777).expect("requeue");
    let fails: i64 = index
        .conn()
        .query_row(
            "SELECT MAX(fail_count) FROM project_job_state WHERE project_id = 1",
            [],
            |r| r.get(0),
        )
        .expect("read");
    assert_eq!(fails, 0);
}

#[test]
fn requeue_clears_the_never_succeeded_state_it_is_offered_from() {
    let dir = tempfile::tempdir().expect("tempdir");
    let index = seeded(dir.path());
    repair::requeue(index.conn(), ProjectId(1), 777).expect("requeue");
    let row: (Option<String>, Option<String>, Option<i64>) = index
        .conn()
        .query_row(
            "SELECT error_kind, error_detail, error_at FROM project WHERE id = 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .expect("read");
    assert_eq!(
        row,
        (None, None, None),
        "the explained error is what TRY AGAIN is offered from, so it clears with the retry"
    );
}

#[test]
fn requeuing_a_project_with_nothing_wrong_moves_nothing_and_is_not_an_error() {
    let dir = tempfile::tempdir().expect("tempdir");
    let index = seeded(dir.path());
    index
        .conn()
        .execute(
            "UPDATE project_job_state SET state = 'ok' WHERE project_id = 1",
            [],
        )
        .expect("all well");
    assert_eq!(
        repair::requeue(index.conn(), ProjectId(1), 777).expect("requeue"),
        0
    );
}
