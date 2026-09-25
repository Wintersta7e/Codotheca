//! `project_remote` — one repository's remote facts, on demand.
//!
//! **Three writing rules, each with a recorded defect behind it** (§21.9):
//!
//! 1. **A call that did not observe a value must not move that value's `observed_at`.** A
//!    skipped, parked, throttled, 401'd, 403'd, 404'd or rejected call moves **no** observation
//!    clock. This is what keeps the staleness marker honest.
//! 2. **A value and its clock commit in one transaction**, which is enforced by *where the
//!    transaction is opened* — here, in the caller — rather than by who writes inside it.
//! 3. **`NotModified` confirms the values and dates them.** A 304 carries no body, but it is an
//!    observation and not the absence of one: the server compared our validator and asserted the
//!    representation is unchanged as of now. **A 200 writes both, a 304 confirms both, a 403 or
//!    404 dates neither** (A14, A15).
//!
//! **This module declares no `remote_repo` writer, and that is R65 applied rather than a
//! convenience.** §25.7 owns the tables' shape, so it owns what writes them, and a second writer
//! for another plan's rows is R1's shape — it compiles, it passes its own tests, and it diverges
//! the first time the two disagree about what a 304 writes. There is **no thin wrapper** either:
//! a function that only forwards is a second name for one job, and the ambiguity *is* the defect.
//! `core/tests/sync_observation_clock.rs` asserts the absence structurally, over a printed file
//! count.
//!
//! **Two conditional reads, two validators, two clocks.** §25.1's field set is `repo_facts`
//! *and* `ci_runs`, and §25.7 gives the Actions read its own `ci_etag` and its own
//! `ci_observed_at` precisely because one call that did not observe the other's value must not
//! move it. The CI read is issued **only** after the facts read succeeded: spending a second
//! request against a budget the first one just found empty is the loop §21.10 exists to prevent.

use std::sync::Mutex;

use crate::identity::binding::RemoteBinding;
use crate::identity::remote::owner_of;
use crate::index::Index;
use crate::protocol::{AccountId, ProjectId};
use crate::provider::{CiRunsRead, Observed, RepoFactsRead};
use crate::remote::store::{
    confirm_ci_observation, confirm_repo_facts, mark_not_permitted, write_ci_runs, write_repo_facts,
};
use crate::sync::budget::mirror;
use crate::sync::outcome::SyncOutcome;
use crate::sync::{observe_one, token_for, SyncDeps, SyncError};

/// Everything the read needs, gathered under one brief lock before any socket is opened.
struct Target {
    account: AccountId,
    token_ref: String,
    owner: String,
    name: String,
    binding: RemoteBinding,
    etag: Option<String>,
    ci_etag: Option<String>,
}

/// The on-demand task: one repository's facts and its Actions runs.
///
/// **R94's second side.** This runs on the sync worker thread, so it takes `&Mutex<Index>` and
/// locks it itself — nothing above the worker holds the guard — and it releases the lock around
/// every request.
///
/// Returns `NotFound` **without issuing a request** when the project carries no forge binding or
/// no account can reach its host. That is a skip, not a refusal: there is nothing to ask and
/// nobody to ask, so no clock moves and no row is written. §21.8's *unseen, never gone* is the
/// same reading from the other direction — an absence of evidence is never evidence of absence.
///
/// # Errors
/// Fails when the index or the keychain refuses. A forge **refusal** is not an error: it is an
/// outcome, classified from the headers by the observing transport.
pub fn run_project_remote(
    deps: &SyncDeps,
    index: &Mutex<Index>,
    project: ProjectId,
) -> Result<SyncOutcome, SyncError> {
    let Some(target) = read_target(index, project) else {
        return Ok(SyncOutcome::NotFound);
    };
    let token = token_for(deps, &target.token_ref)?;

    // ---- The facts read. ---------------------------------------------------------------------
    let answer =
        deps.provider
            .repo_facts(&token, &target.owner, &target.name, target.etag.as_deref());
    let facts_observation = observe_one(deps, &answer);
    let facts: Option<RepoFactsRead> = answer.ok().map(|Observed { value, .. }| value);

    {
        let mut guard = index
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let outcome = facts_observation.outcome.clone();
        let at = facts_observation.at;
        let rate = facts_observation.rate;
        let binding = target.binding.clone();
        guard.with_tx(|tx| {
            // §21.6 first, and whatever the outcome: an error response's headers are the ones
            // that decide whether the next request is even issued.
            mirror(tx, Some(target.account), &rate, at)?;
            match (&outcome, &facts) {
                // A 200 writes the values, the validator and the clock — p2-25's writer, once.
                // The payload is **bound by the pattern**, not unwrapped after a guard: a guard
                // and an `expect` are two statements of one condition, and only one of them is
                // checked by the compiler.
                (
                    SyncOutcome::Done,
                    Some(RepoFactsRead {
                        facts: Some(payload),
                        etag,
                        ..
                    }),
                ) => write_repo_facts(tx, &binding, payload, etag.as_deref(), at)?,
                // A 304 confirms them. The server compared our validator and answered.
                (SyncOutcome::NotModified, _) => confirm_repo_facts(tx, &binding, at)?,
                // A 403 or a 404 observed the *access* state and nothing else.
                (SyncOutcome::Unauthorized { .. } | SyncOutcome::NotFound, _) => {
                    mark_not_permitted(tx, &binding)?;
                }
                // Parked, throttled, transient, rejected: nothing at all. **No call at all is
                // the correct write for a call that observed nothing.**
                _ => {}
            }
            Ok(())
        })?;
    }

    if !matches!(
        facts_observation.outcome,
        SyncOutcome::Done | SyncOutcome::NotModified
    ) {
        return Ok(facts_observation.outcome);
    }

    // ---- The Actions read, with its own validator and its own clock. -------------------------
    let answer = deps.provider.ci_runs(
        &token,
        &target.owner,
        &target.name,
        target.ci_etag.as_deref(),
    );
    let ci_observation = observe_one(deps, &answer);
    let runs: Option<CiRunsRead> = answer.ok().map(|Observed { value, .. }| value);

    {
        let mut guard = index
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let outcome = ci_observation.outcome.clone();
        let at = ci_observation.at;
        let rate = ci_observation.rate;
        let binding = target.binding.clone();
        guard.with_tx(|tx| {
            mirror(tx, Some(target.account), &rate, at)?;
            match (&outcome, &runs) {
                (
                    SyncOutcome::Done,
                    Some(CiRunsRead {
                        runs: Some(payload),
                        etag,
                        ..
                    }),
                ) => write_ci_runs(tx, &binding, payload, etag.as_deref(), at)?,
                (SyncOutcome::NotModified, _) => confirm_ci_observation(tx, &binding, at)?,
                // A refused Actions read says nothing about the facts beside it, so it marks
                // nothing: `permitted` is the facts read's answer and this one must not forge it.
                _ => {}
            }
            // §21.9: a sync write that changes an input to a derived value recomputes it before
            // this transaction commits. §5.2's description chain is the phase-2 case.
            crate::derive::persist::recompute(tx, project, at)?;
            Ok(())
        })?;
    }

    // A refused CI read settles the task, because the task is *this repository's remote facts*
    // and half of them were not observed. A `Done` here is the whole read having landed.
    if !matches!(
        ci_observation.outcome,
        SyncOutcome::Done | SyncOutcome::NotModified
    ) {
        return Ok(ci_observation.outcome);
    }
    Ok(facts_observation.outcome)
}

/// The binding, the stored validators and an account that can reach the host — under one lock,
/// before any socket is opened.
///
/// `None` is *nothing to ask, or nobody to ask with*, and the caller turns it into a skip.
///
/// **The account is chosen by provider and host, not from `project_account`.** That table is
/// §20.7's and **nothing writes it** — verified across `core/src/`, where the only mention is the
/// disconnect census. Picking the enabled account whose provider matches the binding and whose
/// host matches the stored `remote_key` is what this build can actually establish; reading a
/// table with no producer would be reading an absence as a fact. Recorded in
/// `.dev/reports/p2-21.md` rather than filled by writing another section's table.
fn read_target(index: &Mutex<Index>, project: ProjectId) -> Option<Target> {
    let guard = index
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let conn = guard.conn();

    let Ok((provider, repo_id, basis, remote_key)) = conn.query_row(
        "SELECT provider, provider_repo_id, remote_link_basis, remote_key
           FROM project WHERE id = ?1 AND merged_into IS NULL",
        [project.0],
        |r| {
            Ok((
                r.get::<_, Option<String>>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, Option<String>>(3)?,
            ))
        },
    ) else {
        return None;
    };
    let (Some(provider), Some(repo_id), Some(remote_key)) = (provider, repo_id, remote_key) else {
        return None;
    };
    // From the stored `remote_key`, never from `project.owner` and `project.name` — those are a
    // display name and a directory basename, and neither is a forge path.
    let (Some(host), Some(owner), Some(name)) = (
        remote_key.split_once('/').map(|(h, _)| h),
        owner_of(&remote_key),
        remote_key.rsplit('/').next(),
    ) else {
        return None;
    };

    let (account, token_ref) = account_row_for(conn, &provider, host)?;

    let (etag, ci_etag) = conn
        .query_row(
            "SELECT etag, ci_etag FROM remote_repo
              WHERE provider = ?1 AND provider_repo_id = ?2",
            rusqlite::params![provider, repo_id],
            |r| {
                Ok((
                    r.get::<_, Option<String>>(0)?,
                    r.get::<_, Option<String>>(1)?,
                ))
            },
        )
        .unwrap_or((None, None));

    Some(Target {
        account,
        token_ref,
        owner: owner.to_owned(),
        name: name.to_owned(),
        binding: RemoteBinding {
            provider,
            provider_repo_id: repo_id,
            remote_link_basis: basis.as_deref().and_then(basis_of),
        },
        etag,
        ci_etag,
    })
}

/// Which account this runner would read a project's remote facts with, or `None` when none can.
///
/// **One owner.** The runner asks this before issuing, so the budget it checks is the pool the
/// read would actually spend from; `read_target` asks it again for the token. A second expression
/// of *which account reads this project* would let the reserve guard one pool while the request
/// drains another.
///
/// # Errors
/// Never: an unreadable row, an unbound project and an account that does not exist are all the
/// same answer — **nothing to ask with** — and none of them is a fault.
#[must_use]
pub fn account_for_project(conn: &rusqlite::Connection, project: ProjectId) -> Option<AccountId> {
    let (provider, remote_key) = conn
        .query_row(
            "SELECT provider, remote_key FROM project WHERE id = ?1 AND merged_into IS NULL",
            [project.0],
            |r| {
                Ok((
                    r.get::<_, Option<String>>(0)?,
                    r.get::<_, Option<String>>(1)?,
                ))
            },
        )
        .ok()?;
    let host = remote_key.as_deref()?.split_once('/')?.0;
    account_row_for(conn, provider.as_deref()?, host).map(|(account, _)| account)
}

/// The enabled account on this provider and this host, with its keychain entry name.
fn account_row_for(
    conn: &rusqlite::Connection,
    provider: &str,
    host: &str,
) -> Option<(AccountId, String)> {
    conn.query_row(
        "SELECT id, token_ref FROM account
          WHERE is_enabled = 1 AND provider = ?1 AND host = ?2
          ORDER BY id LIMIT 1",
        rusqlite::params![provider, host],
        |r| Ok((AccountId(r.get::<_, i64>(0)?), r.get::<_, String>(1)?)),
    )
    .ok()
}

/// The stored basis, through the generated enum rather than a second spelling of it (R24).
fn basis_of(stored: &str) -> Option<crate::protocol::RemoteLinkBasis> {
    serde_json::from_value(serde_json::Value::String(stored.to_owned())).ok()
}
