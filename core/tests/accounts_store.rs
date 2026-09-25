#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
#![cfg(feature = "testkit")]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use codotheca_core::accounts::commands::{is_github_sso_required, set_org_enabled_off_lock};
use codotheca_core::accounts::keychain::{SecretToken, TokenStore};
use codotheca_core::accounts::store::{
    insert_account, list_accounts, list_orgs, record_observed_scopes, set_org_enabled, upsert_orgs,
    NewAccount,
};
use codotheca_core::accounts::{dispatch_accounts_command, AccountsCtx, ACCOUNT_COMMANDS};
use codotheca_core::http::{normalise_headers, HttpResponse, HttpTransport};
use codotheca_core::index::Index;
use codotheca_core::proto::txguard::TxGuard;
use codotheca_core::protocol::{AccountId, AuthKind, ErrorCode, ProjectId, ScopeTier, SsoState};
use codotheca_core::provider::admit::admit;
use codotheca_core::provider::listing::{OrgListing, RepoListing, GITHUB_CANONICAL_HOST};
use codotheca_core::provider::{GitHubProvider, ProviderError};
use codotheca_core::testing::{FakeTokenStore, FakeTransport};
use rusqlite::OptionalExtension as _;

const NOW: i64 = 1_800_000_000;
const SCHEMA: &str = include_str!("../../protocol/schema/protocol.json");

fn fresh_index() -> (tempfile::TempDir, Index) {
    let dir = tempfile::tempdir().expect("temp dir");
    let index = Index::open_at(dir.path(), NOW).expect("open index");
    (dir, index)
}

fn new_account(login: &str, tier: ScopeTier) -> NewAccount {
    NewAccount {
        provider: "github".to_owned(),
        host: GITHUB_CANONICAL_HOST.to_owned(),
        login: login.to_owned(),
        display_name: Some(format!("{login} Display")),
        auth_kind: AuthKind::Pat,
        scope_tier: tier,
        granted_scopes: Vec::new(),
        token_ref: format!("github:{GITHUB_CANONICAL_HOST}:{login}"),
    }
}

fn store_token(tokens: &FakeTokenStore, account: &NewAccount) {
    tokens
        .store(
            &account.token_ref,
            &SecretToken::new(format!("token-for-{}", account.login)),
        )
        .expect("store fake token");
}

fn insert_new_account(index: &mut Index, account: &NewAccount) -> AccountId {
    let _guard = TxGuard::enter();
    let tx = index.conn_mut().transaction().expect("transaction");
    let id = insert_account(&tx, account, NOW).expect("insert account");
    tx.commit().expect("commit account");
    id
}

fn upsert(index: &mut Index, account: AccountId, orgs: &[OrgListing], now: i64) {
    let _guard = TxGuard::enter();
    let tx = index.conn_mut().transaction().expect("transaction");
    upsert_orgs(&tx, account, orgs, now).expect("upsert orgs");
    tx.commit().expect("commit orgs");
}

fn set_enabled(index: &mut Index, account: AccountId, org: &str, enabled: bool) {
    let _guard = TxGuard::enter();
    let tx = index.conn_mut().transaction().expect("transaction");
    set_org_enabled(&tx, account, org, enabled).expect("set org enabled");
    tx.commit().expect("commit enabled");
}

fn record_scopes(index: &mut Index, account: AccountId, scopes: &[String], now: i64) {
    let _guard = TxGuard::enter();
    let tx = index.conn_mut().transaction().expect("transaction");
    record_observed_scopes(&tx, account, scopes, now).expect("record scopes");
    tx.commit().expect("commit scopes");
}

fn insert_project(index: &Index, name: &str) -> ProjectId {
    index
        .conn()
        .execute(
            "INSERT INTO project (name, seed_basename, created_at, updated_at)
             VALUES (?1, ?1, ?2, ?2)",
            rusqlite::params![name, NOW],
        )
        .expect("insert project");
    ProjectId(index.conn().last_insert_rowid())
}

fn project_ids(index: &Index) -> BTreeSet<i64> {
    let mut stmt = index
        .conn()
        .prepare("SELECT id FROM project ORDER BY id")
        .expect("project ids query");
    stmt.query_map([], |row| row.get::<_, i64>(0))
        .expect("project ids")
        .map(Result::unwrap)
        .collect()
}

fn enabled_orgs(index: &Index, account: AccountId) -> BTreeSet<String> {
    list_orgs(index.conn(), account)
        .expect("list orgs")
        .expect("private tier orgs")
        .into_iter()
        .filter(|org| org.enabled)
        .map(|org| org.login)
        .collect()
}

fn org_repo(org: &str) -> RepoListing {
    RepoListing {
        provider: "github",
        provider_repo_id: "repo-101".to_owned(),
        clone_url: format!("https://{GITHUB_CANONICAL_HOST}/{org}/repo-one.git"),
        owner: org.to_owned(),
        name: "repo-one".to_owned(),
        can_push: Some(true),
        is_fork: false,
        fork_parent_clone_url: None,
        is_archived: false,
        is_private: false,
        in_org: Some(org.to_owned()),
    }
}

fn provider_with_transport(host: &str) -> (Arc<FakeTransport>, GitHubProvider) {
    let transport = Arc::new(FakeTransport::new());
    let http_transport: Arc<dyn HttpTransport> = transport.clone();
    (
        transport,
        GitHubProvider::new(http_transport, host.to_owned()),
    )
}

fn response(status: u16, headers: Vec<(String, String)>, body: &[u8]) -> HttpResponse {
    HttpResponse {
        status,
        headers,
        body: body.to_vec(),
    }
}

const fn ctx(index: &Index) -> AccountsCtx<'_> {
    AccountsCtx { index }
}

fn dispatch(
    ctx: &mut AccountsCtx<'_>,
    command: &str,
    args: serde_json::Value,
) -> Result<serde_json::Value, codotheca_core::proto::dispatch::CommandFailure> {
    dispatch_accounts_command(ctx, command, args)
        .unwrap_or_else(|| panic!("{command} was not owned by the accounts dispatcher"))
}

fn sso_state(index: &Index, account: AccountId, org: &str) -> Option<SsoState> {
    let raw: Option<String> = index
        .conn()
        .query_row(
            "SELECT sso_state FROM account_org WHERE account_id = ?1 AND login = ?2",
            rusqlite::params![account.0, org],
            |row| row.get(0),
        )
        .optional()
        .expect("sso state query")
        .flatten();
    raw.map(|value| {
        serde_json::from_value(serde_json::Value::String(value)).expect("valid sso state")
    })
}

fn account_org_count(index: &Index, account: AccountId, org: &str) -> i64 {
    index
        .conn()
        .query_row(
            "SELECT count(*) FROM account_org WHERE account_id = ?1 AND login = ?2",
            rusqlite::params![account.0, org],
            |row| row.get(0),
        )
        .expect("account org count")
}

fn schema_account_command_names() -> Vec<String> {
    let doc: serde_json::Value = serde_json::from_str(SCHEMA).expect("protocol schema parses");
    let commands = doc["commands"].as_array().expect("commands array");
    let names = commands
        .iter()
        .filter_map(|command| command["name"].as_str())
        .filter(|name| name.starts_with("accounts."))
        .map(str::to_owned)
        .collect::<Vec<_>>();
    eprintln!(
        "accounts_store: protocol schema declares {} accounts command(s)",
        names.len()
    );
    assert!(
        !names.is_empty(),
        "the schema scan found no accounts commands, so it proved nothing"
    );
    names
}

#[test]
fn an_org_admits_nothing_until_enabled() {
    let (_dir, mut index) = fresh_index();
    let account = insert_new_account(&mut index, &new_account("member-one", ScopeTier::Private));
    let org = "org-for-gate";
    upsert(
        &mut index,
        account,
        &[OrgListing {
            login: org.to_owned(),
            repo_count_seen: Some(1),
        }],
        NOW + 1,
    );

    let rows = list_orgs(index.conn(), account)
        .expect("list orgs")
        .expect("private orgs");
    assert_eq!(rows.len(), 1);
    assert!(!rows[0].enabled, "org rows default to disabled");
    let listings = vec![org_repo(org)];
    let before = admit(
        &listings,
        "member-one",
        ScopeTier::Private,
        &enabled_orgs(&index, account),
    );
    assert!(
        before.admitted.is_empty(),
        "a newly enumerated org admits no repository until enabled"
    );

    insert_project(&index, "kept-one");
    insert_project(&index, "kept-two");
    let project_ids_before = project_ids(&index);
    set_enabled(&mut index, account, org, true);

    let after = admit(
        &listings,
        "member-one",
        ScopeTier::Private,
        &enabled_orgs(&index, account),
    );
    assert_eq!(after.admitted.len(), 1);

    set_enabled(&mut index, account, org, false);
    assert_eq!(
        project_ids(&index),
        project_ids_before,
        "disabling an org deletes no project row"
    );
}

#[test]
fn the_public_tier_reports_unknown_not_zero() {
    let (_dir, mut index) = fresh_index();
    let public = insert_new_account(&mut index, &new_account("public-member", ScopeTier::Public));
    let private = insert_new_account(
        &mut index,
        &new_account("private-member", ScopeTier::Private),
    );
    let mut ctx = ctx(&index);

    let public_result = dispatch(
        &mut ctx,
        "accounts.orgs",
        serde_json::json!({ "accountId": public }),
    )
    .expect("public orgs command succeeds");
    assert_eq!(
        public_result,
        serde_json::Value::Null,
        "public tier orgs are unknown on the wire"
    );

    let private_result = dispatch(
        &mut ctx,
        "accounts.orgs",
        serde_json::json!({ "accountId": private }),
    )
    .expect("private orgs command succeeds");
    assert_eq!(
        private_result,
        serde_json::json!([]),
        "private tier with no rows is known empty on the wire"
    );
}

#[test]
fn accounts_list_over_zero_accounts_is_an_empty_array() {
    let (_dir, index) = fresh_index();
    let mut ctx = ctx(&index);
    let result =
        dispatch(&mut ctx, "accounts.list", serde_json::json!({})).expect("accounts list succeeds");
    assert_eq!(result, serde_json::json!([]));
}

#[test]
fn upsert_orgs_preserves_enabled_and_known_repo_count() {
    let (_dir, mut index) = fresh_index();
    let account = insert_new_account(
        &mut index,
        &new_account("refresh-member", ScopeTier::Private),
    );
    upsert(
        &mut index,
        account,
        &[OrgListing {
            login: "refresh-org".to_owned(),
            repo_count_seen: Some(42),
        }],
        NOW + 1,
    );
    set_enabled(&mut index, account, "refresh-org", true);
    upsert(
        &mut index,
        account,
        &[OrgListing {
            login: "refresh-org".to_owned(),
            repo_count_seen: None,
        }],
        NOW + 2,
    );

    let rows = list_orgs(index.conn(), account)
        .expect("list orgs")
        .expect("private orgs");
    assert_eq!(rows.len(), 1);
    assert!(rows[0].enabled, "refresh preserves the user-enabled gate");
    assert_eq!(
        rows[0].repo_count_seen,
        Some(42),
        "an unknown count does not erase a known count"
    );
}

#[test]
fn sso_header_presence_decides_the_sso_required_failure() {
    let error = ProviderError::Http {
        status: 403,
        headers: normalise_headers([("X-GitHub-SSO", "required")]),
    };
    assert!(is_github_sso_required(&error));

    let plain_forbidden = ProviderError::Http {
        status: 403,
        headers: Vec::new(),
    };
    assert!(!is_github_sso_required(&plain_forbidden));

    let wrong_status = ProviderError::Http {
        status: 401,
        headers: normalise_headers([("X-GitHub-SSO", "required")]),
    };
    assert!(!is_github_sso_required(&wrong_status));
}

#[test]
fn sso_403_sets_unauthorized_keeps_rows_and_raises_sso_required() {
    let (_dir, mut index) = fresh_index();
    let tokens = FakeTokenStore::available();
    let account_row = new_account("sso-member", ScopeTier::Private);
    store_token(&tokens, &account_row);
    let account = insert_new_account(&mut index, &account_row);
    upsert(
        &mut index,
        account,
        &[OrgListing {
            login: "sso-org".to_owned(),
            repo_count_seen: None,
        }],
        NOW + 1,
    );
    insert_project(&index, "sso-kept");
    let project_ids_before = project_ids(&index);

    let (transport, provider) = provider_with_transport(GITHUB_CANONICAL_HOST);
    transport.push(response(
        403,
        normalise_headers([("X-GitHub-SSO", "required")]),
        b"{}",
    ));
    let index = Arc::new(Mutex::new(index));
    let failure = set_org_enabled_off_lock(
        &index,
        &provider,
        &tokens,
        serde_json::json!({
            "accountId": account,
            "orgLogin": "sso-org",
            "enabled": true
        }),
        NOW,
    )
    .expect_err("SSO requirement refuses enabling");

    assert_eq!(failure.code, ErrorCode::SsoRequired);
    let guard = index.lock().unwrap_or_else(PoisonError::into_inner);
    assert_eq!(
        sso_state(&guard, account, "sso-org"),
        Some(SsoState::Unauthorized)
    );
    assert_eq!(account_org_count(&guard, account, "sso-org"), 1);
    assert_eq!(project_ids(&guard), project_ids_before);
}

#[test]
fn forbidden_without_sso_header_does_not_mark_the_org_unauthorized() {
    let (_dir, mut index) = fresh_index();
    let tokens = FakeTokenStore::available();
    let account_row = new_account("forbidden-member", ScopeTier::Private);
    store_token(&tokens, &account_row);
    let account = insert_new_account(&mut index, &account_row);
    upsert(
        &mut index,
        account,
        &[OrgListing {
            login: "plain-forbidden-org".to_owned(),
            repo_count_seen: None,
        }],
        NOW + 1,
    );

    let (transport, provider) = provider_with_transport(GITHUB_CANONICAL_HOST);
    transport.push(response(403, Vec::new(), b"{}"));
    let index = Arc::new(Mutex::new(index));
    let failure = set_org_enabled_off_lock(
        &index,
        &provider,
        &tokens,
        serde_json::json!({
            "accountId": account,
            "orgLogin": "plain-forbidden-org",
            "enabled": true
        }),
        NOW,
    )
    .expect_err("plain 403 refuses enabling");

    assert_ne!(
        failure.code,
        ErrorCode::SsoRequired,
        "403 alone is not an SSO requirement"
    );
    assert_ne!(
        failure.code,
        ErrorCode::PermissionDenied,
        "PERMISSION_DENIED is a filesystem error and §20.8 forbids overloading it"
    );
    assert_eq!(
        failure.code,
        ErrorCode::TokenInvalid,
        "a forge refusal is an identity error"
    );
    let guard = index.lock().unwrap_or_else(PoisonError::into_inner);
    assert_eq!(sso_state(&guard, account, "plain-forbidden-org"), None);
}

#[test]
fn token_invalid_from_provider_raises_token_invalid() {
    let (_dir, mut index) = fresh_index();
    let tokens = FakeTokenStore::available();
    let account_row = new_account("expired-member", ScopeTier::Private);
    store_token(&tokens, &account_row);
    let account = insert_new_account(&mut index, &account_row);
    upsert(
        &mut index,
        account,
        &[OrgListing {
            login: "expired-org".to_owned(),
            repo_count_seen: Some(1),
        }],
        NOW + 1,
    );

    let (transport, provider) = provider_with_transport(GITHUB_CANONICAL_HOST);
    transport.push(response(401, Vec::new(), b"{}"));
    let index = Arc::new(Mutex::new(index));
    let failure = set_org_enabled_off_lock(
        &index,
        &provider,
        &tokens,
        serde_json::json!({
            "accountId": account,
            "orgLogin": "expired-org",
            "enabled": true
        }),
        NOW,
    )
    .expect_err("invalid token refuses enabling");

    assert_eq!(failure.code, ErrorCode::TokenInvalid);
}

#[test]
fn enabling_an_org_records_observed_scopes_from_the_provider_response() {
    let (_dir, mut index) = fresh_index();
    let tokens = FakeTokenStore::available();
    let account_row = new_account("scope-command-member", ScopeTier::Private);
    store_token(&tokens, &account_row);
    let account = insert_new_account(&mut index, &account_row);
    upsert(
        &mut index,
        account,
        &[OrgListing {
            login: "scope-command-org".to_owned(),
            repo_count_seen: Some(1),
        }],
        NOW + 1,
    );

    let (transport, provider) = provider_with_transport(GITHUB_CANONICAL_HOST);
    transport.push(response(
        200,
        normalise_headers([("X-OAuth-Scopes", "fresh:command, comma,scope")]),
        b"[]",
    ));
    let index = Arc::new(Mutex::new(index));
    let result = set_org_enabled_off_lock(
        &index,
        &provider,
        &tokens,
        serde_json::json!({
            "accountId": account,
            "orgLogin": "scope-command-org",
            "enabled": true
        }),
        NOW,
    )
    .expect("enable succeeds");

    assert!(result.enabled);
    let guard = index.lock().unwrap_or_else(PoisonError::into_inner);
    let rows = list_accounts(guard.conn()).expect("list accounts");
    assert_eq!(
        rows[0].granted_scopes,
        vec![
            "fresh:command".to_owned(),
            "comma".to_owned(),
            "scope".to_owned()
        ]
    );
    assert_eq!(rows[0].scopes_observed_at, Some(NOW));
}

#[test]
fn account_commands_match_the_schema_accounts_names() {
    let schema_names = schema_account_command_names();
    assert_eq!(schema_names.len(), 8);
    assert_eq!(
        ACCOUNT_COMMANDS,
        schema_names
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
            .as_slice()
    );
}

#[test]
fn record_observed_scopes_round_trips_json_array_values() {
    let (_dir, mut index) = fresh_index();
    let account = insert_new_account(&mut index, &new_account("scope-member", ScopeTier::Private));
    let unusual = ["invented", "server", "grant"].join(":");
    let comma = ["scope", "with", "comma"].join(",");
    let scopes = vec![unusual, comma];
    record_scopes(&mut index, account, &scopes, NOW + 3);

    let rows = list_accounts(index.conn()).expect("list accounts");
    assert_eq!(rows[0].granted_scopes, scopes);
    assert_eq!(rows[0].scopes_observed_at, Some(NOW + 3));

    let raw: String = index
        .conn()
        .query_row(
            "SELECT granted_scopes FROM account WHERE id = ?1",
            [account.0],
            |row| row.get(0),
        )
        .expect("raw scopes");
    let decoded: Vec<String> = serde_json::from_str(&raw).expect("json scope array");
    assert_eq!(decoded, rows[0].granted_scopes);
}

#[test]
fn account_sources_do_not_delete_project_rows() {
    fn walk(dir: &Path, out: &mut Vec<(PathBuf, String)>) {
        for entry in std::fs::read_dir(dir).expect("accounts dir is readable") {
            let entry = entry.expect("a readable accounts dir entry");
            let path = entry.path();
            let kind = entry.file_type().expect("accounts source kind");
            if kind.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                match std::fs::read_to_string(&path) {
                    Ok(source) => out.push((path, source)),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => panic!("{}: {error}", path.display()),
                }
            }
        }
    }

    let source_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/accounts");
    let mut sources = Vec::new();
    walk(&source_dir, &mut sources);
    eprintln!(
        "accounts_store: project delete scan read {} accounts source file(s)",
        sources.len()
    );
    assert!(
        !sources.is_empty(),
        "the accounts source walk read no files, so it proved nothing"
    );

    let offenders = sources
        .into_iter()
        .filter_map(|(path, source)| {
            let code = source
                .lines()
                .filter(|line| !line.trim_start().starts_with("//"))
                .collect::<Vec<_>>()
                .join("\n");
            code.contains("DELETE FROM project").then(|| {
                path.strip_prefix(&source_dir)
                    .unwrap_or(&path)
                    .display()
                    .to_string()
            })
        })
        .collect::<Vec<_>>();
    assert!(
        offenders.is_empty(),
        "accounts source deletes project rows: {offenders:?}"
    );
}

// ---------------------------------------------------------------------------
// Task 10 — the PAT fallback, Enterprise Server, and the scope upgrade.
// ---------------------------------------------------------------------------

fn pat_fixture() -> (tempfile::TempDir, Arc<Mutex<Index>>, Arc<FakeTransport>) {
    let dir = tempfile::tempdir().expect("tmp");
    let index = Arc::new(Mutex::new(
        Index::open_at(dir.path(), 1_000).expect("index opens"),
    ));
    (dir, index, Arc::new(FakeTransport::new()))
}

fn account_rows(index: &Arc<Mutex<Index>>) -> i64 {
    let guard = index.lock().unwrap_or_else(PoisonError::into_inner);
    guard
        .conn()
        .query_row("SELECT count(*) FROM account", [], |row| row.get(0))
        .expect("countable")
}

/// §20.2's normative order: **verify, then keychain, then row.** A token the forge refuses must
/// not create an `account` row, and must not leave a keychain entry either.
#[test]
fn a_pat_the_forge_refuses_writes_no_row_and_no_keychain_entry() {
    let (_dir, index, transport) = pat_fixture();
    transport.push(HttpResponse {
        status: 401,
        headers: Vec::new(),
        body: b"{}".to_vec(),
    });
    let tokens = FakeTokenStore::available();

    let failure = codotheca_core::accounts::commands::connect_pat(
        &index,
        &(Arc::clone(&transport) as Arc<dyn HttpTransport>),
        &tokens,
        "forge.example.invalid",
        &SecretToken::new("not-a-real-token".to_owned()),
        2_000,
    )
    .expect_err("a refused token must fail");

    assert_eq!(failure.code, ErrorCode::TokenInvalid);
    assert!(tokens.entry_names().is_empty(), "the keychain was written");
    assert_eq!(
        account_rows(&index),
        0,
        "an unauthenticated token made a row"
    );
}

/// The second half of the same rule: the forge said yes, the **keychain** said no, and the row
/// must still not exist. A row whose `token_ref` names an entry that was never created reads to
/// every later caller as a connected account with an unreadable token.
#[test]
fn a_pat_whose_keychain_store_fails_writes_no_row() {
    let (_dir, index, transport) = pat_fixture();
    transport.push(ok_json(
        &serde_json::json!({ "login": "octo", "name": "Octo Fixture" }),
        &[("X-OAuth-Scopes", "read:user")],
    ));
    let tokens = FakeTokenStore::refusing_store();

    let outcome = codotheca_core::accounts::commands::connect_pat(
        &index,
        &(Arc::clone(&transport) as Arc<dyn HttpTransport>),
        &tokens,
        "forge.example.invalid",
        &SecretToken::new("pat-sentinel".to_owned()),
        2_000,
    );
    assert!(outcome.is_err(), "a refused keychain must fail the connect");
    assert_eq!(account_rows(&index), 0, "the row was written anyway");
}

/// A successful PAT writes exactly one row, `auth_kind` is `pat`, and `granted_scopes` is the
/// server's set **verbatim** — including a scope no source file in this repository contains.
#[test]
fn a_successful_pat_writes_one_row_with_the_servers_own_scope_set() {
    let (_dir, index, transport) = pat_fixture();
    transport.push(ok_json(
        &serde_json::json!({ "login": "octo", "name": null }),
        &[("X-OAuth-Scopes", "read:user, an:invented:scope")],
    ));
    let tokens = FakeTokenStore::available();

    let account = codotheca_core::accounts::commands::connect_pat(
        &index,
        &(Arc::clone(&transport) as Arc<dyn HttpTransport>),
        &tokens,
        "forge.example.invalid",
        &SecretToken::new("pat-sentinel".to_owned()),
        2_000,
    )
    .expect("a verified token connects");

    assert_eq!(account_rows(&index), 1);
    assert_eq!(account.login, "octo");
    assert_eq!(account.auth_kind, AuthKind::Pat);
    assert_eq!(account.host, "forge.example.invalid");
    assert!(
        account
            .granted_scopes
            .contains(&"an:invented:scope".to_owned()),
        "the grant was not read back from the server: {:?}",
        account.granted_scopes
    );
    // A PAT without `repo` is the public tier, read back rather than asked for.
    assert_eq!(account.scope_tier, ScopeTier::Public);
    assert_eq!(
        tokens.entry_names(),
        ["github:forge.example.invalid:octo".to_owned()]
    );
    assert!(tokens.holds("github:forge.example.invalid:octo", "pat-sentinel"));
}

/// A PAT the server reports as carrying `repo` lands in the private tier — derived from what was
/// **granted**, never from what was asked for, because a pasted token may carry anything.
#[test]
fn the_tier_is_read_back_from_the_grant_not_assumed() {
    let (_dir, index, transport) = pat_fixture();
    transport.push(ok_json(
        &serde_json::json!({ "login": "octo", "name": null }),
        &[("X-OAuth-Scopes", "read:user, user:email, repo, read:org")],
    ));
    let tokens = FakeTokenStore::available();
    let account = codotheca_core::accounts::commands::connect_pat(
        &index,
        &(Arc::clone(&transport) as Arc<dyn HttpTransport>),
        &tokens,
        "",
        &SecretToken::new("pat-sentinel".to_owned()),
        2_000,
    )
    .expect("a verified token connects");
    assert_eq!(account.scope_tier, ScopeTier::Private);
    // An empty host is the canonical one, not an empty string on the row.
    assert_eq!(account.host, GITHUB_CANONICAL_HOST);
}

/// On Enterprise the API base is `https://<host>/api/v3`, and the request must actually go
/// there — the canonical provider would talk to the wrong server entirely.
#[test]
fn an_enterprise_host_is_reached_at_its_own_api_base() {
    let (_dir, index, transport) = pat_fixture();
    transport.push(ok_json(
        &serde_json::json!({ "login": "octo", "name": null }),
        &[("X-OAuth-Scopes", "read:user")],
    ));
    let _ = codotheca_core::accounts::commands::connect_pat(
        &index,
        &(Arc::clone(&transport) as Arc<dyn HttpTransport>),
        &FakeTokenStore::available(),
        "forge.example.invalid",
        &SecretToken::new("pat-sentinel".to_owned()),
        2_000,
    );
    let sent = transport.requests();
    assert_eq!(sent.len(), 1);
    assert!(
        sent[0]
            .url
            .starts_with("https://forge.example.invalid/api/v3"),
        "{}",
        sent[0].url
    );
}

fn ok_json(body: &serde_json::Value, headers: &[(&str, &str)]) -> HttpResponse {
    HttpResponse {
        status: 200,
        headers: normalise_headers(headers.iter().copied()),
        body: serde_json::to_vec(body).expect("json encodes"),
    }
}

/// A response with **no `X-OAuth-Scopes` header** states no grant, and that is unknown.
///
/// `scopes_observed_at` is the column that carries the difference: NULL is *never observed*, and
/// a timestamp beside an empty array is *observed, and the grant is empty*. Writing the second
/// for the first would render the account as having been checked and found to hold no scopes.
#[test]
fn a_grant_the_response_never_stated_is_unknown_not_an_empty_one() {
    let (_dir, index, transport) = pat_fixture();
    // No `X-OAuth-Scopes` at all.
    transport.push(ok_json(
        &serde_json::json!({ "login": "octo", "name": null }),
        &[],
    ));
    let unstated = codotheca_core::accounts::commands::connect_pat(
        &index,
        &(Arc::clone(&transport) as Arc<dyn HttpTransport>),
        &FakeTokenStore::available(),
        "forge.example.invalid",
        &SecretToken::new("pat-sentinel".to_owned()),
        2_000,
    )
    .expect("a verified token connects");

    assert_eq!(
        unstated.scopes_observed_at, None,
        "an absent header was recorded as an observation"
    );
    assert!(unstated.granted_scopes.is_empty());

    // The other half of the pair: a header that is present and empty **is** an observation.
    let (_dir2, index2, transport2) = pat_fixture();
    transport2.push(ok_json(
        &serde_json::json!({ "login": "octo", "name": null }),
        &[("X-OAuth-Scopes", "")],
    ));
    let observed = codotheca_core::accounts::commands::connect_pat(
        &index2,
        &(Arc::clone(&transport2) as Arc<dyn HttpTransport>),
        &FakeTokenStore::available(),
        "forge.example.invalid",
        &SecretToken::new("pat-sentinel".to_owned()),
        2_000,
    )
    .expect("a verified token connects");

    assert_eq!(
        observed.scopes_observed_at,
        Some(2_000),
        "a present but empty header is an observed empty grant"
    );
    assert!(observed.granted_scopes.is_empty());
}
