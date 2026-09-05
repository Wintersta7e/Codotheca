//! Account table reads and writes.
//!
//! `account.granted_scopes` is a single `TEXT` column, but it stores a JSON array rather than a
//! comma-joined string. The provider gives scopes as opaque strings, so a scope containing a comma
//! must round-trip without changing the set.

use rusqlite::{Connection, OptionalExtension as _, Transaction};
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::protocol::{Account, AccountId, AccountOrg, AuthKind, ScopeTier, SsoState};
use crate::provider::listing::OrgListing;

/// A new account row. The token itself belongs in the keychain; this struct carries only the
/// `token_ref` stored in SQLite.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewAccount {
    pub provider: String,
    pub host: String,
    pub login: String,
    pub display_name: Option<String>,
    pub auth_kind: AuthKind,
    pub scope_tier: ScopeTier,
    pub granted_scopes: Vec<String>,
    pub token_ref: String,
}

/// Failures from the account store.
#[derive(Debug, thiserror::Error)]
pub enum AccountError {
    #[error("account store query failed: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("account store JSON value failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("account store value could not be decoded: {0}")]
    Codec(String),
    #[error("account {account:?} or org {org:?} was not found")]
    NotFound {
        account: AccountId,
        org: Option<String>,
    },
}

#[derive(Debug)]
pub(crate) struct AccountRow {
    pub id: AccountId,
    pub provider: String,
    pub host: String,
    pub login: String,
    pub display_name: Option<String>,
    pub auth_kind: AuthKind,
    pub scope_tier: ScopeTier,
    pub granted_scopes: Vec<String>,
    pub scopes_observed_at: Option<i64>,
    pub token_ref: String,
    pub connected_at: i64,
    pub last_verified_at: Option<i64>,
    pub last_error_kind: Option<String>,
    pub last_error_at: Option<i64>,
    pub enabled: bool,
}

#[derive(Debug)]
struct RawAccountRow {
    id: i64,
    provider: String,
    host: String,
    login: String,
    display_name: Option<String>,
    auth_kind: String,
    scope_tier: String,
    granted_scopes: String,
    scopes_observed_at: Option<i64>,
    token_ref: String,
    connected_at: i64,
    last_verified_at: Option<i64>,
    last_error_kind: Option<String>,
    last_error_at: Option<i64>,
    is_enabled: i64,
}

#[derive(Debug)]
struct RawOrgRow {
    account_id: i64,
    login: String,
    is_enabled: i64,
    repo_count_seen: Option<i64>,
    sso_state: Option<String>,
    observed_at: Option<i64>,
}

/// Inserts one account row and returns its generated id.
///
/// # Errors
/// Fails when SQLite refuses the row or when the generated protocol enum/scopes values cannot be
/// serialised into their stored representation.
pub fn insert_account(
    tx: &Transaction<'_>,
    new: &NewAccount,
    now: i64,
) -> Result<AccountId, AccountError> {
    let auth_kind = enum_text(&new.auth_kind)?;
    let scope_tier = enum_text(&new.scope_tier)?;
    let granted_scopes = scopes_text(&new.granted_scopes)?;
    tx.execute(
        "INSERT INTO account
           (provider, host, login, display_name, auth_kind, scope_tier,
            granted_scopes, token_ref, connected_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        rusqlite::params![
            new.provider,
            new.host,
            new.login,
            new.display_name,
            auth_kind,
            scope_tier,
            granted_scopes,
            new.token_ref,
            now,
        ],
    )?;
    Ok(AccountId(tx.last_insert_rowid()))
}

/// Lists all accounts as wire DTOs. An empty account table is `Ok(vec![])`.
///
/// # Errors
/// Fails when SQLite cannot be read or a stored enum/scope value no longer matches the generated
/// protocol vocabulary.
pub fn list_accounts(conn: &Connection) -> Result<Vec<Account>, AccountError> {
    let rows = read_account_rows(conn)?;
    Ok(rows.into_iter().map(Account::from).collect())
}

/// Lists org rows for `id`.
///
/// Returns `None` under the public tier because the org list is unknown without `read:org`.
/// Returns `Some(rows)` under the private tier, where `Some(vec![])` means the enumeration
/// succeeded and found no rows.
///
/// # Errors
/// Fails when the account does not exist, SQLite cannot be read, or a stored enum/count value
/// cannot be decoded.
pub fn list_orgs(
    conn: &Connection,
    id: AccountId,
) -> Result<Option<Vec<AccountOrg>>, AccountError> {
    let scope_tier = account_scope_tier(conn, id)?;
    if scope_tier == ScopeTier::Public {
        return Ok(None);
    }
    org_rows(conn, id).map(Some)
}

/// Sets one org gate and returns the updated row.
///
/// # Errors
/// Fails when the account/org row does not exist, SQLite refuses the write, or the updated row
/// cannot be decoded.
pub fn set_org_enabled(
    tx: &Transaction<'_>,
    id: AccountId,
    org: &str,
    enabled: bool,
) -> Result<AccountOrg, AccountError> {
    let changed = tx.execute(
        "UPDATE account_org SET is_enabled = ?3 WHERE account_id = ?1 AND login = ?2",
        rusqlite::params![id.0, org, bit(enabled)],
    )?;
    if changed != 1 {
        return Err(not_found_for_org(tx, id, org)?);
    }
    org_row(tx, id, org)
}

/// Upserts org memberships from a provider listing.
///
/// Existing `is_enabled` is deliberately not named in either conflict arm, so a refresh cannot
/// silently disable an org the user had enabled. A new unknown `repo_count_seen` also cannot
/// overwrite a known count with `NULL`.
///
/// # Errors
/// Fails when SQLite refuses a row.
pub fn upsert_orgs(
    tx: &Transaction<'_>,
    id: AccountId,
    orgs: &[OrgListing],
    now: i64,
) -> Result<(), AccountError> {
    for org in orgs {
        if let Some(count) = org.repo_count_seen {
            tx.execute(
                "INSERT INTO account_org
                   (account_id, login, repo_count_seen, observed_at)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(account_id, login) DO UPDATE SET
                   repo_count_seen = excluded.repo_count_seen,
                   observed_at = excluded.observed_at",
                rusqlite::params![id.0, org.login, i64::from(count), now],
            )?;
        } else {
            tx.execute(
                "INSERT INTO account_org
                   (account_id, login, observed_at)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(account_id, login) DO UPDATE SET
                   observed_at = excluded.observed_at",
                rusqlite::params![id.0, org.login, now],
            )?;
        }
    }
    Ok(())
}

/// Records scopes from a provider response that actually carried the scope header.
///
/// The caller unwraps `Observed.granted_scopes`; a missing header is unknown and must not call
/// this function with an empty slice by default.
///
/// # Errors
/// Fails when the account does not exist, SQLite refuses the write, or the scope set cannot be
/// serialised as JSON.
pub fn record_observed_scopes(
    tx: &Transaction<'_>,
    id: AccountId,
    scopes: &[String],
    now: i64,
) -> Result<(), AccountError> {
    let changed = tx.execute(
        "UPDATE account
         SET granted_scopes = ?2, scopes_observed_at = ?3
         WHERE id = ?1",
        rusqlite::params![id.0, scopes_text(scopes)?, now],
    )?;
    if changed == 1 {
        Ok(())
    } else {
        Err(AccountError::NotFound {
            account: id,
            org: None,
        })
    }
}

/// Records the latest SSO state observed for an org row.
///
/// # Errors
/// Fails when the account/org row does not exist, SQLite refuses the write, or the generated enum
/// value cannot be serialised into the database vocabulary.
pub fn set_org_sso_state(
    tx: &Transaction<'_>,
    id: AccountId,
    org: &str,
    state: SsoState,
    now: i64,
) -> Result<(), AccountError> {
    let changed = tx.execute(
        "UPDATE account_org
         SET sso_state = ?3, observed_at = ?4
         WHERE account_id = ?1 AND login = ?2",
        rusqlite::params![id.0, org, enum_text(&state)?, now],
    )?;
    if changed != 1 {
        return Err(not_found_for_org(tx, id, org)?);
    }
    Ok(())
}

pub(crate) fn load_account(conn: &Connection, id: AccountId) -> Result<AccountRow, AccountError> {
    let raw = conn
        .query_row(
            "SELECT id, provider, host, login, display_name, auth_kind, scope_tier,
                    granted_scopes, scopes_observed_at, token_ref, connected_at,
                    last_verified_at, last_error_kind, last_error_at, is_enabled
             FROM account WHERE id = ?1",
            [id.0],
            raw_account,
        )
        .optional()?
        .ok_or(AccountError::NotFound {
            account: id,
            org: None,
        })?;
    AccountRow::try_from(raw)
}

fn read_account_rows(conn: &Connection) -> Result<Vec<AccountRow>, AccountError> {
    let mut stmt = conn.prepare(
        "SELECT id, provider, host, login, display_name, auth_kind, scope_tier,
                granted_scopes, scopes_observed_at, token_ref, connected_at,
                last_verified_at, last_error_kind, last_error_at, is_enabled
         FROM account
         ORDER BY id",
    )?;
    let rows = stmt.query_map([], raw_account)?;
    let mut out = Vec::new();
    for row in rows {
        out.push(AccountRow::try_from(row?)?);
    }
    Ok(out)
}

fn account_scope_tier(conn: &Connection, id: AccountId) -> Result<ScopeTier, AccountError> {
    let raw = conn
        .query_row(
            "SELECT scope_tier FROM account WHERE id = ?1",
            [id.0],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .ok_or(AccountError::NotFound {
            account: id,
            org: None,
        })?;
    enum_from_text(&raw)
}

fn org_rows(conn: &Connection, id: AccountId) -> Result<Vec<AccountOrg>, AccountError> {
    let mut stmt = conn.prepare(
        "SELECT account_id, login, is_enabled, repo_count_seen, sso_state, observed_at
         FROM account_org
         WHERE account_id = ?1
         ORDER BY login",
    )?;
    let rows = stmt.query_map([id.0], raw_org)?;
    let mut out = Vec::new();
    for row in rows {
        out.push(AccountOrg::try_from(row?)?);
    }
    Ok(out)
}

fn org_row(conn: &Connection, id: AccountId, org: &str) -> Result<AccountOrg, AccountError> {
    let raw = conn
        .query_row(
            "SELECT account_id, login, is_enabled, repo_count_seen, sso_state, observed_at
             FROM account_org
             WHERE account_id = ?1 AND login = ?2",
            rusqlite::params![id.0, org],
            raw_org,
        )
        .optional()?
        .ok_or(AccountError::NotFound {
            account: id,
            org: Some(org.to_owned()),
        })?;
    AccountOrg::try_from(raw)
}

fn not_found_for_org(
    conn: &Connection,
    id: AccountId,
    org: &str,
) -> Result<AccountError, AccountError> {
    let account_exists = conn
        .query_row("SELECT 1 FROM account WHERE id = ?1", [id.0], |_| Ok(()))
        .optional()?
        .is_some();
    let missing_org = account_exists.then(|| org.to_owned());
    Ok(AccountError::NotFound {
        account: id,
        org: missing_org,
    })
}

fn raw_account(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawAccountRow> {
    Ok(RawAccountRow {
        id: row.get(0)?,
        provider: row.get(1)?,
        host: row.get(2)?,
        login: row.get(3)?,
        display_name: row.get(4)?,
        auth_kind: row.get(5)?,
        scope_tier: row.get(6)?,
        granted_scopes: row.get(7)?,
        scopes_observed_at: row.get(8)?,
        token_ref: row.get(9)?,
        connected_at: row.get(10)?,
        last_verified_at: row.get(11)?,
        last_error_kind: row.get(12)?,
        last_error_at: row.get(13)?,
        is_enabled: row.get(14)?,
    })
}

fn raw_org(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawOrgRow> {
    Ok(RawOrgRow {
        account_id: row.get(0)?,
        login: row.get(1)?,
        is_enabled: row.get(2)?,
        repo_count_seen: row.get(3)?,
        sso_state: row.get(4)?,
        observed_at: row.get(5)?,
    })
}

impl TryFrom<RawAccountRow> for AccountRow {
    type Error = AccountError;

    fn try_from(row: RawAccountRow) -> Result<Self, Self::Error> {
        Ok(Self {
            id: AccountId(row.id),
            provider: row.provider,
            host: row.host,
            login: row.login,
            display_name: row.display_name,
            auth_kind: enum_from_text(&row.auth_kind)?,
            scope_tier: enum_from_text(&row.scope_tier)?,
            granted_scopes: serde_json::from_str(&row.granted_scopes)?,
            scopes_observed_at: row.scopes_observed_at,
            token_ref: row.token_ref,
            connected_at: row.connected_at,
            last_verified_at: row.last_verified_at,
            last_error_kind: row.last_error_kind,
            last_error_at: row.last_error_at,
            enabled: row.is_enabled != 0,
        })
    }
}

impl From<AccountRow> for Account {
    fn from(row: AccountRow) -> Self {
        Self {
            id: row.id,
            provider: row.provider,
            host: row.host,
            login: row.login,
            display_name: row.display_name,
            auth_kind: row.auth_kind,
            scope_tier: row.scope_tier,
            granted_scopes: row.granted_scopes,
            scopes_observed_at: row.scopes_observed_at,
            connected_at: row.connected_at,
            last_verified_at: row.last_verified_at,
            last_error_kind: row.last_error_kind,
            last_error_at: row.last_error_at,
            enabled: row.enabled,
        }
    }
}

impl TryFrom<RawOrgRow> for AccountOrg {
    type Error = AccountError;

    fn try_from(row: RawOrgRow) -> Result<Self, Self::Error> {
        Ok(Self {
            account_id: AccountId(row.account_id),
            login: row.login,
            enabled: row.is_enabled != 0,
            repo_count_seen: repo_count_seen(row.repo_count_seen)?,
            sso_state: row.sso_state.as_deref().map(enum_from_text).transpose()?,
            observed_at: row.observed_at,
        })
    }
}

fn enum_text<T: Serialize>(value: &T) -> Result<String, AccountError> {
    match serde_json::to_value(value)? {
        serde_json::Value::String(raw) => Ok(raw),
        other => Err(AccountError::Codec(format!(
            "generated enum serialised as non-string JSON: {other}"
        ))),
    }
}

fn enum_from_text<T: DeserializeOwned>(raw: &str) -> Result<T, AccountError> {
    Ok(serde_json::from_value(serde_json::Value::String(
        raw.to_owned(),
    ))?)
}

fn scopes_text(scopes: &[String]) -> Result<String, AccountError> {
    Ok(serde_json::to_string(scopes)?)
}

fn repo_count_seen(value: Option<i64>) -> Result<Option<u32>, AccountError> {
    value
        .map(|count| {
            u32::try_from(count)
                .map_err(|_| AccountError::Codec(format!("repo_count_seen {count} is outside u32")))
        })
        .transpose()
}

fn bit(value: bool) -> i64 {
    i64::from(value)
}

/// The fields a caller needs to act on an existing account without re-reading the whole row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountIdentity {
    pub provider: String,
    pub host: String,
    pub login: String,
    pub token_ref: String,
    pub scope_tier: ScopeTier,
}

/// One account's identity, or [`AccountError`]'s not-found variant.
///
/// # Errors
/// Fails when the row does not exist or the read does.
pub fn account_identity(conn: &Connection, id: AccountId) -> Result<AccountIdentity, AccountError> {
    conn.query_row(
        "SELECT provider, host, login, token_ref, scope_tier FROM account WHERE id = ?1",
        [id.0],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        },
    )
    .optional()?
    .map_or(
        Err(AccountError::NotFound {
            account: id,
            org: None,
        }),
        |(provider, host, login, token_ref, tier)| {
            Ok(AccountIdentity {
                provider,
                host,
                login,
                token_ref,
                scope_tier: enum_from_text(&tier)?,
            })
        },
    )
}

/// §20's upgrade: the tier, the grant and its observation time are rewritten **together**, from
/// the server, and the `token_ref` is deliberately untouched — the new token replaces the old in
/// the **same** keychain entry, so nothing else has to be told the name changed.
///
/// # Errors
/// Fails when the row does not exist or the write does.
pub fn record_upgraded_scope(
    tx: &Transaction<'_>,
    id: AccountId,
    tier: ScopeTier,
    scopes: &[String],
    now: i64,
) -> Result<(), AccountError> {
    let changed = tx.execute(
        "UPDATE account
            SET scope_tier = ?2, granted_scopes = ?3, scopes_observed_at = ?4,
                last_verified_at = ?4
          WHERE id = ?1",
        rusqlite::params![id.0, enum_text(&tier)?, scopes_text(scopes)?, now],
    )?;
    if changed == 0 {
        return Err(AccountError::NotFound {
            account: id,
            org: None,
        });
    }
    Ok(())
}

/// **R69's census: which tables hold an account reference.** `&[&str]`, ruled four times and
/// frozen on the fourth.
///
/// It says *which tables hold a reference* and **nothing about how each is cleared**, so naming
/// a table is a true claim about the schema. That is the whole point: an earlier form paired each
/// table with a column, and `("sync_task_state", "key")` would have *satisfied* the enumeration
/// while deleting every sync task whose **project** id happened to equal the disconnected
/// account id — `key` there is polymorphic. In this form that entry is not representable.
///
/// **Read by the test and by nothing else. It is not a delete list.** p2-21 appends
/// `"sync_budget"` and `"sync_task_state"` when `0011` creates them, and `delete_account_tasks`
/// — filtered `WHERE task = 'account_repos' AND key = ?1` — is the sole explicit call.
pub const ACCOUNT_REFERENCING_TABLES: &[&str] = &["account_org", "project_account"];

/// Deletes the `account` row and lets SQLite cascade. **It deletes no `project` row, ever.**
///
/// Every account-referencing table in phase 2 carries `ON DELETE CASCADE` into `account(id)`
/// **except one**: `sync_task_state.key` is a bare polymorphic integer and can carry no foreign
/// key at all. So there is no per-table `DELETE` loop here — the one table that needs an explicit
/// call is exactly the one with no key, which is the whole statement of the problem rather than a
/// special case inside it.
///
/// **The precondition is a refusal, not a comment.** All of the above rests on
/// `PRAGMA foreign_keys=ON`, which `Index::open_connection` sets — and which R59's rebuild
/// machinery turns **off** inside the migration runner and restores. If a later change ever left
/// it off around this call, `account_org`, `project_account` and `sync_budget` would all silently
/// retain rows **while every other assertion still passed**. A warning is precisely what that
/// silent case survives, so this reads the pragma back and refuses.
///
/// If this ever moves to a form that iterates and dispatches per table, **delete the refusal in
/// the same change**: `delete_account` would then clear each table itself, the pragma would stop
/// being load-bearing, and the guard would become unreachable rather than merely redundant —
/// still passing, still read as protection, guarding nothing.
///
/// # Errors
/// Refuses when `PRAGMA foreign_keys` is off, and fails when the row does not exist or the
/// delete does.
pub fn delete_account(tx: &Transaction<'_>, id: AccountId) -> Result<(), AccountError> {
    let enforced: i64 = tx.query_row("PRAGMA foreign_keys", [], |row| row.get(0))?;
    if enforced != 1 {
        return Err(AccountError::Codec(
            "refusing to delete an account with foreign_keys off: the cascade would not fire and \
             account_org, project_account and sync_budget would silently retain rows"
                .to_owned(),
        ));
    }
    let removed = tx.execute("DELETE FROM account WHERE id = ?1", [id.0])?;
    if removed == 0 {
        return Err(AccountError::NotFound {
            account: id,
            org: None,
        });
    }
    Ok(())
}
