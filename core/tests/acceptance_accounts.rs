#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use codotheca_core::protocol::{Affiliation, ScopeTier};
use codotheca_core::provider::admit::{admit, Admission};
use codotheca_core::provider::listing::RepoListing;

const PROVIDER: &str = "forge.example.invalid";

fn key(id: &str) -> (String, String) {
    (PROVIDER.to_owned(), id.to_owned())
}

fn orgs(logins: &[&str]) -> BTreeSet<String> {
    logins.iter().map(|login| (*login).to_owned()).collect()
}

fn repo(id: &str, owner: &str, name: &str, can_push: Option<bool>) -> RepoListing {
    RepoListing {
        provider: PROVIDER,
        provider_repo_id: id.to_owned(),
        clone_url: format!("https://forge.example.invalid/{owner}/{name}.git"),
        owner: owner.to_owned(),
        name: name.to_owned(),
        can_push,
        is_fork: false,
        fork_parent_clone_url: None,
        is_archived: false,
        is_private: false,
        in_org: None,
    }
}

fn private_repo(id: &str, owner: &str, can_push: Option<bool>) -> RepoListing {
    let mut listing = repo(id, owner, id, can_push);
    listing.is_private = true;
    listing
}

fn fork_repo(id: &str, owner: &str, can_push: Option<bool>) -> RepoListing {
    let mut listing = repo(id, owner, id, can_push);
    listing.is_fork = true;
    listing.fork_parent_clone_url =
        Some(format!("https://forge.example.invalid/upstream/{id}.git"));
    listing
}

fn org_repo(id: &str, org: &str, can_push: Option<bool>) -> RepoListing {
    let mut listing = repo(id, org, id, can_push);
    listing.in_org = Some(org.to_owned());
    listing
}

fn admitted_ids(admission: &Admission) -> Vec<&str> {
    admission
        .admitted
        .keys()
        .map(|(_, provider_repo_id)| provider_repo_id.as_str())
        .collect()
}

fn assert_accounted(admission: &Admission, listing_count: usize) {
    let accounted = admission.admitted.len()
        + admission.skipped_unknown_permission
        + admission.skipped_no_push
        + admission.skipped_private_under_public_tier
        + admission.skipped_org_not_enabled;
    assert_eq!(
        accounted, listing_count,
        "admission outcomes must account for every non-duplicate listing"
    );
}

#[test]
fn ac_p2_20_1_admission_is_push_permission() {
    let listings = vec![
        repo("01-owned-public", "case-viewer", "owned-public", Some(true)),
        private_repo("02-owned-private", "case-viewer", Some(true)),
        fork_repo("03-owned-fork", "case-viewer", Some(true)),
        repo("04-push-collab", "sample-user", "push-collab", Some(true)),
        repo("05-read-only", "sample-user", "read-only", Some(false)),
        repo("06-starred", "sample-user", "starred", Some(false)),
        repo("07-unknown", "sample-user", "unknown", None),
    ];

    let admission = admit(
        &listings,
        "case-viewer",
        ScopeTier::Private,
        &BTreeSet::new(),
    );

    assert_eq!(admission.admitted.len(), 4);
    assert_eq!(
        admitted_ids(&admission),
        [
            "01-owned-public",
            "02-owned-private",
            "03-owned-fork",
            "04-push-collab",
        ]
    );
    assert!(!admission.admitted.contains_key(&key("05-read-only")));
    assert!(!admission.admitted.contains_key(&key("06-starred")));
    assert!(!admission.admitted.contains_key(&key("07-unknown")));
    assert_eq!(
        admission.skipped_unknown_permission,
        1,
        "missing permission must stay unknown; admitted count was {}",
        admission.admitted.len()
    );
    assert_eq!(admission.skipped_no_push, 2);
    assert_eq!(admission.skipped_private_under_public_tier, 0);
    assert_eq!(admission.skipped_org_not_enabled, 0);
    assert_accounted(&admission, listings.len());
}

#[test]
fn duplicate_provider_repo_key_collapses_to_one_admission() {
    let first = repo("shared-remote", "case-viewer", "shared-remote", Some(true));
    let mut second = repo("shared-remote", "case-viewer", "shared-remote", Some(true));
    second.clone_url = "https://forge.example.invalid/CASE-VIEWER/shared-remote.git".to_owned();

    let admission = admit(
        &[first, second],
        "case-viewer",
        ScopeTier::Private,
        &BTreeSet::new(),
    );

    assert_eq!(admission.admitted.len(), 1);
    assert!(admission.admitted.contains_key(&key("shared-remote")));
}

#[test]
fn org_repos_require_enabled_org_and_push_permission() {
    let push_repo = org_repo("org-push", "sample-org", Some(true));
    let read_repo = org_repo("org-read", "sample-org", Some(false));

    let disabled = admit(
        std::slice::from_ref(&push_repo),
        "case-viewer",
        ScopeTier::Private,
        &BTreeSet::new(),
    );
    assert_eq!(
        disabled.skipped_org_not_enabled, 1,
        "disabled org entries must be counted before permission checks"
    );
    assert_eq!(disabled.admitted.len(), 0);
    assert_eq!(disabled.skipped_no_push, 0);

    let enabled = admit(
        &[push_repo, read_repo],
        "case-viewer",
        ScopeTier::Private,
        &orgs(&["sample-org"]),
    );
    assert_eq!(admitted_ids(&enabled), ["org-push"]);
    assert_eq!(enabled.skipped_org_not_enabled, 0);
    assert_eq!(enabled.skipped_no_push, 1);
    assert_eq!(
        enabled.admitted[&key("org-push")].affiliation,
        Affiliation::OrganizationMember
    );
}

#[test]
fn archived_repo_is_admitted_and_remote_flag_is_carried() {
    let mut listing = repo("archived", "case-viewer", "archived", Some(true));
    listing.is_archived = true;

    let admission = admit(
        &[listing],
        "case-viewer",
        ScopeTier::Private,
        &BTreeSet::new(),
    );

    let admitted = admission
        .admitted
        .get(&key("archived"))
        .expect("archived remote is admitted");
    assert!(admitted.listing.is_archived);
}

#[test]
fn private_repo_requires_private_tier() {
    let listing = private_repo("private-only", "case-viewer", Some(true));

    let public = admit(
        std::slice::from_ref(&listing),
        "case-viewer",
        ScopeTier::Public,
        &BTreeSet::new(),
    );
    assert_eq!(public.admitted.len(), 0);
    assert_eq!(public.skipped_private_under_public_tier, 1);
    assert_eq!(public.skipped_no_push, 0);

    let private = admit(
        &[listing],
        "case-viewer",
        ScopeTier::Private,
        &BTreeSet::new(),
    );
    assert_eq!(admitted_ids(&private), ["private-only"]);
}

#[test]
fn owned_fork_is_admitted_and_fork_state_is_carried() {
    let listing = fork_repo("owned-fork", "case-viewer", Some(true));

    let admission = admit(
        &[listing],
        "case-viewer",
        ScopeTier::Private,
        &BTreeSet::new(),
    );

    let admitted = admission
        .admitted
        .get(&key("owned-fork"))
        .expect("owned fork is admitted");
    assert!(admitted.listing.is_fork);
    assert_eq!(
        admitted.listing.fork_parent_clone_url,
        Some("https://forge.example.invalid/upstream/owned-fork.git".to_owned())
    );
}

#[test]
fn affiliation_uses_case_insensitive_viewer_and_org_precedence() {
    let owner = repo("owner-case", "Case-Viewer", "owner-case", Some(true));
    let org = org_repo("org-member", "sample-org", Some(true));

    let admission = admit(
        &[owner, org],
        "case-viewer",
        ScopeTier::Private,
        &orgs(&["sample-org"]),
    );

    assert_eq!(
        admission.admitted[&key("owner-case")].affiliation,
        Affiliation::Owner
    );
    assert_eq!(
        admission.admitted[&key("org-member")].affiliation,
        Affiliation::OrganizationMember
    );
}

#[test]
fn accounting_identity_covers_mixed_non_duplicate_listings() {
    let listings = vec![
        repo(
            "accounted-owned",
            "case-viewer",
            "accounted-owned",
            Some(true),
        ),
        repo(
            "accounted-read",
            "sample-user",
            "accounted-read",
            Some(false),
        ),
        repo(
            "accounted-unknown",
            "sample-user",
            "accounted-unknown",
            None,
        ),
        private_repo("accounted-private", "case-viewer", Some(true)),
        org_repo("accounted-org", "sample-org", Some(true)),
    ];

    let admission = admit(
        &listings,
        "case-viewer",
        ScopeTier::Public,
        &BTreeSet::new(),
    );

    assert_eq!(admission.admitted.len(), 1);
    assert_eq!(admission.skipped_no_push, 1);
    assert_eq!(admission.skipped_unknown_permission, 1);
    assert_eq!(admission.skipped_private_under_public_tier, 1);
    assert_eq!(admission.skipped_org_not_enabled, 1);
    assert_accounted(&admission, listings.len());
}

fn source_root(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join(name)
}

fn relative_to_source(path: &Path) -> String {
    path.strip_prefix(Path::new(env!("CARGO_MANIFEST_DIR")).join("src"))
        .unwrap_or(path)
        .display()
        .to_string()
}

fn rust_sources_under(dir: &Path, out: &mut Vec<(PathBuf, String)>) {
    for entry in std::fs::read_dir(dir).expect("source directory is readable") {
        let entry = entry.expect("a readable source entry");
        let kind = entry.file_type().expect("a source entry kind");
        let path = entry.path();
        if kind.is_dir() {
            rust_sources_under(&path, out);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            match std::fs::read_to_string(&path) {
                Ok(text) => out.push((path, text)),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => panic!("{}: {error}", path.display()),
            }
        }
    }
}

fn strip_comment_lines(source: &str) -> String {
    source
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn remote_archived_fact_is_never_written_to_project_archived_flag() {
    let mut sources = Vec::new();
    rust_sources_under(&source_root("provider"), &mut sources);
    rust_sources_under(&source_root("accounts"), &mut sources);
    eprintln!(
        "acceptance_accounts: archival write gate scanned {} source file(s)",
        sources.len()
    );
    assert!(
        !sources.is_empty(),
        "the archival write gate read no files, so it proved nothing"
    );

    let mut offenders = Vec::new();
    for (path, source) in sources {
        let code = strip_comment_lines(&source);
        let squashed = code.split_whitespace().collect::<Vec<_>>().join(" ");
        if squashed.contains("UPDATE project") && squashed.contains("is_archived") {
            offenders.push(format!(
                "{} names is_archived in an UPDATE project statement",
                relative_to_source(&path)
            ));
        }
        if code.contains("projects.setFlags") {
            offenders.push(format!(
                "{} calls projects.setFlags",
                relative_to_source(&path)
            ));
        }
    }

    assert!(
        offenders.is_empty(),
        "remote archival state must not write project.is_archived: {offenders:?}"
    );
}

/// Org logins are case-insensitive on the forge, so `enabled_orgs` must match that way too. A
/// case-sensitive lookup silently excludes every repository in an org the user had enabled, and
/// the exclusion looks exactly like a correct org gate from the outside.
#[test]
fn an_enabled_org_matches_regardless_of_case() {
    let listings = [org_repo("1", "an-org", Some(true))];
    let admission = admit(&listings, "someone", ScopeTier::Private, &orgs(&["An-Org"]));
    assert_eq!(
        admitted_ids(&admission),
        ["1"],
        "a case difference excluded an enabled org"
    );
    assert_eq!(admission.skipped_org_not_enabled, 0);
    assert_accounted(&admission, listings.len());
}

// ---------------------------------------------------------------------------
// Task 11 — disconnect: the keychain first, never a project row, R69's census.
// ---------------------------------------------------------------------------

use codotheca_core::accounts::keychain::TokenStore as _;
use codotheca_core::accounts::store::ACCOUNT_REFERENCING_TABLES;
use std::sync::Arc;

struct Disconnectable {
    dir: tempfile::TempDir,
    index: codotheca_core::index::Index,
    account: codotheca_core::protocol::AccountId,
    projects: Vec<i64>,
}

/// One account, one org, one cloned project and one zero-location project, both linked.
fn disconnectable(now: i64) -> Disconnectable {
    let dir = tempfile::tempdir().expect("tmp");
    let index = codotheca_core::index::Index::open_at(dir.path(), now).expect("index opens");
    let account = {
        let tx = index.conn().unchecked_transaction().expect("tx");
        let id = codotheca_core::accounts::store::insert_account(
            &tx,
            &codotheca_core::accounts::store::NewAccount {
                provider: "github".to_owned(),
                host: "forge.example.invalid".to_owned(),
                login: "octo".to_owned(),
                display_name: None,
                auth_kind: codotheca_core::protocol::AuthKind::Device,
                scope_tier: ScopeTier::Private,
                granted_scopes: vec!["read:user".to_owned()],
                token_ref: "github:forge.example.invalid:octo".to_owned(),
            },
            now,
        )
        .expect("account inserts");
        tx.execute(
            "INSERT INTO account_org (account_id, login) VALUES (?1, 'an-org')",
            [id.0],
        )
        .expect("org inserts");
        tx.commit().expect("commit");
        id
    };

    let mut projects = Vec::new();
    for name in ["cloned", "zero-location"] {
        index
            .conn()
            .execute(
                "INSERT INTO project (name, seed_basename, created_at, updated_at)
                 VALUES (?1, ?1, 0, 0)",
                [name],
            )
            .expect("project inserts");
        let project = index.conn().last_insert_rowid();
        index
            .conn()
            .execute(
                "INSERT INTO project_account
                   (project_id, account_id, affiliation, can_push, observed_at)
                 VALUES (?1, ?2, 'owner', 1, ?3)",
                rusqlite::params![project, account.0, now],
            )
            .expect("link inserts");
        projects.push(project);
    }
    Disconnectable {
        dir,
        index,
        account,
        projects,
    }
}

fn project_ids(index: &codotheca_core::index::Index) -> Vec<i64> {
    let mut stmt = index
        .conn()
        .prepare("SELECT id FROM project ORDER BY id")
        .expect("prepared");
    let ids = stmt
        .query_map([], |row| row.get::<_, i64>(0))
        .expect("query")
        .map(Result::unwrap)
        .collect::<Vec<_>>();
    ids
}

fn count(index: &codotheca_core::index::Index, sql: &str, id: i64) -> i64 {
    index
        .conn()
        .query_row(sql, [id], |row| row.get(0))
        .expect("countable")
}

/// **AC-P2-20-6.** Disconnect deletes the account and its two side tables and **zero `project`
/// rows** — asserted as a full id **set** before and after, not as a count, because a count
/// survives one project being deleted and another created.
#[test]
fn ac_p2_20_6_disconnect_deletes_no_project_row() {
    let fixture = disconnectable(1_000);
    let index = fixture.index;
    let before = project_ids(&index);
    assert_eq!(before.len(), 2);

    let tokens = codotheca_core::testing::FakeTokenStore::available();
    tokens
        .store(
            "github:forge.example.invalid:octo",
            &codotheca_core::accounts::keychain::SecretToken::new("sentinel".to_owned()),
        )
        .expect("stored");
    // R75: `accounts.disconnect` is answered off the index lock, so it takes the shared handle
    // the assembly hands it rather than an `AccountsCtx`.
    let index = Arc::new(std::sync::Mutex::new(index));
    codotheca_core::accounts::commands::handle_disconnect(
        &index,
        &tokens,
        serde_json::json!({ "accountId": fixture.account.0 }),
    )
    .expect("disconnect succeeds");
    let index = index
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);

    assert_eq!(
        count(
            &index,
            "SELECT count(*) FROM account WHERE id = ?1",
            fixture.account.0
        ),
        0
    );
    assert_eq!(
        count(
            &index,
            "SELECT count(*) FROM account_org WHERE account_id = ?1",
            fixture.account.0
        ),
        0,
        "account_org did not cascade"
    );
    assert_eq!(
        count(
            &index,
            "SELECT count(*) FROM project_account WHERE account_id = ?1",
            fixture.account.0
        ),
        0,
        "project_account did not cascade"
    );
    assert_eq!(
        project_ids(&index),
        before,
        "disconnect deleted a project row"
    );
    assert!(
        tokens.entry_names().is_empty(),
        "the keychain entry survived"
    );
    drop(fixture.dir);
    let _ = fixture.projects;
}

/// **Order: the keychain first.** With deletion failing, the command fails and the `account` row
/// **and both of its side tables** survive — deleting the row first would lose `token_ref`,
/// orphan a live secret, and tell the user a token was destroyed when it was not.
#[test]
fn a_failing_keychain_deletion_leaves_the_account_and_its_side_tables() {
    let fixture = disconnectable(1_000);
    let index = fixture.index;
    let tokens = codotheca_core::testing::FakeTokenStore::refusing_delete();
    let index = Arc::new(std::sync::Mutex::new(index));
    let outcome = codotheca_core::accounts::commands::handle_disconnect(
        &index,
        &tokens,
        serde_json::json!({ "accountId": fixture.account.0 }),
    );
    assert!(outcome.is_err(), "a refused keychain must fail the command");
    let index = index
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);

    assert_eq!(
        count(
            &index,
            "SELECT count(*) FROM account WHERE id = ?1",
            fixture.account.0
        ),
        1,
        "the row was deleted while its secret is still alive"
    );
    assert_eq!(
        count(
            &index,
            "SELECT count(*) FROM account_org WHERE account_id = ?1",
            fixture.account.0
        ),
        1
    );
    assert_eq!(
        count(
            &index,
            "SELECT count(*) FROM project_account WHERE account_id = ?1",
            fixture.account.0
        ),
        2
    );
    drop(fixture.dir);
}

/// R69's three assertions over the census, each printing its counts.
///
/// 1. **Completeness** — every table in the *migrated* schema with an `account_id` column is in
///    the census, and `sync_task_state`/`sync_budget` are absent from the schema or listed.
/// 2. **Coverage by mechanism** — `PRAGMA foreign_key_list` per census table: a cascading key
///    into `account` covers it; a table without one must be named in `delete_account`'s source.
/// 3. Behaviour is the two tests above.
#[test]
fn the_account_referencing_census_is_complete_and_covered() {
    let fixture = disconnectable(1_000);
    let index = fixture.index;

    // 1. Completeness.
    let mut stmt = index
        .conn()
        .prepare(
            "SELECT m.name FROM sqlite_master m
             JOIN pragma_table_info(m.name) c
             WHERE m.type = 'table' AND c.name = 'account_id'
             ORDER BY m.name",
        )
        .expect("prepared");
    let holders: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .expect("query")
        .map(Result::unwrap)
        .collect();
    assert!(
        !holders.is_empty(),
        "no table holds an account_id, so this proved nothing"
    );
    for table in &holders {
        assert!(
            ACCOUNT_REFERENCING_TABLES.contains(&table.as_str()),
            "{table} holds an account_id and is not in the census"
        );
    }
    for polymorphic in ["sync_task_state", "sync_budget"] {
        let present: i64 = index
            .conn()
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table' AND name=?1",
                [polymorphic],
                |row| row.get(0),
            )
            .expect("countable");
        assert!(
            present == 0 || ACCOUNT_REFERENCING_TABLES.contains(&polymorphic),
            "{polymorphic} exists in the schema and is not in the census"
        );
    }

    // 2. Coverage by mechanism.
    let source = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/accounts/store.rs"),
    )
    .expect("store source is readable");
    assert!(!source.is_empty());
    let mut by_cascade = 0_usize;
    let mut explicit = 0_usize;
    let mut handled = 0_usize;
    for table in ACCOUNT_REFERENCING_TABLES {
        let mut keys = index
            .conn()
            .prepare(&format!(
                "SELECT \"table\", on_delete FROM pragma_foreign_key_list('{table}')"
            ))
            .expect("prepared");
        let cascades = keys
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .expect("query")
            .map(Result::unwrap)
            .any(|(target, on_delete)| target == "account" && on_delete == "CASCADE");
        if cascades {
            by_cascade += 1;
        } else {
            explicit += 1;
            // A table with no cascading key must be named in `delete_account`'s source, which is
            // the weaker of the two forms this project has ruled on and is recorded as such.
            if source.contains(table) {
                handled += 1;
            }
        }
    }
    eprintln!(
        "acceptance_accounts: {} census tables, {by_cascade} by cascade, {explicit} explicit",
        ACCOUNT_REFERENCING_TABLES.len()
    );
    // [p2-21] `0011` adds `sync_budget` (cascading) and `sync_task_state` (no key at all), so
    // R69's figures move from `2, 2, 0` at version 8 to `4, 3, 1` here. **`0` handled by a call
    // would mean `sync_task_state` is being left behind**, which is the one case these three
    // numbers exist to make visible.
    assert_eq!(ACCOUNT_REFERENCING_TABLES.len(), 4, "the census at 0011");
    assert_eq!(by_cascade, 3);
    assert_eq!(explicit, 1, "sync_task_state, the one table with no key");
    assert_eq!(
        explicit, handled,
        "{explicit} explicit, {handled} handled: a census table with no cascading key is not \
         named in delete_account's source"
    );
    drop(fixture.dir);
}

/// **The precondition is a refusal, not a comment.** With `foreign_keys` off the cascade would
/// not fire, and `account_org` and `project_account` would silently retain rows while every
/// other assertion still passed. A warning is exactly what that silent case survives.
#[test]
fn delete_account_refuses_when_the_cascade_would_not_fire() {
    let fixture = disconnectable(1_000);
    let index = fixture.index;
    index
        .conn()
        .execute_batch("PRAGMA foreign_keys=OFF;")
        .expect("pragma");

    let tx = index.conn().unchecked_transaction().expect("tx");
    let outcome = codotheca_core::accounts::store::delete_account(&tx, fixture.account);
    assert!(
        outcome.is_err(),
        "delete_account proceeded with the cascade disabled"
    );
    drop(tx);

    assert_eq!(
        count(
            &index,
            "SELECT count(*) FROM account WHERE id = ?1",
            fixture.account.0
        ),
        1,
        "the account row was deleted anyway"
    );
    drop(fixture.dir);
}

/// The rows this plan added to `UNOWNED_COMMANDS` are gone: every `accounts.*` command now
/// reaches a module.
///
/// **Narrowed, not loosened** (R99). It asserted the whole constant was empty, which was true
/// when §20.8 wrote it and is wider than the claim its own name makes. §24.9's three `install.*`
/// rows arrive in the state the eight `accounts.*` rows once had, and an accounts test that goes
/// red on them is measuring another section's progress. The general property — every unowned row
/// names the task that owes it, and the shell offers none of them to the renderer — is asserted
/// by `assembly::route`'s own tests and by `app/test/knownCommands.test.ts`.
#[test]
fn no_accounts_command_is_unowned_any_more() {
    let rows = codotheca_core::assembly::route::UNOWNED_COMMANDS;
    let unowned: Vec<&str> = rows
        .iter()
        .map(|(c, _)| *c)
        .filter(|c| c.starts_with("accounts."))
        .collect();
    assert!(
        unowned.is_empty(),
        "an accounts command is still unowned: {unowned:?}, of {} unowned rows",
        rows.len()
    );
}
