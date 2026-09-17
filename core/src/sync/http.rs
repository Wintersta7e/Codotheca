//! The layer between p2-20's two seams: one decorator that observes **every** response.
//!
//! ```text
//! SyncRunner  ──drives──▶  Arc<dyn Provider>            p2-20 + p2-25: the typed seam
//!                                 │
//!                                 ▼
//!                     Arc<dyn HttpTransport>            THIS FILE — ObservingTransport
//!                                 │  · mirrors x-ratelimit-* from every response  (§21.6)
//!                                 │  · classifies status + headers → SyncOutcome  (§21.8)
//!                                 │  · pushes one HttpObservation per response
//!                                 ▼
//!                     Arc<dyn HttpTransport>            p2-20 — core/src/http/mod.rs (R66)
//!                          ReqwestTransport             the only reqwest::Client in the process
//! ```
//!
//! **It implements the same trait it wraps**, so p2-20's `GitHubProvider` takes it without
//! knowing it exists and every request the provider makes is mirrored whether or not its call
//! site remembered to. §21.6's rule is *every* response, including ones that carry no body and
//! ones the provider turns into an error; leaving that to each call site is one rule stated N
//! times — R12's shape.
//!
//! **It is pure.** It holds no `Index` and opens no transaction: it classifies, records, and
//! returns the response unchanged. The runner applies what it recorded, so the budget row and
//! the task row are written in the runner's own transactions where §21.9's rule 2 — *a value and
//! its clock commit together* — can be honoured.
//!
//! **This plan declares no HTTP client and adds no dependency** (R66). `HttpTransport`,
//! `HttpRequest`, `HttpResponse`, `RequestLimits`, `TransportError`, `ReqwestTransport` and the
//! testkit `FakeTransport` are all p2-20's, in `core/src/http/mod.rs`.
//!
//! **There is no `SYNC_LIMITS`, and that is a deviation from this plan's own task table.**
//! §21.2's budget — 10 s connect, 30 s total — is already declared once, as
//! `crate::http::ACCOUNT_LIMITS` (`core/src/http/mod.rs:44-55`), whose doc names §21.2 and whose
//! values `core/tests/http_transport.rs:428-441` already asserts. A second constant would be
//! that value stated twice (R12) **and would have no caller**: this runner constructs no
//! `HttpRequest` at all, because `GitHubProvider` builds every one of them
//! (`core/src/provider/github.rs:82,95,174`) and passes `ACCOUNT_LIMITS` itself. A constant whose
//! only readers are its own tests is R90's defect.

use std::sync::{Arc, Mutex, PoisonError};

use crate::clock::Clock;
use crate::http::{HttpRequest, HttpResponse, HttpTransport, TransportError};
use crate::sync::classify::{classify, RateSnapshot};
use crate::sync::outcome::SyncOutcome;

/// One response, as §21 saw it.
///
/// **R64: `HttpObservation`, never `Observation`.** `crate::git::observe::Observation<T>` is this
/// codebase's word for *a value carrying its freshness basis* — `{ value, basis, observed_at }` —
/// and §6's whole model is built on it. Different paths, so it would compile either way, which is
/// exactly why it would drift. Do not shorten it back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpObservation {
    pub outcome: SyncOutcome,
    pub rate: RateSnapshot,
    /// When this machine saw it, from the injected clock — never `SystemTime::now`.
    pub at: i64,
}

/// The decorator.
#[derive(Debug)]
pub struct ObservingTransport {
    inner: Arc<dyn HttpTransport>,
    clock: Arc<dyn Clock>,
    /// Drained per task step. It is deterministic because §21's own invariant is **one HTTP
    /// request in flight in the process**, which `core/tests/sync_runner.rs` asserts rather than
    /// assumes.
    seen: Mutex<Vec<HttpObservation>>,
}

impl ObservingTransport {
    #[must_use]
    pub fn new(inner: Arc<dyn HttpTransport>, clock: Arc<dyn Clock>) -> ObservingTransport {
        ObservingTransport {
            inner,
            clock,
            seen: Mutex::new(Vec::new()),
        }
    }

    /// Take everything observed since the last drain, leaving the channel empty.
    ///
    /// Emptying is the point: a channel that accumulated would make every later task step read an
    /// earlier step's budget and settle on an earlier step's outcome.
    #[must_use]
    pub fn drain(&self) -> Vec<HttpObservation> {
        let mut seen = self.seen.lock().unwrap_or_else(PoisonError::into_inner);
        std::mem::take(&mut seen)
    }
}

impl HttpTransport for ObservingTransport {
    fn send(&self, req: &HttpRequest) -> Result<HttpResponse, TransportError> {
        let at = self.clock.now_unix();
        let answer = self.inner.send(req);
        let (outcome, rate) = classify(&answer, at);
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(HttpObservation { outcome, rate, at });
        // Unchanged, header for header and byte for byte: the provider above must parse exactly
        // what it would have parsed undecorated.
        answer
    }
}
