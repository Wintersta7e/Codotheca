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

use crate::index::Index;
use crate::proto::dispatch::CommandFailure;

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

/// Context for the account commands answered **under the index guard**.
///
/// It carries the index and nothing else, and that is the point: the guarded arm must not be
/// *able* to reach the forge or the keychain, because holding the process's one SQLite mutex
/// across either has now been fixed twice. A `provider` and a `tokens` here were read by nothing
/// and were an invitation to do it a third time; R75 is structural rather than a convention.
#[derive(Debug)]
pub struct AccountsCtx<'a> {
    pub index: &'a Index,
}

/// Dispatches the `accounts.*` subset implemented by this task.
///
/// This dispatcher answers the account commands that **only read a row**. Every command that
/// makes an unbounded call — `accounts.connect`, `accounts.cancelConnect`, `accounts.connectPat`,
/// `accounts.upgradeScope`, `accounts.setOrgEnabled` and `accounts.disconnect` — is answered by
/// the assembly **without the index guard** (R75), returns `None` here, and never reaches this
/// match. `disconnect` is on that list for its **keychain** round trip, not a forge one: nothing
/// bounds it either.
#[must_use]
pub fn dispatch_accounts_command(
    ctx: &mut AccountsCtx<'_>,
    command: &str,
    args: Value,
) -> Option<Result<Value, CommandFailure>> {
    match command {
        "accounts.list" => Some(commands::handle_list(ctx, args).and_then(|value| encode(&value))),
        "accounts.orgs" => Some(commands::handle_orgs(ctx, args).and_then(|value| encode(&value))),
        _ => None,
    }
}

fn encode<T: serde::Serialize>(value: &T) -> Result<Value, CommandFailure> {
    serde_json::to_value(value).map_err(|error| CommandFailure::internal(error.to_string()))
}
