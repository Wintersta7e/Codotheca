//! `sync_task_state` reads and writes.
//!
//! Every function here takes a `&Transaction` or a `&Connection` **from its caller**, which is
//! how every writer in this codebase is shaped; the caller opens the transaction through
//! `crate::proto::txguard::TxGuard`.

use rusqlite::Transaction;

use crate::index::IndexError;
use crate::protocol::AccountId;
use crate::sync::task::kind_slug;

/// Clear the one account-referencing table that can carry no foreign key.
///
/// **`sync_task_state` has no key into `account(id)` and cannot have one**: `key` is polymorphic
/// (§21.3) — an account id for `account_repos`, a *project* id for `project_remote` and
/// `rename_probe`. So `delete_account` cannot lean on a cascade here, and it calls this by name.
///
/// **The task filter is not optional.** A bare `DELETE FROM sync_task_state WHERE key = ?1`
/// satisfies every enumeration over the census while deleting another task's rows for whichever
/// project happens to share that integer — a bar written past its defect on the one command whose
/// job is to delete. That is why `("sync_task_state", "key")` is not representable under
/// `ACCOUNT_REFERENCING_TABLES`, and why the census is a census (R69).
///
/// Returns the number of rows removed, so a caller or a test can see that it did something.
///
/// # Errors
/// Fails when SQLite refuses the delete.
pub fn delete_account_tasks(tx: &Transaction<'_>, account: AccountId) -> Result<usize, IndexError> {
    let removed = tx.execute(
        "DELETE FROM sync_task_state WHERE task = ?1 AND key = ?2",
        rusqlite::params![
            kind_slug(crate::protocol::SyncTaskKind::AccountRepos),
            account.0
        ],
    )?;
    Ok(removed)
}
