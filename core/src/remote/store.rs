//! §25.7's three tables, and the one rule every writer here obeys.
//!
//! **A call that did not observe a value must not move that value's clock.**
//!
//! | Outcome | `observed_at` / `ci_observed_at` | Other columns |
//! |---|---|---|
//! | `200` | **moves** | values and the validator written |
//! | `304` | **moves** — the server compared our validator and asserted the representation current (A14) | unchanged |
//! | `403` / `404` | **does not move** | `permitted = 0`, and nothing else |
//! | skipped · parked · throttled · transient failure | **does not move** | nothing |
//!
//! **A value and its clock commit in one transaction.** Every function here takes a
//! `&Transaction` from its caller, which is how every writer in this codebase is shaped; the
//! caller opens it through `crate::proto::txguard::TxGuard`.
//!
//! **Keyed on `(provider, provider_repo_id)`, never on `remote_key`** (§22.9, §25.7).
//! `remote_key` is non-unique by requirement (§1.1) and two live projects can legitimately carry
//! one, so keying facts on it would make one project's read overwrite another's.
//!
//! **R65 assigns these to §25 rather than to §21**, in as many words: *"it needs the `Provider`
//! fetch methods **and** the persistence functions for `remote_repo`, `remote_topic` and
//! `remote_ci_run`, all of which §25.7 owns because it owns the tables' shape."* §21's
//! `run_project_remote` is the task driver and is the production caller, in wave 5.

use rusqlite::Transaction;

use crate::identity::binding::RemoteBinding;
use crate::index::IndexError;
use crate::provider::{CiRunPayload, RepoFactsPayload};
use crate::remote::facts::CI_RUN_LIMIT;

/// Ensure the pair has a row, creating one with **no clock and no value** if it has none.
///
/// A row that exists with `observed_at IS NULL` reads as *not observed*, which is exactly what a
/// pair nobody has read yet is — so creating one costs no honesty and gives every other writer a
/// row to update.
fn ensure_row(tx: &Transaction<'_>, binding: &RemoteBinding) -> Result<(), IndexError> {
    let (provider, repo_id) = binding.key();
    tx.execute(
        "INSERT INTO remote_repo (provider, provider_repo_id) VALUES (?1, ?2)
         ON CONFLICT(provider, provider_repo_id) DO NOTHING",
        rusqlite::params![provider, repo_id],
    )?;
    Ok(())
}

/// The `200` path: every value, the validator and the clock, in one statement.
///
/// The topic set is replaced **inside the same transaction**, because a topic rail dated by a
/// read that did not write it is the same lie one table along.
///
/// # Errors
/// Fails when SQLite refuses a write.
pub fn write_repo_facts(
    tx: &Transaction<'_>,
    binding: &RemoteBinding,
    facts: &RepoFactsPayload,
    etag: Option<&str>,
    now: i64,
) -> Result<(), IndexError> {
    ensure_row(tx, binding)?;
    let (provider, repo_id) = binding.key();
    tx.execute(
        "UPDATE remote_repo
            SET visibility = ?3, description = ?4, fork_parent_remote_key = ?5,
                stars = ?6, open_issues = ?7, good_first_issues = ?8,
                open_prs = ?9, open_prs_from_user = ?10,
                permitted = 1, observed_at = ?11, etag = ?12
          WHERE provider = ?1 AND provider_repo_id = ?2",
        rusqlite::params![
            provider,
            repo_id,
            facts.visibility,
            facts.description,
            facts.fork_parent_remote_key,
            facts.stars,
            facts.open_issues,
            facts.good_first_issues,
            facts.open_prs,
            facts.open_prs_from_user,
            now,
            etag,
        ],
    )?;
    write_topics(tx, binding, &facts.topics)
}

/// The `304` path (A14): the clock moves and **no value is rewritten**.
///
/// The server compared the validator this row already holds and said the representation is
/// current, which is an observation of every value in it. Rewriting the values from a response
/// that carried none is the same defect from the other direction.
///
/// # Errors
/// Fails when SQLite refuses the write.
pub fn confirm_repo_facts(
    tx: &Transaction<'_>,
    binding: &RemoteBinding,
    now: i64,
) -> Result<(), IndexError> {
    let (provider, repo_id) = binding.key();
    tx.execute(
        "UPDATE remote_repo SET observed_at = ?3
          WHERE provider = ?1 AND provider_repo_id = ?2",
        rusqlite::params![provider, repo_id, now],
    )?;
    Ok(())
}

/// Replace the pair's **whole** topic set.
///
/// A topic that is gone from the forge is gone here: a set assembled by insertion only would
/// keep every topic the repository has ever carried and render a rail nobody can explain.
///
/// # Errors
/// Fails when SQLite refuses a write.
pub fn write_topics(
    tx: &Transaction<'_>,
    binding: &RemoteBinding,
    topics: &[String],
) -> Result<(), IndexError> {
    let (provider, repo_id) = binding.key();
    tx.execute(
        "DELETE FROM remote_topic WHERE provider = ?1 AND provider_repo_id = ?2",
        rusqlite::params![provider, repo_id],
    )?;
    for topic in topics {
        tx.execute(
            "INSERT INTO remote_topic (provider, provider_repo_id, topic) VALUES (?1, ?2, ?3)
             ON CONFLICT(provider, provider_repo_id, topic) DO NOTHING",
            rusqlite::params![provider, repo_id, topic],
        )?;
    }
    Ok(())
}

/// The Actions `200` path: the runs, the validator and **`ci_observed_at`** together, then the
/// trim to [`CI_RUN_LIMIT`].
///
/// §25.7 says the rows are *"trimmed by §21"* while §21.3's table says the task writes *§25's
/// rows*. Under R65 the writer is here, and the trim is a property of the table rather than of
/// the scheduler that calls it — so it lives beside the insert. Recorded as a reading, not
/// assumed.
///
/// # Errors
/// Fails when SQLite refuses a write.
pub fn write_ci_runs(
    tx: &Transaction<'_>,
    binding: &RemoteBinding,
    runs: &[CiRunPayload],
    etag: Option<&str>,
    now: i64,
) -> Result<(), IndexError> {
    ensure_row(tx, binding)?;
    let (provider, repo_id) = binding.key();
    for run in runs {
        tx.execute(
            "INSERT INTO remote_ci_run
               (provider, provider_repo_id, run_id, workflow_name, conclusion, branch,
                run_number, started_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(provider, provider_repo_id, run_id) DO UPDATE SET
                 workflow_name = excluded.workflow_name,
                 conclusion = excluded.conclusion,
                 branch = excluded.branch,
                 run_number = excluded.run_number,
                 started_at = excluded.started_at",
            rusqlite::params![
                provider,
                repo_id,
                run.run_id,
                run.workflow_name,
                run.conclusion,
                run.branch,
                run.run_number,
                run.started_at,
            ],
        )?;
    }
    // Most recent first, and the same order the reader uses — a trim that kept a different five
    // from the ones the surface draws would delete a row the page is showing.
    tx.execute(
        "DELETE FROM remote_ci_run
          WHERE provider = ?1 AND provider_repo_id = ?2
            AND run_id NOT IN (
                SELECT run_id FROM remote_ci_run
                 WHERE provider = ?1 AND provider_repo_id = ?2
                 ORDER BY started_at DESC, run_id DESC
                 LIMIT ?3)",
        rusqlite::params![provider, repo_id, i64::try_from(CI_RUN_LIMIT).unwrap_or(5)],
    )?;
    tx.execute(
        "UPDATE remote_repo SET ci_observed_at = ?3, ci_etag = ?4
          WHERE provider = ?1 AND provider_repo_id = ?2",
        rusqlite::params![provider, repo_id, now, etag],
    )?;
    Ok(())
}

/// The Actions `304` path: `ci_observed_at` moves and no run is rewritten.
///
/// # Errors
/// Fails when SQLite refuses the write.
pub fn confirm_ci_observation(
    tx: &Transaction<'_>,
    binding: &RemoteBinding,
    now: i64,
) -> Result<(), IndexError> {
    let (provider, repo_id) = binding.key();
    tx.execute(
        "UPDATE remote_repo SET ci_observed_at = ?3
          WHERE provider = ?1 AND provider_repo_id = ?2",
        rusqlite::params![provider, repo_id, now],
    )?;
    Ok(())
}

/// The `403` / `404` path: `permitted = 0`, and **nothing is dated**.
///
/// `permitted` has no wire field, which is why it can only be written here: `RemoteFacts` carries
/// a `state` rather than a flag, so no function taking a `&RemoteFacts` can reach the column.
/// Dating the counts by a read that returned none of them is the staleness marker lying.
///
/// # Errors
/// Fails when SQLite refuses the write.
pub fn mark_not_permitted(tx: &Transaction<'_>, binding: &RemoteBinding) -> Result<(), IndexError> {
    ensure_row(tx, binding)?;
    let (provider, repo_id) = binding.key();
    tx.execute(
        "UPDATE remote_repo SET permitted = 0 WHERE provider = ?1 AND provider_repo_id = ?2",
        rusqlite::params![provider, repo_id],
    )?;
    Ok(())
}
