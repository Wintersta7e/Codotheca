//! Handlers for the account command subset owned by this task.

use std::sync::{Arc, Mutex};

use serde_json::Value;

use crate::accounts::keychain::{KeychainError, SecretToken, TokenStore};
use crate::accounts::store::{self, AccountError};
use crate::accounts::AccountsCtx;
use crate::proto::dispatch::{parse_args, CommandFailure};
use crate::proto::txguard::TxGuard;
use crate::protocol::{
    Account, AccountId, AccountOrg, AccountsListArgs, AccountsOrgsArgs, AccountsSetOrgEnabledArgs,
    AuthKind, ErrorCode, ScopeTier, SsoState,
};
use crate::provider::{Provider as _, ProviderError};

/// Handles `accounts.list`.
///
/// # Errors
/// Fails when arguments are malformed or the account store cannot be read.
pub fn handle_list(ctx: &AccountsCtx<'_>, args: Value) -> Result<Vec<Account>, CommandFailure> {
    let _args: AccountsListArgs = parse_args(args)?;
    store::list_accounts(ctx.index.conn()).map_err(|error| account_failure(&error))
}

/// Handles `accounts.orgs`.
///
/// # Errors
/// Fails when arguments are malformed, the account id does not exist, or the account store cannot
/// be read.
pub fn handle_orgs(
    ctx: &AccountsCtx<'_>,
    args: Value,
) -> Result<Option<Vec<AccountOrg>>, CommandFailure> {
    let args: AccountsOrgsArgs = parse_args(args)?;
    store::list_orgs(ctx.index.conn(), args.account_id).map_err(|error| account_failure(&error))
}

/// Handles `accounts.setOrgEnabled`.
///
/// # Errors
/// Fails when arguments are malformed, the account/org row does not exist, the token cannot be
/// read, the provider refuses the preflight, or the account store cannot be written.
pub fn handle_set_org_enabled(
    ctx: &AccountsCtx<'_>,
    args: Value,
) -> Result<AccountOrg, CommandFailure> {
    let args: AccountsSetOrgEnabledArgs = parse_args(args)?;
    let observed_scopes = if args.enabled {
        preflight_enable(ctx, args.account_id, &args.org_login)?
    } else {
        None
    };

    let _guard = TxGuard::enter();
    let tx = ctx.index.conn().unchecked_transaction().map_err(internal)?;
    if let Some(scopes) = observed_scopes.as_deref() {
        store::record_observed_scopes(&tx, args.account_id, scopes, ctx.now)
            .map_err(|error| account_failure(&error))?;
    }
    let org = store::set_org_enabled(&tx, args.account_id, &args.org_login, args.enabled)
        .map_err(|error| account_failure(&error))?;
    tx.commit().map_err(internal)?;
    Ok(org)
}

/// GitHub marks SSO refusal by sending `x-github-sso` on a 403.
///
/// Header presence decides; a plain 403 is not an SSO requirement, and any other status with the
/// header is not this condition.
#[must_use]
pub fn is_github_sso_required(error: &ProviderError) -> bool {
    match error {
        ProviderError::Http {
            status: 403,
            headers,
        } => headers
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case("x-github-sso")),
        ProviderError::Http { .. } | ProviderError::Transport(_) | ProviderError::Decode(_) => {
            false
        }
    }
}

fn preflight_enable(
    ctx: &AccountsCtx<'_>,
    account_id: crate::protocol::AccountId,
    org_login: &str,
) -> Result<Option<Vec<String>>, CommandFailure> {
    let row = store::load_account(ctx.index.conn(), account_id)
        .map_err(|error| account_failure(&error))?;
    let token = ctx
        .tokens
        .read(&row.token_ref)
        .map_err(|error| keychain_failure(&error))?;
    match ctx.provider.list_repos(&token, None) {
        Ok(observed) => Ok(observed.granted_scopes),
        Err(error) if is_github_sso_required(&error) => {
            mark_sso_required(ctx, account_id, org_login)?;
            Err(coded_failure(ErrorCode::SsoRequired, error.to_string()))
        }
        Err(error) => Err(provider_failure(&error)),
    }
}

fn mark_sso_required(
    ctx: &AccountsCtx<'_>,
    account_id: crate::protocol::AccountId,
    org_login: &str,
) -> Result<(), CommandFailure> {
    let _guard = TxGuard::enter();
    let tx = ctx.index.conn().unchecked_transaction().map_err(internal)?;
    store::set_org_sso_state(&tx, account_id, org_login, SsoState::Unauthorized, ctx.now)
        .map_err(|error| account_failure(&error))?;
    tx.commit().map_err(internal)?;
    Ok(())
}

fn provider_failure(error: &ProviderError) -> CommandFailure {
    match error {
        ProviderError::Http { status: 401, .. } => {
            coded_failure(ErrorCode::TokenInvalid, error.to_string())
        }
        ProviderError::Http { status: 403, .. } => {
            coded_failure(ErrorCode::PermissionDenied, error.to_string())
        }
        ProviderError::Http { .. } | ProviderError::Transport(_) | ProviderError::Decode(_) => {
            internal(error)
        }
    }
}

fn keychain_failure(error: &KeychainError) -> CommandFailure {
    match error {
        KeychainError::NotFound => coded_failure(ErrorCode::TokenInvalid, error.to_string()),
        KeychainError::Unavailable | KeychainError::Backend(_) => {
            coded_failure(ErrorCode::StoreOffline, error.to_string())
        }
    }
}

fn account_failure(error: &AccountError) -> CommandFailure {
    let message = error.to_string();
    if matches!(error, AccountError::NotFound { .. }) {
        CommandFailure::protocol(message)
    } else {
        CommandFailure::internal(message)
    }
}

fn coded_failure(code: ErrorCode, message: String) -> CommandFailure {
    CommandFailure {
        code,
        message,
        outcome: None,
    }
}

fn internal(error: impl std::fmt::Display) -> CommandFailure {
    CommandFailure::internal(error.to_string())
}

/// §20.2's PAT fallback — **the advanced path, and the only path against Enterprise Server**,
/// where the application is not registered so no client id exists to run a Device Flow with.
///
/// **Order is normative: verify, then keychain, then row.** A token that does not authenticate
/// must not create an `account` row, and if the keychain store fails **no row is written** — a
/// row whose `token_ref` names an entry that does not exist reads to every later caller as a
/// connected account with an unreadable token.
///
/// **The pasted token crosses the wire exactly once, inbound.** It is never returned by any
/// command, never written to the rolling log, and never quoted in an error message: every
/// failure below is built from the provider's status, never from its body.
///
/// It takes `Arc<Mutex<Index>>` rather than an `&Index` because it is answered off the index
/// lock (R75): the verification is a forge round trip, and the guard is taken only around the
/// write that follows it.
///
/// # Errors
/// `TOKEN_INVALID` when the forge refuses the token, and the keychain's or the store's own
/// failure otherwise.
pub fn connect_pat(
    index: &Arc<Mutex<crate::index::Index>>,
    http: &Arc<dyn crate::http::HttpTransport>,
    tokens: &dyn TokenStore,
    host: &str,
    token: &SecretToken,
    now: i64,
) -> Result<Account, CommandFailure> {
    let host = if host.is_empty() {
        crate::provider::listing::GITHUB_CANONICAL_HOST.to_owned()
    } else {
        host.to_owned()
    };
    // A provider for THIS host: on Enterprise the API base is `https://<host>/api/v3`, and the
    // canonical provider the handler holds would talk to the wrong server.
    let provider = crate::provider::GitHubProvider::new(Arc::clone(http), host.clone());

    // 1. Verify. No lock is held, and nothing is written yet.
    let verified = provider.verify_token(token).map_err(|error| {
        if is_github_sso_required(&error) {
            coded_failure(ErrorCode::SsoRequired, error.to_string())
        } else {
            provider_failure(&error)
        }
    })?;
    let login = verified.value.login.clone();
    let display_name = verified.value.display_name.clone();
    let entry = super::keychain::token_ref(GITHUB_PROVIDER_ID, &host, &login);

    // 2. The keychain, before the row.
    tokens
        .store(&entry, token)
        .map_err(|error| keychain_failure(&error))?;

    // 3. The row. `granted_scopes` is the server's set, verbatim — never a source literal.
    let new = super::store::NewAccount {
        provider: GITHUB_PROVIDER_ID.to_owned(),
        host,
        login,
        display_name,
        auth_kind: AuthKind::Pat,
        scope_tier: tier_for(&verified.value.granted_scopes),
        granted_scopes: verified.value.granted_scopes.clone(),
        token_ref: entry,
    };
    let mut guard = index
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let _tx_guard = crate::proto::txguard::TxGuard::enter();
    let tx = guard
        .conn_mut()
        .transaction()
        .map_err(|e| CommandFailure::internal(e.to_string()))?;
    let id = super::store::insert_account(&tx, &new, now)
        .map_err(|e| CommandFailure::internal(e.to_string()))?;
    tx.commit()
        .map_err(|e| CommandFailure::internal(e.to_string()))?;

    super::store::list_accounts(guard.conn())
        .map_err(|e| CommandFailure::internal(e.to_string()))?
        .into_iter()
        .find(|a| a.id == id)
        .ok_or_else(|| CommandFailure::internal("the account just written was not readable"))
}

/// The tier a **read-back** grant places the account in.
///
/// Derived from what the server actually granted, never from what was asked for: a PAT is pasted
/// by the user and may carry anything. `repo` is the scope that separates the tiers, and it is
/// read **and** write — the forge offers no read-only variant, which is the fact §20.3's two-tier
/// design exists to surface rather than hide.
fn tier_for(granted: &[String]) -> ScopeTier {
    if granted.iter().any(|s| s == PRIVATE_TIER_MARKER) {
        ScopeTier::Private
    } else {
        ScopeTier::Public
    }
}

/// The adapter id stored in `account.provider`.
const GITHUB_PROVIDER_ID: &str = "github";

/// The one scope whose presence separates the two tiers. It is `SCOPES_PRIVATE`'s member rather
/// than a second literal, so the scope audit still sees exactly one home for scope strings.
const PRIVATE_TIER_MARKER: &str = crate::provider::scopes::PRIVATE_TIER_SCOPE;

/// §20.9's disconnect.
///
/// **Order matters and is normative: the keychain entry is deleted first.** If that fails, the
/// disconnect **fails with the reason and the row stays**. Deleting the row first would lose
/// `token_ref`, orphan a live secret, and tell the user a token was destroyed when it was not.
///
/// **It deletes no `project` row, ever.** Every project that had a local copy is unchanged, and
/// every zero-location project survives as *unseen since T* with its remote fields stale-marked —
/// absence is not evidence.
///
/// This is not the filesystem boundary: it removes credential state and database rows, and it is
/// **not** the one warranted removal primitive.
///
/// # Errors
/// The keychain's failure, or the store's.
pub fn disconnect(ctx: &mut AccountsCtx<'_>, id: AccountId) -> Result<(), CommandFailure> {
    let identity =
        store::account_identity(ctx.index.conn(), id).map_err(|e| account_failure(&e))?;

    // 1. The keychain, first. A `NotFound` entry is not a failure: the secret is already gone,
    //    and refusing here would strand the row for a user who cleared their keychain by hand.
    match ctx.tokens.delete(&identity.token_ref) {
        Ok(()) | Err(KeychainError::NotFound) => {}
        Err(error) => return Err(keychain_failure(&error)),
    }

    // 2. The rows. `delete_account` refuses if the cascade would not fire.
    let _tx_guard = TxGuard::enter();
    let tx = ctx.index.conn().unchecked_transaction().map_err(internal)?;
    store::delete_account(&tx, id).map_err(|e| account_failure(&e))?;
    tx.commit().map_err(internal)
}

/// Handles `accounts.disconnect`.
///
/// # Errors
/// Fails when arguments are malformed, the keychain refuses, or the store does.
pub fn handle_disconnect(ctx: &mut AccountsCtx<'_>, args: Value) -> Result<Value, CommandFailure> {
    let parsed: crate::protocol::AccountsDisconnectArgs = parse_args(args)?;
    disconnect(ctx, parsed.account_id)?;
    Ok(serde_json::json!({}))
}
