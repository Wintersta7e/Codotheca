//! Four user decisions from phases 1–3 that a rebuild cannot re-derive (§48.8.3).
//!
//! They are a check's N/A ruling, a copy's trust, a project's consent to remote README images, and
//! a connected account with its organisation switches.
//!
//! Each section reads its rows here and restores through its owner's writer: `check_na` through
//! `completion::set_check_na`, `location_trust` through `surfaces::repair::set_trusted`,
//! `readme_consent` through `readme::consent::write_readme_remote`, and `accounts` through the
//! account store. The sidecar never holds a token — an account carries the keychain entry's name,
//! and the keychain keeps the secret.

use rusqlite::{Connection, OptionalExtension as _, Transaction};

use super::sidecar::{location_key, RestoreCtx, RestoreOutcome, SectionRow};
use super::subject::subject_for_project;
use super::IndexError;
use crate::accounts::store::{insert_account, set_org_enabled, upsert_orgs, NewAccount};
use crate::protocol::{AuthKind, CompletionCheck, ProjectId, ScopeTier};
use crate::provider::listing::OrgListing;

fn row_error(section: &str, error: impl std::fmt::Display) -> IndexError {
    IndexError::Sidecar(format!("a {section} row: {error}"))
}

/// The subject key a `Subject` row waits under. A project with none has no copy a scan could
/// find, so a row of its could never be matched and is not exported.
fn subject_key(conn: &Connection, project: i64) -> Result<Option<String>, IndexError> {
    Ok(subject_for_project(conn, ProjectId(project))?.map(|s| s.to_key()))
}

/// A stored enum's wire word, read back through the generated type's own serde names.
fn from_word<T: serde::de::DeserializeOwned>(section: &str, word: String) -> Result<T, IndexError> {
    serde_json::from_value(serde_json::Value::String(word)).map_err(|e| row_error(section, e))
}

fn to_data<T: serde::Serialize>(section: &str, data: &T) -> Result<serde_json::Value, IndexError> {
    serde_json::to_value(data).map_err(|e| row_error(section, e))
}

/// One `check_na` row: the user's ruling on one check (§31.4).
#[derive(serde::Serialize, serde::Deserialize)]
struct CheckRuling {
    check_key: CompletionCheck,
    user_na: bool,
}

/// The `check_na` section's export: one row per check the user has ruled on, on a project that
/// was not merged into another.
pub(crate) fn export_check_na(conn: &Connection) -> Result<Vec<SectionRow>, IndexError> {
    let ruled: Vec<(i64, String, i64)> = conn
        .prepare(
            "SELECT c.project_id, c.check_key, c.user_na FROM project_check c
             JOIN project p ON p.id = c.project_id
             WHERE c.user_na IS NOT NULL AND p.merged_into IS NULL
             ORDER BY c.project_id, c.check_key",
        )?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<Result<_, _>>()?;
    let mut out = Vec::new();
    for (project, key, user_na) in ruled {
        let Some(subject) = subject_key(conn, project)? else {
            continue;
        };
        let ruling = CheckRuling {
            check_key: from_word("check_na", key)?,
            user_na: user_na != 0,
        };
        out.push(SectionRow {
            subject: Some(subject),
            location_keys: Vec::new(),
            data: to_data("check_na", &ruling)?,
        });
    }
    Ok(out)
}

/// The `check_na` section's restore, through `completion::set_check_na`, which records the ruling
/// on the check's row and recomputes the project.
///
/// A ruling is a column of that row, and a rebuilt index has the row only once the evaluator has
/// written the project's ten — never at the hand-off that brings the project back. Until then
/// the restore answers `Pending` and writes nothing, so the record waits for the evaluator's
/// first write, which matches it, rather than being spent on a write that changes no row.
pub(crate) fn restore_check_na(
    tx: &Transaction<'_>,
    row: &SectionRow,
    ctx: &RestoreCtx<'_>,
) -> Result<RestoreOutcome, IndexError> {
    let ruling: CheckRuling =
        serde_json::from_value(row.data.clone()).map_err(|e| row_error("check_na", e))?;
    let project = ctx
        .project
        .ok_or_else(|| row_error("check_na", "restored without a project"))?;
    let has_row = crate::completion::store::load_rows(tx, project)?
        .iter()
        .any(|check| check.key == ruling.check_key);
    if !has_row {
        return Ok(RestoreOutcome::Pending);
    }
    crate::completion::set_check_na(tx, project, ruling.check_key, Some(ruling.user_na), ctx.now)?;
    Ok(RestoreOutcome::Applied(1))
}

/// One `location_trust` row: when the user trusted the copy its one location key names (§11.1).
#[derive(serde::Serialize, serde::Deserialize)]
struct Trust {
    trusted_at: i64,
}

/// The `location_trust` section's export: one row per trusted copy of a project that was not
/// merged into another.
pub(crate) fn export_location_trust(conn: &Connection) -> Result<Vec<SectionRow>, IndexError> {
    let trusted: Vec<(i64, i64, i64)> = conn
        .prepare(
            "SELECT l.id, l.project_id, l.trusted_at FROM location l
             JOIN project p ON p.id = l.project_id
             WHERE l.trusted_at IS NOT NULL AND p.merged_into IS NULL
             ORDER BY l.id",
        )?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<Result<_, _>>()?;
    let mut out = Vec::new();
    for (location, project, trusted_at) in trusted {
        let (Some(subject), Some(key)) = (
            subject_key(conn, project)?,
            location_key(conn, Some(location))?,
        ) else {
            continue;
        };
        out.push(SectionRow {
            subject: Some(subject),
            location_keys: vec![key],
            data: to_data("location_trust", &Trust { trusted_at })?,
        });
    }
    Ok(out)
}

/// The `location_trust` section's restore, write-once: a copy already trusted keeps its own time.
/// `set_trusted` stamps the time it is handed, so the exported time is the one passed.
///
/// A copy no scan has found yet resolves to no location; the restore then answers `Pending` and
/// the record waits for the hand-off that brings the copy back (§48.8.4).
pub(crate) fn restore_location_trust(
    tx: &Transaction<'_>,
    row: &SectionRow,
    ctx: &RestoreCtx<'_>,
) -> Result<RestoreOutcome, IndexError> {
    let trust: Trust =
        serde_json::from_value(row.data.clone()).map_err(|e| row_error("location_trust", e))?;
    let Some(location) = row
        .location_keys
        .first()
        .and_then(|key| ctx.location_for(key))
    else {
        return Ok(RestoreOutcome::Pending);
    };
    let present: Option<i64> = tx.query_row(
        "SELECT trusted_at FROM location WHERE id = ?1",
        [location.0],
        |r| r.get(0),
    )?;
    if present.is_some() {
        return Ok(RestoreOutcome::Applied(0));
    }
    let written = crate::surfaces::repair::set_trusted(tx, location, trust.trusted_at)?;
    Ok(RestoreOutcome::Applied(u64::from(written)))
}

/// One `readme_consent` row: when the user allowed remote README images for the project (§25.5).
#[derive(serde::Serialize, serde::Deserialize)]
struct ReadmeConsent {
    readme_remote_at: i64,
}

/// The `readme_consent` section's export: one row per project, not merged into another, that the
/// user allowed remote README images for.
pub(crate) fn export_readme_consent(conn: &Connection) -> Result<Vec<SectionRow>, IndexError> {
    let granted: Vec<(i64, i64)> = conn
        .prepare(
            "SELECT id, readme_remote_at FROM project
             WHERE readme_remote_at IS NOT NULL AND merged_into IS NULL ORDER BY id",
        )?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<Result<_, _>>()?;
    let mut out = Vec::new();
    for (project, readme_remote_at) in granted {
        let Some(subject) = subject_key(conn, project)? else {
            continue;
        };
        out.push(SectionRow {
            subject: Some(subject),
            location_keys: Vec::new(),
            data: to_data("readme_consent", &ReadmeConsent { readme_remote_at })?,
        });
    }
    Ok(out)
}

/// The `readme_consent` section's restore, write-once: a project that already holds a grant keeps
/// its own. The grant is written with the time it was given, through the column's one writer and
/// not the command, which stamps the present time and publishes the change.
pub(crate) fn restore_readme_consent(
    tx: &Transaction<'_>,
    row: &SectionRow,
    ctx: &RestoreCtx<'_>,
) -> Result<RestoreOutcome, IndexError> {
    let consent: ReadmeConsent =
        serde_json::from_value(row.data.clone()).map_err(|e| row_error("readme_consent", e))?;
    let project = ctx
        .project
        .ok_or_else(|| row_error("readme_consent", "restored without a project"))?;
    let present: Option<i64> = tx.query_row(
        "SELECT readme_remote_at FROM project WHERE id = ?1",
        [project.0],
        |r| r.get(0),
    )?;
    if present.is_some() {
        return Ok(RestoreOutcome::Applied(0));
    }
    let written = crate::readme::consent::write_readme_remote(
        tx,
        project,
        Some(consent.readme_remote_at),
        ctx.now,
    )?;
    Ok(RestoreOutcome::Applied(u64::from(written)))
}

/// One organisation under an account: the user's switch, and what the listing last said of it.
#[derive(serde::Serialize, serde::Deserialize)]
struct OrgRecord {
    login: String,
    enabled: bool,
    repo_count_seen: Option<u32>,
    observed_at: Option<i64>,
}

/// One `accounts` row: a connection as the user made it (§20). Observations a sync re-reads —
/// verification, errors, the listing validator, SSO state — are not carried.
#[derive(serde::Serialize, serde::Deserialize)]
struct AccountRecord {
    provider: String,
    host: String,
    login: String,
    display_name: Option<String>,
    auth_kind: AuthKind,
    scope_tier: ScopeTier,
    granted_scopes: Vec<String>,
    /// The keychain entry's name, `<provider>:<host>:<login>` — never the token.
    token_ref: String,
    connected_at: i64,
    orgs: Vec<OrgRecord>,
}

type AccountColumns = (
    i64,
    String,
    String,
    String,
    Option<String>,
    String,
    String,
    String,
    String,
    i64,
);

/// The `accounts` section's export: every connected account with its organisations.
pub(crate) fn export_accounts(conn: &Connection) -> Result<Vec<SectionRow>, IndexError> {
    let accounts: Vec<AccountColumns> = conn
        .prepare(
            "SELECT id, provider, host, login, display_name, auth_kind, scope_tier,
                    granted_scopes, token_ref, connected_at
             FROM account ORDER BY id",
        )?
        .query_map([], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
                r.get(6)?,
                r.get(7)?,
                r.get(8)?,
                r.get(9)?,
            ))
        })?
        .collect::<Result<_, _>>()?;
    let mut out = Vec::new();
    for (id, provider, host, login, display_name, auth_kind, scope_tier, scopes, token_ref, at) in
        accounts
    {
        let orgs: Vec<(String, i64, Option<i64>, Option<i64>)> = conn
            .prepare(
                "SELECT login, is_enabled, repo_count_seen, observed_at FROM account_org
                 WHERE account_id = ?1 ORDER BY login",
            )?
            .query_map([id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
            .collect::<Result<_, _>>()?;
        let orgs = orgs
            .into_iter()
            .map(|(org, enabled, count, observed_at)| {
                Ok(OrgRecord {
                    login: org,
                    enabled: enabled != 0,
                    repo_count_seen: count
                        .map(u32::try_from)
                        .transpose()
                        .map_err(|e| row_error("accounts", e))?,
                    observed_at,
                })
            })
            .collect::<Result<_, IndexError>>()?;
        let record = AccountRecord {
            provider,
            host,
            login,
            display_name,
            auth_kind: from_word("accounts", auth_kind)?,
            scope_tier: from_word("accounts", scope_tier)?,
            granted_scopes: serde_json::from_str(&scopes).map_err(|e| row_error("accounts", e))?,
            token_ref,
            connected_at: at,
            orgs,
        };
        out.push(SectionRow {
            subject: None,
            location_keys: Vec::new(),
            data: to_data("accounts", &record)?,
        });
    }
    Ok(out)
}

/// The `accounts` section's restore, write-once by `(provider, host, login)`: an account already
/// connected keeps its own row. Otherwise the account is inserted as it was connected, each
/// organisation upserted as it was last listed, and each switch the user turned on turned on.
pub(crate) fn restore_accounts(
    tx: &Transaction<'_>,
    row: &SectionRow,
    _ctx: &RestoreCtx<'_>,
) -> Result<RestoreOutcome, IndexError> {
    let record: AccountRecord =
        serde_json::from_value(row.data.clone()).map_err(|e| row_error("accounts", e))?;
    let present = tx
        .query_row(
            "SELECT id FROM account WHERE provider = ?1 AND host = ?2 AND login = ?3",
            [&record.provider, &record.host, &record.login],
            |r| r.get::<_, i64>(0),
        )
        .optional()?;
    if present.is_some() {
        return Ok(RestoreOutcome::Applied(0));
    }
    let account = insert_account(
        tx,
        &NewAccount {
            provider: record.provider,
            host: record.host,
            login: record.login,
            display_name: record.display_name,
            auth_kind: record.auth_kind,
            scope_tier: record.scope_tier,
            granted_scopes: record.granted_scopes,
            token_ref: record.token_ref,
        },
        record.connected_at,
    )
    .map_err(|e| row_error("accounts", e))?;
    for org in &record.orgs {
        let listing = OrgListing {
            login: org.login.clone(),
            repo_count_seen: org.repo_count_seen,
        };
        // Every production write stamps `observed_at`; were one missing, the connection's own
        // time is the older claim, and an older time never over-claims currency.
        let observed_at = org.observed_at.unwrap_or(record.connected_at);
        upsert_orgs(tx, account, &[listing], observed_at).map_err(|e| row_error("accounts", e))?;
        if org.enabled {
            set_org_enabled(tx, account, &org.login, true).map_err(|e| row_error("accounts", e))?;
        }
    }
    Ok(RestoreOutcome::Applied(1))
}
