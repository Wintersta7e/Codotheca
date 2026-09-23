//! `0008_accounts.sql` against the real schema.
//!
//! The point of this file is that it reads the *other side*: every variant the wire vocabulary
//! declares is inserted into the column that stores it, so a DDL `CHECK` and a generated enum
//! cannot drift the way R26 found them drifting. Four columns carry a `CHECK` whose values are
//! also schema enums — `auth_kind`, `scope_tier`, `sso_state` and `affiliation` — and one value
//! stated twice is this project's dominant defect class.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use codotheca_core::index::migrate::{apply_all, schema_version, MIGRATIONS};
use codotheca_core::index::{open_connection, Index};

/// The schema text, compiled in. `core/tests/project_page_writes.rs` sets the precedent: the
/// contract travels with the binary rather than being re-found at runtime.
const SCHEMA: &str = include_str!("../../protocol/schema/protocol.json");

fn fresh() -> (tempfile::TempDir, rusqlite::Connection) {
    let dir = tempfile::tempdir().unwrap();
    let mut conn = open_connection(&Index::db_path(dir.path())).unwrap();
    apply_all(&mut conn, MIGRATIONS).unwrap();
    (dir, conn)
}

/// Every variant the schema declares for `name`, in declaration order.
///
/// Panics rather than returning an empty vector for an absent type: a missing enum must fail
/// this test, not silently reduce it to a loop over nothing.
fn schema_variants(name: &str) -> Vec<String> {
    let doc: serde_json::Value = serde_json::from_str(SCHEMA).expect("protocol.json parses");
    let decl = doc["types"]
        .get(name)
        .unwrap_or_else(|| panic!("{name} is not declared in protocol.json"));
    assert_eq!(decl["kind"], "enum", "{name} is not an enum");
    let variants: Vec<String> = decl["variants"]
        .as_array()
        .unwrap_or_else(|| panic!("{name} declares no variants"))
        .iter()
        .map(|v| v.as_str().expect("a variant is a string").to_owned())
        .collect();
    assert!(!variants.is_empty(), "{name} declares no variants");
    variants
}

fn insert_project(conn: &rusqlite::Connection, name: &str) -> i64 {
    conn.execute(
        "INSERT INTO project (name, seed_basename, created_at, updated_at)
         VALUES (?1, ?1, 0, 0)",
        [name],
    )
    .unwrap();
    conn.last_insert_rowid()
}

/// `login` is the only thing that varies, so the UNIQUE triple is satisfied without the test
/// having to think about it. Invented logins and an `example.invalid` host throughout.
fn insert_account(
    conn: &rusqlite::Connection,
    login: &str,
    auth_kind: &str,
    scope_tier: &str,
) -> rusqlite::Result<i64> {
    conn.execute(
        "INSERT INTO account
           (provider, host, login, auth_kind, scope_tier, granted_scopes, token_ref, connected_at)
         VALUES ('github', 'forge.example.invalid', ?1, ?2, ?3, '', ?4, 100)",
        rusqlite::params![
            login,
            auth_kind,
            scope_tier,
            format!("github:forge.example.invalid:{login}")
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// The literal moves with every migration, by reading the base value off the failure rather than
/// by writing a running total. `0008` put it at 8; `0009`'s `project` rebuild raised it to 9;
/// `0010`'s install tables raised it to 10; `0011`'s sync tables raised it to 11; `0012`'s
/// `project_job_state` rebuild raised it to 12; `0013`'s `xp_events` rebuild raised it to 13;
/// `0014`'s `sync_task_state` rebuild raises it to 14; `0015`'s `project_check` and
/// `location.tag_count` raise it to 15; `0016`'s `health_delta` rebuild raises it to 16.
/// The name deliberately does not carry the number, so raising it is one line.
#[test]
fn a_fresh_index_migrates_to_the_version_this_build_supports() {
    let (_dir, conn) = fresh();
    assert_eq!(schema_version(&conn).unwrap(), 16);
    assert_eq!(
        MIGRATIONS.last().map(|m| m.version),
        Some(codotheca_core::index::migrate::SUPPORTED_SCHEMA_VERSION),
        "the constant and the last migration are one value stated twice"
    );
}

#[test]
fn the_three_tables_exist_with_the_columns_the_section_states() {
    let (_dir, conn) = fresh();
    let columns = |table: &str| -> Vec<String> {
        let mut stmt = conn
            .prepare(&format!("SELECT name FROM pragma_table_info('{table}')"))
            .unwrap();
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .map(Result::unwrap)
            .collect::<Vec<_>>();
        assert!(!rows.is_empty(), "{table} does not exist");
        rows
    };

    assert_eq!(
        columns("account"),
        [
            "id",
            "provider",
            "host",
            "login",
            "display_name",
            "auth_kind",
            "scope_tier",
            "granted_scopes",
            "scopes_observed_at",
            "token_ref",
            "connected_at",
            "last_verified_at",
            "last_error_kind",
            "last_error_at",
            "is_enabled",
            // §21.7's listing validator, with no clock: an ETag is never rendered as a time.
            "listing_etag",
        ]
    );
    assert_eq!(
        columns("account_org"),
        [
            "account_id",
            "login",
            "is_enabled",
            "repo_count_seen",
            "sso_state",
            "observed_at",
        ]
    );
    assert_eq!(
        columns("project_account"),
        [
            "project_id",
            "account_id",
            "affiliation",
            "can_push",
            "observed_at",
            "last_seen_generation",
        ]
    );
}

/// R26's shape, held mechanically over all four columns at once. **It prints the number of
/// variants it inserted and fails at zero**: a run that inserted nothing would pass while
/// proving nothing, which is the failing-gate shape this project has already shipped once.
#[test]
fn every_schema_variant_of_the_four_enums_is_accepted_by_its_column() {
    let (_dir, conn) = fresh();
    let mut inserted = 0_usize;

    for (i, kind) in schema_variants("AuthKind").into_iter().enumerate() {
        insert_account(&conn, &format!("auth-{i}"), &kind, "public")
            .unwrap_or_else(|e| panic!("auth_kind {kind:?} was refused: {e}"));
        inserted += 1;
    }
    for (i, tier) in schema_variants("ScopeTier").into_iter().enumerate() {
        insert_account(&conn, &format!("tier-{i}"), "device", &tier)
            .unwrap_or_else(|e| panic!("scope_tier {tier:?} was refused: {e}"));
        inserted += 1;
    }

    let account = insert_account(&conn, "orgs", "device", "private").unwrap();
    for (i, state) in schema_variants("SsoState").into_iter().enumerate() {
        conn.execute(
            "INSERT INTO account_org (account_id, login, sso_state) VALUES (?1, ?2, ?3)",
            rusqlite::params![account, format!("org-{i}"), state],
        )
        .unwrap_or_else(|e| panic!("sso_state {state:?} was refused: {e}"));
        inserted += 1;
    }

    for (i, affiliation) in schema_variants("Affiliation").into_iter().enumerate() {
        let project = insert_project(&conn, &format!("p-{i}"));
        conn.execute(
            "INSERT INTO project_account
               (project_id, account_id, affiliation, can_push, observed_at)
             VALUES (?1, ?2, ?3, 1, 100)",
            rusqlite::params![project, account, affiliation],
        )
        .unwrap_or_else(|e| panic!("affiliation {affiliation:?} was refused: {e}"));
        inserted += 1;
    }

    // stderr, on `git_readonly.rs`'s precedent: stdout carries protocol frames and a test that
    // writes there is one `println!` away from teaching the wrong habit.
    eprintln!("accounts_migration: inserted {inserted} declared enum variants");
    assert!(
        inserted > 0,
        "the loop inserted nothing, so it proved nothing"
    );
}

/// §20.4's gate is off by default, and the default lives in the DDL rather than in whatever
/// inserts the row — an org enumerated by a path that forgot to pass 0 would otherwise admit
/// thousands of tiles the user never asked for.
#[test]
fn an_org_is_disabled_by_default_and_its_count_is_nullable() {
    let (_dir, conn) = fresh();
    let account = insert_account(&conn, "defaults", "device", "private").unwrap();
    conn.execute(
        "INSERT INTO account_org (account_id, login) VALUES (?1, 'an-org')",
        [account],
    )
    .unwrap();

    let (enabled, count): (i64, Option<i64>) = conn
        .query_row(
            "SELECT is_enabled, repo_count_seen FROM account_org WHERE account_id = ?1",
            [account],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(enabled, 0, "the org gate is off by default");
    assert_eq!(count, None, "an uncounted org is unknown, never zero");
}

#[test]
fn the_unique_triple_refuses_a_repeat_and_admits_a_second_login() {
    let (_dir, conn) = fresh();
    insert_account(&conn, "first", "device", "public").unwrap();

    let repeat = insert_account(&conn, "first", "pat", "private");
    assert!(
        repeat.is_err(),
        "(provider, host, login) is the only uniqueness constraint and it must bite"
    );

    insert_account(&conn, "second", "device", "public")
        .expect("a second, different login is a second account, not a conflict");
    let rows: i64 = conn
        .query_row("SELECT count(*) FROM account", [], |row| row.get(0))
        .unwrap();
    assert_eq!(rows, 2);
}

/// §20.9: disconnect cascades the two side tables and touches **no** `project` row. Asserted as
/// a full id set rather than a count, because a count survives one project being deleted and
/// another created.
#[test]
fn deleting_an_account_cascades_its_side_tables_and_no_project_row() {
    let (_dir, conn) = fresh();
    let on: i64 = conn
        .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
        .unwrap();
    assert_eq!(
        on, 1,
        "open_connection sets foreign_keys=ON before we rely on the cascade"
    );

    let account = insert_account(&conn, "leaving", "device", "private").unwrap();
    let kept = insert_account(&conn, "staying", "device", "public").unwrap();
    let cloned = insert_project(&conn, "cloned");
    let zero_location = insert_project(&conn, "zero-location");
    conn.execute(
        "INSERT INTO account_org (account_id, login) VALUES (?1, 'an-org')",
        [account],
    )
    .unwrap();
    for project in [cloned, zero_location] {
        conn.execute(
            "INSERT INTO project_account
               (project_id, account_id, affiliation, can_push, observed_at)
             VALUES (?1, ?2, 'owner', 1, 100)",
            rusqlite::params![project, account],
        )
        .unwrap();
    }

    let project_ids = |conn: &rusqlite::Connection| -> Vec<i64> {
        let mut stmt = conn.prepare("SELECT id FROM project ORDER BY id").unwrap();
        let ids = stmt
            .query_map([], |row| row.get::<_, i64>(0))
            .unwrap()
            .map(Result::unwrap)
            .collect::<Vec<_>>();
        ids
    };
    let before = project_ids(&conn);
    assert_eq!(before.len(), 2);

    conn.execute("DELETE FROM account WHERE id = ?1", [account])
        .unwrap();

    let orgs: i64 = conn
        .query_row(
            "SELECT count(*) FROM account_org WHERE account_id = ?1",
            [account],
            |row| row.get(0),
        )
        .unwrap();
    let links: i64 = conn
        .query_row(
            "SELECT count(*) FROM project_account WHERE account_id = ?1",
            [account],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(orgs, 0, "account_org cascaded");
    assert_eq!(links, 0, "project_account cascaded");
    assert_eq!(
        project_ids(&conn),
        before,
        "disconnect deletes no project row, ever"
    );
    let remaining: i64 = conn
        .query_row(
            "SELECT count(*) FROM account WHERE id = ?1",
            [kept],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(remaining, 1, "the other account is untouched");
}
