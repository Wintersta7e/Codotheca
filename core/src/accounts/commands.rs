//! Handlers for the account command subset owned by this task.

use serde_json::Value;

use crate::accounts::keychain::KeychainError;
use crate::accounts::store::{self, AccountError};
use crate::accounts::AccountsCtx;
use crate::proto::dispatch::{parse_args, CommandFailure};
use crate::proto::txguard::TxGuard;
use crate::protocol::{
    Account, AccountOrg, AccountsListArgs, AccountsOrgsArgs, AccountsSetOrgEnabledArgs, ErrorCode,
    SsoState,
};
use crate::provider::ProviderError;

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
