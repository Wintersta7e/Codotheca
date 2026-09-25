//! §21.8's decision procedure. **The status does not decide — header presence does.**
//!
//! The question this answers was posed as *"what a 403 and a 429 each do"*, and it is the wrong
//! question: two 403s can be **opposite** outcomes, and the failure the settled row actually
//! names — an expired token — arrives as **401**. A classifier written to the question as posed
//! ships a retry loop against an unauthorised token.
//!
//! Everything here is pure: no I/O, no database, no clock of its own. `now_local` arrives from
//! the caller's [`crate::clock::Clock`], which is what makes the skew rule testable at all.

use crate::http::{HttpResponse, TransportError};
use crate::sync::outcome::{SyncOutcome, UnauthorizedReason};

/// What the rate headers said, **whatever the status**.
///
/// Every field is optional because **unobserved is unknown, not zero**, and a response that
/// carried no `x-ratelimit-resource` cannot be keyed at all — §21.6 — so it is not mirrored and
/// dates nothing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RateSnapshot {
    /// `x-ratelimit-resource`: the pool this response spent from, and the key its budget row is
    /// mirrored under. `None` means the response cannot be mirrored at all.
    pub resource: Option<String>,
    /// `x-ratelimit-remaining`: requests left in that pool.
    pub remaining: Option<i64>,
    /// `x-ratelimit-limit`: the pool's size per window.
    pub limit: Option<i64>,
    /// **On our clock, not the server's** — see [`translate_instant`].
    pub reset_at: Option<i64>,
}

/// An instant the server named, moved onto this machine's clock.
///
/// `now_local + (server_epoch − date_server)`, from the response's **own** `Date` header. A
/// machine with a skewed clock that copied the server's epoch verbatim would either park forever
/// or never park, and the skew is exactly the kind of thing nobody notices until a user's clock
/// is wrong.
///
/// **With no `Date` header the offset is zero and the value is used as given.** That is the only
/// honest fallback — inventing an offset would be worse than trusting the one number we have —
/// and it is recorded rather than treated as a failure.
#[must_use]
pub fn translate_instant(server_epoch: i64, date_header: Option<&str>, now_local: i64) -> i64 {
    let Some(date_server) = date_header.and_then(parse_http_date) else {
        return server_epoch;
    };
    now_local.saturating_add(server_epoch.saturating_sub(date_server))
}

/// An HTTP-date into epoch seconds. `None` for anything that does not parse, which is then the
/// no-`Date` case above rather than a failure.
fn parse_http_date(raw: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc2822(raw.trim())
        .ok()
        .map(|d| d.timestamp())
}

/// The first value under `name` as an `i64`, or `None` when it is absent or not a number.
fn header_i64(res: &HttpResponse, name: &str) -> Option<i64> {
    res.header(name)?.trim().parse::<i64>().ok()
}

/// `Retry-After` in **either** of its two forms, as an instant on our clock.
///
/// A delta-seconds value is relative to *receipt* and therefore needs no skew translation:
/// `now_local + secs`. An HTTP-date is an absolute instant the **server** named, and is
/// translated like every other server instant. Treating the two alike is how a park lands hours
/// away in the wrong direction.
fn retry_after_instant(res: &HttpResponse, now_local: i64) -> Option<i64> {
    let raw = res.header("retry-after")?.trim();
    if let Ok(secs) = raw.parse::<i64>() {
        return Some(now_local.saturating_add(secs.max(0)));
    }
    let at = parse_http_date(raw)?;
    Some(translate_instant(at, res.header("date"), now_local))
}

/// Step 1: pull `x-ratelimit-*` out of **any** response, error responses included.
#[must_use]
pub fn rate_snapshot(res: &HttpResponse, now_local: i64) -> RateSnapshot {
    RateSnapshot {
        resource: res.header("x-ratelimit-resource").map(str::to_owned),
        remaining: header_i64(res, "x-ratelimit-remaining"),
        limit: header_i64(res, "x-ratelimit-limit"),
        reset_at: header_i64(res, "x-ratelimit-reset")
            .map(|at| translate_instant(at, res.header("date"), now_local)),
    }
}

/// §21.8, in order. Header presence decides; the status alone never does.
///
/// Returns the outcome **and** the snapshot, because step 1 runs whatever the status and the
/// caller mirrors it into `sync_budget` even for the responses it is about to call a failure.
/// A 2xx is `Done` here; the task that parsed the page promotes it with
/// [`SyncOutcome::with_next_page`], because the cursor lives in the body and this function is
/// deliberately blind to it.
#[must_use]
pub fn classify(
    res: &Result<HttpResponse, TransportError>,
    now_local: i64,
) -> (SyncOutcome, RateSnapshot) {
    // 7 (first half): a transport failure carries no response, and therefore no headers to
    // mirror. The snapshot is empty rather than zeroed — nothing was observed.
    let res = match res {
        Ok(res) => res,
        Err(error) => {
            return (
                SyncOutcome::TransientFail {
                    reason: error.to_string(),
                },
                RateSnapshot::default(),
            );
        }
    };

    let rate = rate_snapshot(res, now_local);
    let outcome = match res.status {
        // 2. The failure the settled row actually names, and the one the original question
        //    omitted. Terminal, and never retried on a backoff.
        401 => SyncOutcome::Unauthorized {
            reason: UnauthorizedReason::TokenInvalid,
        },
        // 3 and 4. Two 403s can be opposite outcomes and only the headers say which.
        403 => classify_forbidden(res, &rate, now_local),
        429 => throttled(res, &rate, now_local),
        // 5. Per-resource and **never terminal for the account**: the repository is *unseen*,
        //    never *gone*. No row is deleted, and §22's rename probe is the resolution path.
        404 => SyncOutcome::NotFound,
        // 8.
        304 => SyncOutcome::NotModified,
        // 9. The task promotes this to `NextPage` if the page it parsed carried a cursor.
        200..=299 => SyncOutcome::Done,
        // 7 (second half).
        500..=599 => SyncOutcome::TransientFail {
            reason: format!("server returned {}", res.status),
        },
        // 6. Any other 4xx. Terminal: a request this client formed wrongly will be formed
        //    wrongly again, and retrying it spends the allowance to learn nothing.
        400..=499 => SyncOutcome::Rejected { status: res.status },
        other => SyncOutcome::Rejected { status: other },
    };
    (outcome, rate)
}

/// §21.8 steps 3 and 4 over a 403, which is the whole reason this procedure is header-driven.
fn classify_forbidden(res: &HttpResponse, rate: &RateSnapshot, now_local: i64) -> SyncOutcome {
    let has_retry_after = res.header("retry-after").is_some();
    let out_of_budget = rate.remaining == Some(0);
    if !has_retry_after && !out_of_budget {
        // Terminal. A missing scope, an unauthorised SSO organisation or revoked access — none
        // of the three is fixed by waiting, so waiting is not offered.
        let reason = if res.header("x-github-sso").is_some() {
            UnauthorizedReason::SsoRequired
        } else {
            UnauthorizedReason::Forbidden
        };
        return SyncOutcome::Unauthorized { reason };
    }
    throttled(res, rate, now_local)
}

/// `until` is the **later** of `retry-after` and `x-ratelimit-reset`, both on our clock.
///
/// It is a **secondary** limit iff `retry-after` is present **and** `remaining > 0`; otherwise it
/// is a primary yield. The two are carried distinctly because §21.4 backs them off differently,
/// and by the time `apply_outcome` runs the headers are gone.
///
/// With neither header `until` is `now_local`, which is this module saying **the server named no
/// instant** — not saying *retry now*. Inventing a delay here would be a number nothing observed,
/// so the floor lives where an outcome becomes a clock:
/// `crate::sync::state::apply_outcome` raises an already-expired park to
/// `crate::sync::state::SYNC_UNNAMED_PARK_SECS`. Unfloored, this shape was measured at 2,413
/// requests in 500 ms against a forge already answering `429`.
fn throttled(res: &HttpResponse, rate: &RateSnapshot, now_local: i64) -> SyncOutcome {
    let retry_after = retry_after_instant(res, now_local);
    let until = [retry_after, rate.reset_at]
        .into_iter()
        .flatten()
        .max()
        .unwrap_or(now_local);
    let secondary = retry_after.is_some() && rate.remaining.is_some_and(|n| n > 0);
    SyncOutcome::Throttled { until, secondary }
}
