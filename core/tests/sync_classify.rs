#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! §21.8's response table, row by row, and **AC-P2-21-8**'s skew rule.
//!
//! The table has ten rows and **the count is asserted**, so a row cannot be dropped silently.
//! The three 403 shapes are three separate cases and not one: a test asserting only
//! *"403 → back off"* passes against the wrong behaviour and ships a retry loop against an
//! unauthorised token, which is the whole failure §21.8 is written to prevent.

use codotheca_core::http::{HttpResponse, TransportError};
use codotheca_core::sync::classify::{classify, translate_instant};
use codotheca_core::sync::outcome::{SyncOutcome, UnauthorizedReason};

/// Local wall clock during these tests.
const NOW: i64 = 1_800_000_000;
/// `Sat, 17 Jan 2027 06:13:20 GMT` — the epoch second `NOW` is, formatted as a `Date` header.
/// A server whose clock agrees with ours.
const DATE_IN_SYNC: &str = "Sat, 17 Jan 2027 06:13:20 GMT";
const SEVEN_HOURS: i64 = 7 * 3600;

/// A response **in the shape the transport hands over** — `Ok`, whatever the status. That is
/// `HttpTransport`'s contract and the reason this classifier can read a 403's headers at all, so
/// the wrapper is the point rather than an accident.
#[allow(clippy::unnecessary_wraps)]
fn response(status: u16, headers: &[(&str, &str)]) -> Result<HttpResponse, TransportError> {
    Ok(HttpResponse {
        status,
        headers: codotheca_core::http::normalise_headers(headers.iter().copied()),
        body: b"{}".to_vec(),
    })
}

/// An HTTP-date for an epoch second, in the spelling a forge actually sends.
fn http_date(epoch: i64) -> String {
    chrono::DateTime::from_timestamp(epoch, 0)
        .expect("representable")
        .format("%a, %d %b %Y %H:%M:%S GMT")
        .to_string()
}

/// **The ten rows of §21.8's table, each asserted for what it actually produces.**
///
/// The vector is the count: a row deleted from the table is a row deleted from here, and the
/// length assertion says so.
#[test]
fn every_row_of_the_response_table_classifies_to_its_own_outcome() {
    let reset = NOW + 900;
    let cases: Vec<(&str, Result<HttpResponse, TransportError>, SyncOutcome)> = vec![
        ("2xx with a body", response(200, &[]), SyncOutcome::Done),
        ("304", response(304, &[]), SyncOutcome::NotModified),
        (
            "429",
            response(
                429,
                &[
                    ("x-ratelimit-reset", &reset.to_string()),
                    ("date", DATE_IN_SYNC),
                ],
            ),
            SyncOutcome::Throttled {
                until: reset,
                secondary: false,
            },
        ),
        (
            "403 + x-ratelimit-remaining: 0 — a primary yield",
            response(
                403,
                &[
                    ("x-ratelimit-remaining", "0"),
                    ("x-ratelimit-reset", &reset.to_string()),
                    ("date", DATE_IN_SYNC),
                ],
            ),
            SyncOutcome::Throttled {
                until: reset,
                secondary: false,
            },
        ),
        (
            "403 + retry-after with remaining > 0 — a secondary limit",
            response(
                403,
                &[("retry-after", "30"), ("x-ratelimit-remaining", "4999")],
            ),
            SyncOutcome::Throttled {
                until: NOW + 30,
                secondary: true,
            },
        ),
        (
            "403 with neither header — terminal",
            response(403, &[]),
            SyncOutcome::Unauthorized {
                reason: UnauthorizedReason::Forbidden,
            },
        ),
        (
            "401",
            response(401, &[]),
            SyncOutcome::Unauthorized {
                reason: UnauthorizedReason::TokenInvalid,
            },
        ),
        ("404", response(404, &[]), SyncOutcome::NotFound),
        (
            "another 4xx",
            response(422, &[]),
            SyncOutcome::Rejected { status: 422 },
        ),
        (
            "5xx",
            response(503, &[]),
            SyncOutcome::TransientFail {
                reason: "server returned 503".to_owned(),
            },
        ),
    ];
    eprintln!("sync_classify: {} table rows exercised", cases.len());
    assert_eq!(cases.len(), 10, "§21.8's table has ten rows");

    for (name, res, expected) in cases {
        let (outcome, _rate) = classify(&res, NOW);
        assert_eq!(outcome, expected, "{name}");
    }
}

/// Row 10's other two halves. A timeout and a transport failure carry **no response**, so they
/// carry no headers to mirror — the snapshot is empty rather than zeroed.
#[test]
fn a_transport_failure_is_transient_and_mirrors_nothing() {
    for error in [
        TransportError::Timeout,
        TransportError::Connect {
            detail: "refused".to_owned(),
        },
        TransportError::Io {
            detail: "reset".to_owned(),
        },
    ] {
        let (outcome, rate) = classify(&Err(error.clone()), NOW);
        assert!(
            matches!(outcome, SyncOutcome::TransientFail { .. }),
            "{error:?} must be transient"
        );
        assert_eq!(rate.resource, None, "nothing was observed");
        assert_eq!(rate.remaining, None);
        assert_eq!(rate.limit, None);
        assert_eq!(rate.reset_at, None);
    }
}

/// §21.6: the rate headers are read from **every** response, error responses included. That
/// clause is the one a decorator exists to make structural, and it is asserted here on the
/// classifier that produces the snapshot.
#[test]
fn an_error_response_mirrors_its_rate_headers_exactly_as_a_200_does() {
    let reset = NOW + 60;
    let headers = [
        ("x-ratelimit-resource", "core"),
        ("x-ratelimit-remaining", "0"),
        ("x-ratelimit-limit", "5000"),
        ("x-ratelimit-reset", &*reset.to_string()),
        ("date", DATE_IN_SYNC),
    ];
    let (_, ok) = classify(&response(200, &headers), NOW);
    let (_, forbidden) = classify(&response(403, &headers), NOW);
    let (_, throttled) = classify(&response(429, &headers), NOW);
    assert_eq!(ok, forbidden, "a 403's headers must reach the budget");
    assert_eq!(ok, throttled, "a 429's headers must reach the budget");
    assert_eq!(ok.resource.as_deref(), Some("core"));
    assert_eq!(ok.remaining, Some(0));
    assert_eq!(ok.limit, Some(5000));
    assert_eq!(ok.reset_at, Some(reset));
}

/// **AC-P2-21-8.** `reset_at` is derived from the response's **own** `Date`, proven with a server
/// clock offset from the local one by **+7 hours** and by **−7 hours**, and with the header
/// **absent**.
///
/// A machine that copies the server's epoch verbatim either parks forever or never parks, and
/// the failure is invisible on a machine whose clock happens to be right.
#[test]
fn the_park_instant_is_translated_onto_our_clock_from_the_responses_own_date() {
    // The server says "you may retry 900 s from *my* now".
    let ahead = NOW + SEVEN_HOURS;
    let behind = NOW - SEVEN_HOURS;

    let (outcome, rate) = classify(
        &response(
            429,
            &[
                ("x-ratelimit-reset", &(ahead + 900).to_string()),
                ("date", &http_date(ahead)),
            ],
        ),
        NOW,
    );
    assert_eq!(rate.reset_at, Some(NOW + 900), "server clock +7h");
    assert_eq!(
        outcome,
        SyncOutcome::Throttled {
            until: NOW + 900,
            secondary: false
        }
    );

    let (_, rate) = classify(
        &response(
            429,
            &[
                ("x-ratelimit-reset", &(behind + 900).to_string()),
                ("date", &http_date(behind)),
            ],
        ),
        NOW,
    );
    assert_eq!(rate.reset_at, Some(NOW + 900), "server clock -7h");

    // No `Date`: the offset is zero and the value is used as given. The only honest fallback.
    let (_, rate) = classify(
        &response(429, &[("x-ratelimit-reset", &(ahead + 900).to_string())]),
        NOW,
    );
    assert_eq!(
        rate.reset_at,
        Some(ahead + 900),
        "with no Date header the server's epoch is used verbatim"
    );

    // The function directly, which is what the two cases above go through.
    assert_eq!(
        translate_instant(ahead + 900, Some(&http_date(ahead)), NOW),
        NOW + 900
    );
    assert_eq!(
        translate_instant(behind + 900, Some(&http_date(behind)), NOW),
        NOW + 900
    );
    assert_eq!(translate_instant(ahead + 900, None, NOW), ahead + 900);
}

/// `until` is the **later** of the two instants, not the first one found. A `retry-after` of 30 s
/// under a reset an hour out must not release the task in 30 s.
#[test]
fn the_park_takes_the_later_of_retry_after_and_the_reset() {
    let reset = NOW + 3600;
    let (outcome, _) = classify(
        &response(
            429,
            &[
                ("retry-after", "30"),
                ("x-ratelimit-reset", &reset.to_string()),
                ("date", DATE_IN_SYNC),
                ("x-ratelimit-remaining", "0"),
            ],
        ),
        NOW,
    );
    assert_eq!(
        outcome,
        SyncOutcome::Throttled {
            until: reset,
            secondary: false
        },
        "remaining == 0 is a primary yield however `retry-after` reads"
    );
}

/// `Retry-After` has two forms and they are **not** translated alike: delta-seconds is relative
/// to receipt and needs no skew correction, while an HTTP-date is an instant the server named.
#[test]
fn retry_after_is_read_in_both_of_its_forms() {
    let ahead = NOW + SEVEN_HOURS;
    let (delta, _) = classify(
        &response(
            403,
            &[
                ("retry-after", "120"),
                ("x-ratelimit-remaining", "10"),
                ("date", &http_date(ahead)),
            ],
        ),
        NOW,
    );
    assert_eq!(
        delta,
        SyncOutcome::Throttled {
            until: NOW + 120,
            secondary: true
        },
        "delta-seconds is relative to receipt, so the server's skew does not apply"
    );

    let (absolute, _) = classify(
        &response(
            403,
            &[
                ("retry-after", &http_date(ahead + 120)),
                ("x-ratelimit-remaining", "10"),
                ("date", &http_date(ahead)),
            ],
        ),
        NOW,
    );
    assert_eq!(
        absolute,
        SyncOutcome::Throttled {
            until: NOW + 120,
            secondary: true
        },
        "an HTTP-date is an instant the server named and is translated"
    );
}

/// R24's mirror, and the reason the terminal-403 reason set is three and not four: only these
/// three are observable, and each maps onto a **generated** `ErrorCode` read from p2-20's enum
/// rather than from a second spelling of it.
#[test]
fn every_unauthorized_reason_maps_onto_p2_20s_error_code() {
    use codotheca_core::protocol::ErrorCode;
    assert_eq!(
        UnauthorizedReason::TokenInvalid.error_code(),
        ErrorCode::TokenInvalid
    );
    assert_eq!(
        UnauthorizedReason::SsoRequired.error_code(),
        ErrorCode::SsoRequired
    );
    assert_eq!(
        UnauthorizedReason::Forbidden.error_code(),
        ErrorCode::TokenInvalid,
        "p2-20 collapses a missing scope and a revoked grant onto TOKEN_INVALID, and nothing in \
         the response separates them"
    );
    assert_eq!(UnauthorizedReason::TokenInvalid.slug(), "token_invalid");
    assert_eq!(UnauthorizedReason::SsoRequired.slug(), "sso_required");
    assert_eq!(UnauthorizedReason::Forbidden.slug(), "forbidden");
}

/// The SSO header is what separates the two terminal 403s, and it is the same detector p2-20
/// already uses (`core/src/accounts/commands.rs:113-129`) — one condition, one signal.
#[test]
fn a_terminal_403_carrying_the_sso_header_is_sso_required() {
    let (outcome, _) = classify(&response(403, &[("x-github-sso", "required")]), NOW);
    assert_eq!(
        outcome,
        SyncOutcome::Unauthorized {
            reason: UnauthorizedReason::SsoRequired
        }
    );
}

/// §21.8 step 9's second half lives with the code that parsed the page, and **only a `Done`
/// promotes**: a 304, a park or a refusal delivered no page, so there is no next one to ask for.
#[test]
fn only_a_done_is_promoted_to_a_next_page() {
    assert_eq!(
        SyncOutcome::Done.with_next_page(Some("c2".to_owned())),
        SyncOutcome::NextPage {
            cursor: "c2".to_owned()
        }
    );
    assert_eq!(SyncOutcome::Done.with_next_page(None), SyncOutcome::Done);
    assert_eq!(
        SyncOutcome::NotModified.with_next_page(Some("c2".to_owned())),
        SyncOutcome::NotModified
    );
    assert_eq!(
        SyncOutcome::NotFound.with_next_page(Some("c2".to_owned())),
        SyncOutcome::NotFound
    );
}
