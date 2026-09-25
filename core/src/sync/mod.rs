//! §21 — remote sync, the runner.
//!
//! **Sync is its own runner and is not a job** (§21.1). It adds no `JobKind` variant, writes no
//! `project_job_state` row and does not ride `JobOutcome::Partial`, which stays declared, unused
//! and untouched by phase 2 (A2). Six structural blockers put it here rather than in the job
//! queue, and the two that decide it are the shape of the work and the shape of the budget: a
//! sync task has no `location_id` and the queue coalesces on one, and no admission concept in
//! `core::jobs` can express *a budget shared across every task, refilling at a wall-clock instant
//! the server names*.
//!
//! One dedicated blocking thread carrying **one HTTP request in flight at a time**, assembled as
//! `SyncPump` beside `JobPump` with the same cancel-then-join stop discipline.
//!
//! **This module declares no HTTP client and adds no dependency** (R66). `core::http` is p2-20's;
//! what lives here is `http::ObservingTransport`, the decorator that mirrors `x-ratelimit-*` from
//! **every** response — error responses included — and runs §21.8's classification.

pub mod budget;
pub mod classify;
pub mod commands;
pub mod events;
pub mod http;
pub mod outcome;
pub mod progress;
pub mod runner;
pub mod schedule;
pub mod state;
pub mod store;
pub mod task;
pub mod tasks;

use std::sync::Arc;

use crate::accounts::keychain::{SecretToken, TokenStore};
use crate::cancel::CancelToken;
use crate::clock::Clock;
use crate::index::IndexError;
use crate::provider::{Provider, ProviderError};
use crate::sync::http::{HttpObservation, ObservingTransport};
use crate::sync::outcome::SyncOutcome;
use crate::sync::task::SyncTask;

/// What a sync task cannot recover from.
///
/// **A forge refusal is not one of these.** A 401, a 403, a 404 and a 429 are *outcomes*,
/// classified from the response's headers and settled onto the task's row; turning them into
/// errors here would throw away the headers §21.6 mirrors and §21.8 reads. What is left is the
/// machine failing: the index, the keychain, or an account row that is gone.
#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    /// The index refused a read or a write the task needed to settle.
    #[error("sync index write failed: {0}")]
    Index(#[from] IndexError),
    /// The account's identity or org rows could not be read — or the rename pass over its
    /// repositories failed — carrying the underlying message.
    #[error("sync could not read the account: {0}")]
    Account(String),
    /// The keychain refused the account's token, or holds no entry under its `token_ref`.
    #[error("sync could not read the token: {0}")]
    Keychain(String),
}

/// Everything a task needs that is not the index.
pub struct SyncDeps {
    /// The forge client every task issues its requests through.
    pub provider: Arc<dyn Provider>,
    /// The **decorated** transport, held so a task can drain what the provider's call observed.
    /// It is the same object the provider sends through, which is what makes the drain the
    /// provider's own responses rather than a parallel record of them.
    pub transport: Arc<ObservingTransport>,
    /// The keychain each account's token is read from, through [`token_for`].
    pub tokens: Arc<dyn TokenStore>,
    /// The runner's wall clock, in Unix seconds: settles, parks and budget checks all read it.
    pub clock: Arc<dyn Clock>,
    /// The loop's stop token: once `SyncPump::stop` cancels it the loop takes no further task,
    /// while a request already in flight runs to its own time bound.
    pub cancel: CancelToken,
    /// **A property of the machine, read ONCE by the composition root and passed down as data**
    /// — `ProjectsCtx.tz_offset_min`'s twin (`core/src/clock.rs:65-67`). §28.4's `debt_day` key
    /// is a **local** date and §28's singleton evaluator settles from here, so nothing below the
    /// root reaches for a zone of its own.
    pub tz_offset_min: i32,
}

impl std::fmt::Debug for SyncDeps {
    /// Written by hand: a `TokenStore` has no business in a log line even by name, and `Provider`
    /// and `Clock` carry nothing a reader wants.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SyncDeps").finish_non_exhaustive()
    }
}

/// §21.5's priority, as a property of the task rather than a parameter.
///
/// **True for `ProjectRemote` alone.** Named for what §21.5 actually says — *"only **on-demand**
/// tasks may spend"*. **Not `is_scheduled`**: §21.5's *"Only `account_repos` is scheduled"* is
/// about **cadence**, and `RenameProbe` has a trigger rather than a cadence, so a predicate called
/// `is_scheduled` would have to answer `true` for it and contradict the sentence it was named
/// after. The reserve keys on the on-demand side, which is exactly one task.
#[must_use]
pub const fn is_on_demand(task: &SyncTask) -> bool {
    matches!(task, SyncTask::ProjectRemote { .. })
}

/// The token for one account's keychain entry.
///
/// # Errors
/// Fails when the keychain refuses or holds no entry under that name.
pub fn token_for(deps: &SyncDeps, token_ref: &str) -> Result<SecretToken, SyncError> {
    deps.tokens
        .read(token_ref)
        .map_err(|e| SyncError::Keychain(e.to_string()))
}

/// The one observation a provider call produced.
///
/// §21's own invariant is **one HTTP request in flight on the runner's thread**, so a drain after
/// one call yields exactly that call's response. It is *not* an invariant of the process: the
/// Device Flow pump issues requests through the same decorator from its own thread, on purpose,
/// so that §21.6 sees those responses too. `ObservingTransport::drain` returning this thread's
/// observations only is what makes the sentence above true of the assembled product rather than
/// of the runner in isolation; `core/tests/sync_runner.rs` asserts the one-in-flight half rather
/// than assuming it.
///
/// **The empty case is real and is not a gap.** A provider call can fail before it reaches the
/// transport — a body that will not decode is the live case — and there is then no response to
/// classify. It becomes a `TransientFail` carrying the provider's own words: three attempts, then
/// `deferred`, which is the right budget for a forge answering in a shape this build cannot read.
#[must_use]
pub fn observe_one<T>(deps: &SyncDeps, answer: &Result<T, ProviderError>) -> HttpObservation {
    let mut drained = deps.transport.drain();
    if let Some(last) = drained.pop() {
        return last;
    }
    let reason = match answer {
        Ok(_) => "the provider answered without issuing a request".to_owned(),
        Err(e) => e.to_string(),
    };
    HttpObservation {
        outcome: SyncOutcome::TransientFail { reason },
        rate: classify::RateSnapshot::default(),
        at: deps.clock.now_unix(),
    }
}

/// The settle for a task step that made **several** requests.
///
/// The **worst** outcome governs: one 401 among twenty answers is still a token that does not
/// authenticate, and settling `ok` on the strength of the other nineteen would leave the account
/// looking healthy while every later request fails the same way. An empty slice is `Done` — a
/// pass that asked nothing succeeded at asking nothing, which is not the same as observing a
/// failure.
#[must_use]
pub fn settle_of(observed: &[HttpObservation]) -> SyncOutcome {
    observed
        .iter()
        .max_by_key(|o| severity(&o.outcome))
        .map_or(SyncOutcome::Done, |o| o.outcome.clone())
}

/// How much one outcome should govern a settle, worst last.
///
/// Terminal refusals outrank parks because waiting does not fix them; a park outranks a transient
/// failure because the server named a time and a retry before it would spend the allowance to be
/// refused again.
const fn severity(outcome: &SyncOutcome) -> u8 {
    match outcome {
        SyncOutcome::Done | SyncOutcome::NextPage { .. } => 0,
        SyncOutcome::NotModified => 1,
        SyncOutcome::NotFound => 2,
        SyncOutcome::TransientFail { .. } => 3,
        SyncOutcome::Throttled { .. } => 4,
        SyncOutcome::Rejected { .. } => 5,
        SyncOutcome::Unauthorized { .. } => 6,
    }
}
