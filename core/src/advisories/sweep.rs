//! §32.2's sweep: **one process-wide scheduled task, unauthenticated, batched, resumable.**
//!
//! It reads the distinct `(ecosystem, package_name, version)` triples the whole library holds,
//! batched, and writes four library-wide facts — the advisory, its CVE ids, the match, and the
//! record that the question was asked. **No project row, no working copy, no git invocation.**
//!
//! **The batch is the transaction, and a batch is one request.** A sweep over a real library costs
//! more than one request and may cost more than one hour's allowance, so each batch settles
//! `NextPage` and the next pick re-reads the budget before issuing again. That is §21.4's existing
//! park-to-`reset_at` machinery doing what it was built for; it is **not** a batching knob, and a
//! library whose first sweep spans several reset windows simply has triples that have not been
//! asked about yet — whose verdict is `unknown`, because the absence of an `advisory_triple` row
//! says exactly that.
//!
//! **§21.9 rule 1 binds unchanged: a call that did not observe a value must not move that value's
//! `observed_at`.** A parked, throttled, refused or failed sweep dates nothing.

use std::sync::Mutex;

use crate::advisories::store::{
    close_sweep, fold_response, open_sweep, unanswered_triples, ADVISORY_SWEEP_BATCH,
};
use crate::advisories::{ADVISORY_AFFECTS_BYTE_CAP, ADVISORY_BATCH_CAP};
use crate::index::Index;
use crate::protocol::Ecosystem;
use crate::provider::PackageVersion;
use crate::sync::budget::mirror;
use crate::sync::outcome::SyncOutcome;
use crate::sync::{observe_one, SyncDeps, SyncError};

/// The cursor a settle carries when the **next batch** is due but no page link is outstanding.
///
/// A sentinel rather than an empty string, because an empty cursor and no cursor are the same
/// value in a nullable TEXT column and the two mean different things here.
pub const NEXT_BATCH: &str = "next-batch";

/// One batch of one sweep.
///
/// Returns `NextPage` while anything is left to ask about and `Done` when the sweep is complete.
/// The open sweep row is found by its own `settled_at IS NULL`, so a restart resumes rather than
/// starting a second one.
///
/// # Errors
/// Fails when the index refuses. A forge **refusal** is not an error: it is an outcome, classified
/// from the headers by the observing transport.
pub fn run_advisory_sweep(
    deps: &SyncDeps,
    index: &Mutex<Index>,
    cursor: Option<&str>,
) -> Result<SyncOutcome, SyncError> {
    let now = deps.clock.now_unix();
    let page_link = cursor.filter(|c| *c != NEXT_BATCH).map(str::to_owned);

    let (sweep_id, batch) = {
        let mut guard = index
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.with_tx(|tx| {
            let sweep_id = open_sweep(tx, now)?;
            let batch = next_batch(tx, sweep_id)?;
            Ok((sweep_id, batch))
        })?
    };

    let Some((ecosystem, asked)) = batch else {
        // Nothing left to ask about: the sweep is complete and its row settles.
        let mut guard = index
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.with_tx(|tx| close_sweep(tx, sweep_id, now, "done", true))?;
        return Ok(SyncOutcome::Done);
    };

    let answer = deps
        .provider
        .advisories(ecosystem, &asked, page_link.as_deref());
    let observed = observe_one(deps, &answer);

    let mut guard = index
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    guard.with_tx(|tx| {
        // §21.6's *every response*: the rate headers are mirrored against the **per-IP** pool,
        // which is the one this task spends from and the one keyed by the absence of an account.
        mirror(tx, None, &observed.rate, observed.at)?;
        // The resource this endpoint answers from, persisted so the **next** pre-issue read is
        // keyed by it rather than by a process-wide guess.
        if let Some(resource) = observed.rate.resource.as_deref() {
            crate::advisories::store::note_sweep_resource(tx, sweep_id, resource)?;
        }
        Ok(())
    })?;

    let page = match answer {
        Ok(page) => page.value,
        // The classifier turned the response into an outcome. **Nothing is dated**: no
        // `advisory_triple` row is written, so every triple in this batch stays unasked and every
        // project reading it stays `unknown` rather than becoming `clean`.
        Err(_) => return Ok(observed.outcome),
    };

    let more_pages = page.next_cursor.clone();
    let complete_answer = more_pages.is_none();
    guard.with_tx(|tx| {
        fold_response(
            tx,
            sweep_id,
            ecosystem,
            &asked,
            &page.items,
            complete_answer,
            now,
        )
        .map(|_| ())
    })?;

    Ok(SyncOutcome::NextPage {
        cursor: more_pages.unwrap_or_else(|| NEXT_BATCH.to_owned()),
    })
}

/// The next batch to ask about: one ecosystem, and a package name **at most once**.
///
/// The endpoint answers with the advisories matching *any* of the asked pairs and names the
/// affected **package** rather than the pair that matched, so two versions of one package in one
/// request would be indistinguishable in the answer. Keeping a name unique per request is what
/// makes the fold's attribution honest.
///
/// Bounded in **two** dimensions, because the measured ceiling is a URL byte length and not a pair
/// count: a cap expressed only as a count is right for short names and refused with HTTP 414 the
/// first time a library resolves a scoped one.
fn next_batch(
    tx: &rusqlite::Transaction<'_>,
    sweep_id: i64,
) -> Result<Option<(Ecosystem, Vec<PackageVersion>)>, crate::index::IndexError> {
    for ecosystem in Ecosystem::ALL {
        let pending = unanswered_triples(tx, ecosystem, sweep_id, ADVISORY_SWEEP_BATCH)?;
        if pending.is_empty() {
            continue;
        }
        let mut asked: Vec<PackageVersion> = Vec::new();
        let mut used = 0usize;
        for item in pending {
            if asked.len() >= ADVISORY_BATCH_CAP {
                break;
            }
            if asked.iter().any(|a| a.name == item.name) {
                continue;
            }
            let cost = encoded_cost(&item);
            if used + cost > ADVISORY_AFFECTS_BYTE_CAP && !asked.is_empty() {
                break;
            }
            used += cost;
            asked.push(item);
        }
        if !asked.is_empty() {
            return Ok(Some((ecosystem, asked)));
        }
    }
    Ok(None)
}

/// What one `name@version` pair costs in the percent-encoded query, separator included.
///
/// Derived from the same encoding the provider applies, so the budget and the request cannot
/// disagree about how big a pair is.
fn encoded_cost(item: &PackageVersion) -> usize {
    let unreserved = |b: u8| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~');
    let width = |s: &str| -> usize { s.bytes().map(|b| if unreserved(b) { 1 } else { 3 }).sum() };
    // `@` and the `,` separator each encode to three bytes.
    width(&item.name) + width(&item.version) + 3 + 3
}
