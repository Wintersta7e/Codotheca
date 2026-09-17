#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! The observing decorator: §21.6's *every response, including error responses*, made structural.
//!
//! **The fakes fake the transport, never the provider.** §21.8's classification and §21.6's
//! mirroring are assertions about the code under the provider; a fake above it would assert the
//! fake.
//!
//! What this file does **not** assert is how many `reqwest::Client`s exist or whether the core
//! contains an `async fn` — `core/tests/http_transport.rs` owns both, over the whole of
//! `core/src/`, and two files asserting one rule is R12's shape.

use std::sync::Arc;

use codotheca_core::accounts::keychain::SecretToken;
use codotheca_core::http::{HttpRequest, HttpResponse, HttpTransport, TransportError};
use codotheca_core::provider::{GitHubProvider, Provider};
use codotheca_core::sync::http::ObservingTransport;
use codotheca_core::sync::outcome::SyncOutcome;
use codotheca_core::testing::{FakeClock, FakeTransport};

const NOW: i64 = 1_800_000_000;

fn clock() -> Arc<FakeClock> {
    Arc::new(FakeClock::new(NOW))
}

fn request(headers: Vec<(String, String)>) -> HttpRequest {
    HttpRequest {
        method: "GET",
        url: "https://forge.example.invalid/user".to_owned(),
        headers,
        body: None,
        limits: codotheca_core::http::ACCOUNT_LIMITS,
    }
}

fn response(status: u16, headers: &[(&str, &str)], body: &[u8]) -> HttpResponse {
    HttpResponse {
        status,
        headers: codotheca_core::http::normalise_headers(headers.iter().copied()),
        body: body.to_vec(),
    }
}

/// **(a) The decorator injects nothing, and neither does the seam under it.**
///
/// p2-25b's whole gate rests on this: the README asset fetch runs over the same transport against
/// arbitrary badge and image hosts, so a default `Authorization` anywhere in the chain sends a
/// forge token to a stranger. Adding a header on top of a seam is possible; removing one the
/// seam already added is not.
#[test]
fn the_decorator_adds_no_header_the_caller_did_not_write() {
    let inner = Arc::new(FakeTransport::new());
    inner.push(response(200, &[], b"{}"));
    inner.push(response(200, &[], b"{}"));
    let observing = ObservingTransport::new(Arc::clone(&inner) as Arc<dyn HttpTransport>, clock());

    observing.send(&request(Vec::new())).expect("sent");
    observing
        .send(&request(vec![(
            "authorization".to_owned(),
            "Bearer caller-wrote-this".to_owned(),
        )]))
        .expect("sent");

    let sent = inner.requests();
    assert_eq!(sent.len(), 2);
    assert!(
        sent[0].headers.is_empty(),
        "the decorator added {:?}",
        sent[0].headers
    );
    assert_eq!(
        sent[1].headers,
        vec![(
            "authorization".to_owned(),
            "Bearer caller-wrote-this".to_owned()
        )],
        "the caller's own header must arrive unchanged and unaccompanied"
    );
}

/// The same claim one layer up: every header a decorated **provider** call puts on the wire is
/// one the provider wrote. A failure here is p2-20's shape being wrong, and it is reported rather
/// than patched around.
#[test]
fn a_decorated_provider_call_carries_only_the_headers_the_provider_wrote() {
    let inner = Arc::new(FakeTransport::new());
    inner.push(response(200, &[], br#"{"login":"someone"}"#));
    let observing = Arc::new(ObservingTransport::new(
        Arc::clone(&inner) as Arc<dyn HttpTransport>,
        clock(),
    ));
    let provider = GitHubProvider::new(
        Arc::clone(&observing) as Arc<dyn HttpTransport>,
        "forge.example.invalid".to_owned(),
    );
    provider
        .viewer(&SecretToken::new("t".to_owned()))
        .expect("viewer");

    let sent = inner.requests();
    assert_eq!(sent.len(), 1);
    let names: Vec<&str> = sent[0].headers.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(
        names,
        [
            "authorization",
            "accept",
            "x-github-api-version",
            "user-agent"
        ],
        "a header nobody wrote reached the wire"
    );
    assert_eq!(
        sent[0]
            .headers
            .iter()
            .filter(|(n, _)| n == "cookie")
            .count(),
        0
    );
    assert_eq!(
        sent[0]
            .headers
            .iter()
            .filter(|(n, _)| n == "referer")
            .count(),
        0
    );
}

/// **(b) Every response drains exactly one observation**, a transport failure included, and the
/// count is printed. Fewer than six means a shape reached the wire unobserved.
#[test]
fn every_response_produces_exactly_one_observation() {
    let inner = Arc::new(FakeTransport::new());
    inner.push(response(200, &[], b"{}"));
    inner.push(response(304, &[], b""));
    inner.push(response(403, &[], b""));
    inner.push(response(429, &[], b""));
    inner.push(response(404, &[], b""));
    inner.push_err(TransportError::Timeout);
    let observing = ObservingTransport::new(Arc::clone(&inner) as Arc<dyn HttpTransport>, clock());

    for _ in 0..6 {
        let _ = observing.send(&request(Vec::new()));
    }
    let drained = observing.drain();
    eprintln!(
        "sync_transport: {} observation(s) from {} response(s)",
        drained.len(),
        inner.request_count()
    );
    assert_eq!(drained.len(), 6, "one observation per response, no more");
    let kinds: Vec<SyncOutcome> = drained.iter().map(|o| o.outcome.clone()).collect();
    assert!(matches!(kinds[0], SyncOutcome::Done));
    assert!(matches!(kinds[1], SyncOutcome::NotModified));
    assert!(matches!(kinds[2], SyncOutcome::Unauthorized { .. }));
    assert!(matches!(kinds[3], SyncOutcome::Throttled { .. }));
    assert!(matches!(kinds[4], SyncOutcome::NotFound));
    assert!(matches!(kinds[5], SyncOutcome::TransientFail { .. }));
    for observed in &drained {
        assert_eq!(observed.at, NOW, "every observation is dated by the clock");
    }
}

/// **(c) §21.6's *including error responses*.** This is the clause a decorator exists to make
/// structural: leaving it to each call site is one rule stated N times, and the site that forgets
/// is the one whose budget silently never updates.
#[test]
fn an_error_responses_rate_headers_reach_the_observation_exactly_as_a_200s_do() {
    let reset = NOW + 600;
    let rate: Vec<(&str, &str)> = vec![
        ("x-ratelimit-resource", "core"),
        ("x-ratelimit-remaining", "0"),
        ("x-ratelimit-limit", "5000"),
        (
            "x-ratelimit-reset",
            Box::leak(reset.to_string().into_boxed_str()),
        ),
    ];
    let inner = Arc::new(FakeTransport::new());
    inner.push(response(200, &rate, b"{}"));
    inner.push(response(403, &rate, b""));
    inner.push(response(429, &rate, b""));
    let observing = ObservingTransport::new(Arc::clone(&inner) as Arc<dyn HttpTransport>, clock());
    for _ in 0..3 {
        let _ = observing.send(&request(Vec::new()));
    }

    let drained = observing.drain();
    assert_eq!(drained.len(), 3);
    assert_eq!(drained[0].rate.resource.as_deref(), Some("core"));
    assert_eq!(drained[0].rate.remaining, Some(0));
    assert_eq!(drained[0].rate.limit, Some(5000));
    assert_eq!(drained[0].rate.reset_at, Some(reset));
    assert_eq!(drained[1].rate, drained[0].rate, "the 403's headers");
    assert_eq!(drained[2].rate, drained[0].rate, "the 429's headers");
}

/// **(d) The response comes back unchanged**, so the provider parses exactly what it would have
/// parsed undecorated. A decorator that rewrote a header or truncated a body would be a second
/// transport wearing the first one's name.
#[test]
fn the_inner_response_is_returned_untouched() {
    let original = response(
        403,
        &[("x-ratelimit-remaining", "0"), ("x-github-sso", "required")],
        b"{\"message\":\"nope\"}",
    );
    let inner = Arc::new(FakeTransport::new());
    inner.push(original.clone());
    let observing = ObservingTransport::new(Arc::clone(&inner) as Arc<dyn HttpTransport>, clock());
    let got = observing.send(&request(Vec::new())).expect("a 403 is Ok");
    assert_eq!(got, original);
}

/// A transport failure stays a failure: the decorator records it and re-raises it, so a caller
/// cannot mistake an observed failure for a response.
#[test]
fn a_transport_failure_is_recorded_and_re_raised() {
    let inner = Arc::new(FakeTransport::new());
    inner.push_err(TransportError::Connect {
        detail: "refused".to_owned(),
    });
    let observing = ObservingTransport::new(Arc::clone(&inner) as Arc<dyn HttpTransport>, clock());
    let error = observing.send(&request(Vec::new())).expect_err("must fail");
    assert!(matches!(error, TransportError::Connect { .. }));
    assert_eq!(observing.drain().len(), 1);
}

/// **(f) A drain claims this thread's responses and nobody else's.**
///
/// `core/src/main.rs` hands one provider — and so one decorator — to the sync runner **and** to
/// the accounts Device Flow pump, on purpose: §21.6 wants the poll's `x-ratelimit-*` in
/// `sync_budget` too. So *one request in flight* is true of the runner's thread and not of the
/// process, and an untagged channel let a poll landing mid-task be taken as that task's own
/// observation while the task's real response was discarded unmirrored.
#[test]
fn a_drain_takes_this_threads_responses_and_leaves_another_threads() {
    let inner = Arc::new(FakeTransport::new());
    for _ in 0..3 {
        inner.push(response(200, &[("x-ratelimit-resource", "core")], b"{}"));
    }
    let observing = Arc::new(ObservingTransport::new(
        Arc::clone(&inner) as Arc<dyn HttpTransport>,
        clock(),
    ));

    // Another thread's request, standing in for the Device Flow poll.
    {
        let other = Arc::clone(&observing);
        std::thread::spawn(move || {
            other.send(&request(Vec::new())).expect("sent");
        })
        .join()
        .expect("joined");
    }
    observing.send(&request(Vec::new())).expect("sent");

    let mine = observing.drain();
    assert_eq!(mine.len(), 1, "a drain claimed another thread's response");
    let foreign = observing.drain_foreign();
    assert_eq!(foreign.len(), 1, "the other thread's response was lost");
    assert!(
        observing.drain_foreign().is_empty(),
        "a foreign drain must empty what it took"
    );
}

/// **(e) `drain` empties the channel**, so a second task step sees only its own responses. A
/// channel that accumulated would make every later step read an earlier step's budget.
#[test]
fn draining_empties_the_channel() {
    let inner = Arc::new(FakeTransport::new());
    inner.push(response(200, &[], b"{}"));
    inner.push(response(200, &[], b"{}"));
    let observing = ObservingTransport::new(Arc::clone(&inner) as Arc<dyn HttpTransport>, clock());

    observing.send(&request(Vec::new())).expect("sent");
    assert_eq!(observing.drain().len(), 1);
    assert_eq!(observing.drain().len(), 0, "the first drain took them all");
    observing.send(&request(Vec::new())).expect("sent");
    assert_eq!(
        observing.drain().len(),
        1,
        "only the second step's response"
    );
}
