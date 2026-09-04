//! A scripted [`HttpTransport`], and the record of what was actually sent.
//!
//! It records the **request** rather than only answering it, because most of what §20 has to
//! prove about the transport is a claim about what did *not* reach the wire — no
//! `Authorization` a caller did not set, no cookie, no `Referer`. An assertion about an absence
//! needs the sent request in hand.

use std::collections::VecDeque;
use std::sync::Mutex;

use crate::http::{HttpRequest, HttpResponse, HttpTransport, TransportError};

/// One scripted outcome.
#[derive(Debug, Clone)]
enum Scripted {
    Response(HttpResponse),
    Failure(TransportError),
}

#[derive(Debug, Default)]
pub struct FakeTransport {
    scripted: Mutex<VecDeque<Scripted>>,
    sent: Mutex<Vec<HttpRequest>>,
}

impl FakeTransport {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Queue a response. Answers are handed out in the order they were pushed.
    pub fn push(&self, response: HttpResponse) {
        self.scripted
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push_back(Scripted::Response(response));
    }

    /// Queue a transport failure — a timeout, a connect failure or an IO failure. These are the
    /// only outcomes that carry no response, and therefore no headers to mirror.
    pub fn push_err(&self, error: TransportError) {
        self.scripted
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push_back(Scripted::Failure(error));
    }

    /// Every request that reached the transport, in order.
    #[must_use]
    pub fn requests(&self) -> Vec<HttpRequest> {
        self.sent
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// How many requests reached the transport. The counting half of the seam: a `Provider`
    /// helper that issues a request fails an assertion that this stayed at zero.
    #[must_use]
    pub fn request_count(&self) -> usize {
        self.sent
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len()
    }
}

impl HttpTransport for FakeTransport {
    fn send(&self, req: &HttpRequest) -> Result<HttpResponse, TransportError> {
        self.sent
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(req.clone());
        let next = self
            .scripted
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .pop_front();
        match next {
            Some(Scripted::Response(response)) => Ok(response),
            Some(Scripted::Failure(error)) => Err(error),
            // An unscripted call is a test bug, not a network condition. Saying so here beats
            // answering `200 {}` and letting the assertion fail somewhere unrelated.
            None => Err(TransportError::Io {
                detail: format!("FakeTransport has no scripted answer for {}", req.url),
            }),
        }
    }
}
