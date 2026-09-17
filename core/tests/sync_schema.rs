#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! §21.13's two tables, asserted against a **real migrated database** rather than against the
//! DDL text — a test that reads the SQL restates it, and a restatement cannot disagree with the
//! thing it restates.
//!
//! Covers **AC-P2-21-1**, **AC-P2-21-2** and **AC-P2-21-7**, plus the two halves R69 leaves to
//! this plan: the wire/Rust slug mirror, and the disconnect proven *behaviourally* with the
//! colliding key that a bare `key = <account id>` delete gets wrong.

use codotheca_core::accounts::store::{delete_account, insert_account, NewAccount};
use codotheca_core::jobs::JobKind;
use codotheca_core::protocol::{AccountId, AuthKind, ScopeTier, SyncTaskKind, SyncTaskState};
use codotheca_core::sync::state::{state_slug, SYNC_STATES};
use codotheca_core::sync::store::delete_account_tasks;
use codotheca_core::sync::task::kind_slug;
use codotheca_core::testing::TempIndex;

const NOW: i64 = 1_800_000_000;

fn seed_account(index: &mut codotheca_core::index::Index, login: &str) -> AccountId {
    index
        .with_tx(|tx| {
            Ok(insert_account(
                tx,
                &NewAccount {
                    provider: "github".to_owned(),
                    host: "api.example.invalid".to_owned(),
                    login: login.to_owned(),
                    display_name: None,
                    auth_kind: AuthKind::Device,
                    scope_tier: ScopeTier::Public,
                    granted_scopes: vec!["repo".to_owned()],
                    token_ref: format!("github:api.example.invalid:{login}"),
                },
                NOW,
            )
            .expect("account inserted"))
        })
        .expect("transaction")
}

/// **AC-P2-21-1.** The two vocabularies share no slug, and both counts are printed: a zero on
/// either side means the walk found nothing and proved nothing.
///
/// This is what stops a later *"add a `j7` for symmetry"*: the two enums are scheduled by two
/// different runners against two different tables, and one slug appearing in both would make a
/// grep over either column ambiguous for good.
#[test]
fn the_sync_and_job_vocabularies_are_disjoint() {
    let jobs: Vec<&str> = JobKind::ALL.iter().map(|k| k.slug()).collect();
    let syncs: Vec<&str> = SyncTaskKind::ALL.iter().map(|k| kind_slug(*k)).collect();
    eprintln!(
        "sync_schema: {} job slugs, {} sync task slugs",
        jobs.len(),
        syncs.len()
    );
    assert_eq!(jobs.len(), 7, "the job vocabulary");
    assert_eq!(syncs.len(), 3, "the sync task vocabulary");
    for sync in &syncs {
        assert!(
            !jobs.contains(sync),
            "{sync} is both a job slug and a sync task slug"
        );
    }
}

/// **AC-P2-21-2.** Every slug the writer emits is accepted by its column's CHECK, proven by
/// insertion against a migrated database and printing the number inserted.
///
/// The shape is `every_job_state_slug_is_accepted_by_the_column`, whose real home is
/// `core/tests/jobs_state.rs:45` — `core/src/jobs/mod.rs:200-206` is the doc comment *about* it.
/// It is the pattern that caught R26: a DDL CHECK rejecting the values its own core emits.
#[test]
fn every_sync_slug_is_accepted_by_its_column() {
    let mut fixture = TempIndex::new();
    let mut inserted = 0_usize;
    fixture
        .index_mut()
        .with_tx(|tx| {
            // Every (kind, state) pair, so neither column is exercised against one value of the
            // other. `key` is the row index so the partial unique index admits them all.
            let mut key = 0_i64;
            for kind in SyncTaskKind::ALL {
                for state in SYNC_STATES {
                    key += 1;
                    tx.execute(
                        "INSERT INTO sync_task_state (task, key, state, at) VALUES (?1, ?2, ?3, ?4)",
                        rusqlite::params![kind_slug(kind), key, state_slug(state), NOW],
                    )
                    .unwrap_or_else(|e| {
                        panic!(
                            "the CHECK rejected task={} state={}: {e}",
                            kind_slug(kind),
                            state_slug(state)
                        )
                    });
                    inserted += 1;
                }
            }
            Ok(())
        })
        .expect("transaction");
    eprintln!("sync_schema: {inserted} (task, state) pairs accepted by their CHECKs");
    assert_eq!(inserted, 18, "3 kinds x 6 states");
}

/// **R24's mirror.** The Rust spelling and the wire spelling are one value with one owner,
/// asserted by **reading the other side** through serde rather than by restating the list.
#[test]
fn every_slug_round_trips_through_the_generated_enum() {
    for kind in SyncTaskKind::ALL {
        let wire = serde_json::to_value(kind).expect("serialises");
        assert_eq!(
            wire,
            serde_json::Value::String(kind_slug(kind).to_owned()),
            "kind_slug disagrees with the generated serde rename"
        );
        let back: SyncTaskKind = serde_json::from_value(wire).expect("deserialises");
        assert_eq!(back, kind);
    }
    for state in SYNC_STATES {
        let wire = serde_json::to_value(state).expect("serialises");
        assert_eq!(
            wire,
            serde_json::Value::String(state_slug(state).to_owned()),
            "state_slug disagrees with the generated serde rename"
        );
        let back: SyncTaskState = serde_json::from_value(wire).expect("deserialises");
        assert_eq!(back, state);
    }
}

/// **AC-P2-21-7.** The per-IP pool is one row.
///
/// A `STRICT` table makes every PRIMARY KEY column implicitly NOT NULL, so
/// `PRIMARY KEY (account_id, resource)` would reject the per-IP row outright; a plain
/// `UNIQUE(account_id, resource)` treats NULLs as distinct and would admit **unlimited**
/// duplicate per-IP rows, at which point "the last observation" stops being a single row. The
/// paired partial indexes are what make the NULL key mean one pool, and that is asserted here
/// against the migrated database rather than by reading the DDL.
#[test]
fn two_per_ip_budget_rows_for_one_resource_cannot_both_exist() {
    let mut fixture = TempIndex::new();
    let a = seed_account(fixture.index_mut(), "one");
    let b = seed_account(fixture.index_mut(), "two");

    fixture
        .index_mut()
        .with_tx(|tx| {
            tx.execute(
                "INSERT INTO sync_budget (account_id, resource, observed_at) VALUES (NULL, 'core', ?1)",
                [NOW],
            )
            .expect("the first per-IP row");
            let second = tx.execute(
                "INSERT INTO sync_budget (account_id, resource, observed_at) VALUES (NULL, 'core', ?1)",
                [NOW + 1],
            );
            assert!(
                second.is_err(),
                "a second per-IP row for the same resource was admitted, so the pool is not one row"
            );

            // Two *accounts* on the same resource are two pools and must both exist.
            tx.execute(
                "INSERT INTO sync_budget (account_id, resource, observed_at) VALUES (?1, 'core', ?2)",
                rusqlite::params![a.0, NOW],
            )
            .expect("account a's row");
            tx.execute(
                "INSERT INTO sync_budget (account_id, resource, observed_at) VALUES (?1, 'core', ?2)",
                rusqlite::params![b.0, NOW],
            )
            .expect("account b's row");
            Ok(())
        })
        .expect("transaction");

    let rows: i64 = fixture
        .index()
        .conn()
        .query_row("SELECT count(*) FROM sync_budget", [], |r| r.get(0))
        .expect("counted");
    assert_eq!(rows, 3, "one per-IP row and two account rows");
}

/// **The disconnect, proven behaviourally.**
///
/// The colliding integer is the point. `key` is polymorphic, so a bare
/// `DELETE FROM sync_task_state WHERE key = <account id>` passes every enumeration over the
/// census while deleting a *project*'s rows — and a test that used distinct ids would pass
/// against exactly that delete. The per-IP budget row must survive too: it belongs to no
/// account, so no account's disconnect may take it.
#[test]
fn disconnecting_clears_only_that_accounts_sync_rows() {
    let mut fixture = TempIndex::new();
    let account = seed_account(fixture.index_mut(), "owner");
    // The project id that collides with the account id. Both are 1 in a fresh index, which is
    // the realistic case rather than a contrived one.
    let colliding = account.0;

    fixture
        .index_mut()
        .with_tx(|tx| {
            for (task, key) in [
                ("account_repos", account.0),
                ("project_remote", colliding),
                ("rename_probe", colliding),
            ] {
                tx.execute(
                    "INSERT INTO sync_task_state (task, key, state, at) VALUES (?1, ?2, 'queued', ?3)",
                    rusqlite::params![task, key, NOW],
                )
                .expect("seeded");
            }
            tx.execute(
                "INSERT INTO sync_budget (account_id, resource, observed_at) VALUES (?1, 'core', ?2)",
                rusqlite::params![account.0, NOW],
            )
            .expect("the account's budget row");
            tx.execute(
                "INSERT INTO sync_budget (account_id, resource, observed_at) VALUES (NULL, 'core', ?1)",
                [NOW],
            )
            .expect("the per-IP row");
            Ok(())
        })
        .expect("seed transaction");

    fixture
        .index_mut()
        .with_tx(|tx| {
            // `delete_account` is what the disconnect command runs, so it is what this asserts —
            // calling `delete_account_tasks` here would prove the helper and not the path.
            delete_account(tx, account).expect("account deleted");
            Ok(())
        })
        .expect("delete transaction");

    let conn = fixture.index().conn();
    let surviving: Vec<String> = conn
        .prepare("SELECT task FROM sync_task_state ORDER BY task")
        .expect("prepared")
        .query_map([], |r| r.get::<_, String>(0))
        .expect("query")
        .map(Result::unwrap)
        .collect();
    assert_eq!(
        surviving,
        ["project_remote", "rename_probe"],
        "a project-keyed row was deleted by an account's disconnect"
    );

    let budgets: Vec<Option<i64>> = conn
        .prepare("SELECT account_id FROM sync_budget")
        .expect("prepared")
        .query_map([], |r| r.get::<_, Option<i64>>(0))
        .expect("query")
        .map(Result::unwrap)
        .collect();
    assert_eq!(
        budgets,
        [None],
        "the account's budget row must cascade and the per-IP pool must not"
    );
}

/// The filtered delete, asserted on its own so its **count** is visible: `delete_account` above
/// proves the path, and this proves the primitive reports what it removed rather than reporting
/// success over zero rows.
#[test]
fn the_task_delete_is_filtered_by_task_and_reports_its_count() {
    let mut fixture = TempIndex::new();
    let account = seed_account(fixture.index_mut(), "owner");
    fixture
        .index_mut()
        .with_tx(|tx| {
            for (task, key) in [
                ("account_repos", account.0),
                ("project_remote", account.0),
                ("rename_probe", account.0),
            ] {
                tx.execute(
                    "INSERT INTO sync_task_state (task, key, state, at) VALUES (?1, ?2, 'queued', ?3)",
                    rusqlite::params![task, key, NOW],
                )
                .expect("seeded");
            }
            let removed = delete_account_tasks(tx, account).expect("filtered delete");
            eprintln!("sync_schema: delete_account_tasks removed {removed} row(s)");
            assert_eq!(removed, 1, "only the account_repos row is this account's");
            let left: i64 = tx
                .query_row("SELECT count(*) FROM sync_task_state", [], |r| r.get(0))
                .expect("counted");
            assert_eq!(left, 2, "the two project-keyed rows survive");
            Ok(())
        })
        .expect("transaction");
}
