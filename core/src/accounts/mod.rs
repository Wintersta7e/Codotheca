//! §20: accounts, credentials and the `accounts.*` command surface.
//!
//! A token reaches the database never. The database stores `token_ref`, the OS keychain stores
//! the secret, and the renderer holds neither.

pub mod commands;
pub mod device;
pub mod keychain;
pub mod pump;
pub mod store;

use serde_json::Value;

use crate::accounts::keychain::TokenStore;
use crate::index::Index;
use crate::proto::dispatch::CommandFailure;
use crate::provider::Provider;

/// Every `accounts.*` command declared by the protocol schema, in schema order.
pub const ACCOUNT_COMMANDS: [&str; 8] = [
    "accounts.list",
    "accounts.orgs",
    "accounts.connect",
    "accounts.cancelConnect",
    "accounts.connectPat",
    "accounts.upgradeScope",
    "accounts.disconnect",
    "accounts.setOrgEnabled",
];

/// Context for account commands. `now` is unix seconds supplied by the caller, so command handlers
/// do not read the wall clock themselves.
#[derive(Debug)]
pub struct AccountsCtx<'a> {
    pub index: &'a Index,
    pub provider: &'a dyn Provider,
    pub tokens: &'a dyn TokenStore,
    pub now: i64,
}

/// Dispatches the `accounts.*` subset implemented by this task.
///
/// This dispatcher answers the account commands that only read or write a row. The ones that
/// reach the network — `accounts.connect`, `accounts.cancelConnect`, `accounts.connectPat` and
/// `accounts.upgradeScope` — are answered by the assembly **without the index guard** (R75) and
/// never reach here.
/// `accounts.connect`, `accounts.cancelConnect`, `accounts.connectPat`,
/// `accounts.upgradeScope` and `accounts.disconnect` deliberately return `None` here because
/// later tasks still own them.
#[must_use]
pub fn dispatch_accounts_command(
    ctx: &mut AccountsCtx<'_>,
    command: &str,
    args: Value,
) -> Option<Result<Value, CommandFailure>> {
    match command {
        "accounts.list" => Some(commands::handle_list(ctx, args).and_then(|value| encode(&value))),
        "accounts.orgs" => Some(commands::handle_orgs(ctx, args).and_then(|value| encode(&value))),
        "accounts.setOrgEnabled" => {
            Some(commands::handle_set_org_enabled(ctx, args).and_then(|value| encode(&value)))
        }
        "accounts.disconnect" => Some(commands::handle_disconnect(ctx, args)),
        _ => None,
    }
}

fn encode<T: serde::Serialize>(value: &T) -> Result<Value, CommandFailure> {
    serde_json::to_value(value).map_err(|error| CommandFailure::internal(error.to_string()))
}
