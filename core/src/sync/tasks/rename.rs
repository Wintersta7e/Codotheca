//! `rename_probe` — §22.7's repair, scheduled.
//!
//! **This task implements nothing of the repair.** `crate::identity::rename_repair::repair_renames`
//! is p2-22's and is the only thing that decides which projects are unmatched, issues the
//! lookups, and writes the bindings. Before this lane it had **no production caller** —
//! `core/tests/identity_rename_repair.rs` was its only one — which is R90's condition exactly: a
//! correct, fully tested function nothing in the product ever ran. This is the caller.
//!
//! **Keyed by account, not by project, and that is a deviation from §21.3's table.** §21.3 keys
//! the task by project id and expects one probe per unmatched `remote_key`. p2-22 shipped the
//! repair as **one bounded pass** (`core/src/identity/rename_repair.rs:50-97`) whose own bound is
//! *one request per unmatched key, once* (`:11`, `:104-111`) — the same bound stated per pass
//! rather than per task. Re-keying it per project would mean a second copy of that selection
//! query and that loop, against the module p2-22 owns, which is R1's shape and what R65 rules
//! against. The `key` column is polymorphic by design, so an account id there is exactly as
//! representable as a project id. Recorded in `.dev/reports/p2-21.md`.
//!
//! **A `404` is `NotFound`, and `NotFound` is `ok`** — never `blocked`, never a deletion. The
//! repository is *unseen*, never *gone*: absence is not evidence, and §22.6's third form binds
//! every plan.

use std::sync::{Arc, Mutex};

use crate::identity::rename_repair::repair_renames;
use crate::index::Index;
use crate::protocol::AccountId;
use crate::provider::declared_host_aliases;
use crate::sync::budget::mirror;
use crate::sync::outcome::SyncOutcome;
use crate::sync::{settle_of, token_for, SyncDeps, SyncError};

/// Repair every unmatched `remote_key` this account's forge can resolve, once.
///
/// **R94's second side**: a worker-thread signature. It takes the `Arc` because
/// `repair_renames` does — that function locks the index itself, around its writes and never
/// around a request.
///
/// The settle is the **worst** of however many lookups the pass issued: one 401 among twenty
/// answers is still a token that does not authenticate, and settling `ok` on the strength of the
/// nineteen would leave the account looking healthy.
///
/// # Errors
/// Fails when the index or the keychain refuses. A forge **refusal** is not an error.
pub fn run_rename_probe(
    deps: &SyncDeps,
    index: &Arc<Mutex<Index>>,
    account: AccountId,
) -> Result<SyncOutcome, SyncError> {
    let now = deps.clock.now_unix();
    let token_ref = {
        let guard = index
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        crate::accounts::store::account_identity(guard.conn(), account)
            .map_err(|e| SyncError::Account(e.to_string()))?
            .token_ref
    };
    let token = token_for(deps, &token_ref)?;

    // p2-22's pass. It holds no guard across a lookup, and it writes only what resolved.
    let report = repair_renames(
        index,
        deps.provider.as_ref(),
        &token,
        &declared_host_aliases(),
        now,
    )
    .map_err(|e| SyncError::Account(format!("{e:?}")))?;

    // Every lookup the pass made, mirrored: §21.6's *every response* does not stop being true
    // because one task step issued more than one.
    let observed = deps.transport.drain();
    {
        let mut guard = index
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.with_tx(|tx| {
            for one in &observed {
                mirror(tx, Some(account), &one.rate, one.at)?;
            }
            Ok(())
        })?;
    }

    // A pass that asked nothing settles `Done`: there was nothing unmatched, which is success and
    // not an absence of information.
    debug_assert!(report.attempted >= report.resolved);
    Ok(settle_of(&observed))
}
