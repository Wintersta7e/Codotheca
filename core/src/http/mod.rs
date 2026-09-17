//! The one HTTP client in the product (R66), and the untyped seam every subsystem reads through.
//!
//! `core::provider` sits on top of it typed; §21's `ObservingTransport` decorates it to mirror
//! `x-ratelimit-*` from **every** response into the rate budget; §25's README asset fetch
//! consumes it directly with its own [`RequestLimits`]. One client, one owner.
//!
//! # The transport surfaces raw status, headers and body, and injects nothing
//!
//! **A non-2xx is `Ok`, not `Err`, and that is load-bearing.** A `403` carrying
//! `x-ratelimit-remaining: 0` is a *response*; returning it as an error with the headers dropped
//! destroys the input the rate-budget classification reads — header presence decides, and the
//! status alone never does. Only a timeout, a connection failure or an IO failure is a
//! [`TransportError`].
//!
//! **It injects nothing, and this is the one rule that cannot be fixed later.** No default
//! `Authorization`, no cookie jar, no `Referer`, no automatic credential helper. Every header on
//! the wire comes from [`HttpRequest::headers`]. The reason is four waves away and is not
//! hypothetical: the README asset fetch runs over this transport against arbitrary badge and
//! image hosts, so a default `Authorization` here sends a forge token to a stranger. Adding a
//! header on top of this seam is possible; removing one the transport already added is not.
//!
//! # No async of ours
//!
//! `reqwest`'s blocking client is implemented over its async one and starts a background tokio
//! runtime, so tokio **is** in the tree. What is not in the tree is any async code of ours, which
//! is what "the core is blocking threads" actually means here. `core/tests/http_transport.rs`
//! asserts it over every file under `core/src/`.

use std::io::Read as _;
use std::time::Duration;

/// The per-call bounds. Public fields, so each consumer builds its own rather than sharing a
/// constant whose values would then have to suit all of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestLimits {
    pub connect_secs: u64,
    pub total_secs: u64,
    pub max_body_bytes: usize,
    pub redirect_limit: u8,
    /// A redirect that changes scheme is a downgrade to plaintext. Never allowed by §20's calls.
    pub allow_scheme_change: bool,
}

/// §21.2's budget for an account call: 10 s to connect, 30 s in total.
///
/// `redirect_limit` is 3 rather than 0 because a forge answers a renamed repository with a
/// permanent redirect, and `max_body_bytes` caps a listing page at 4 MiB — an answer larger than
/// that is a malfunction, not a large page.
pub const ACCOUNT_LIMITS: RequestLimits = RequestLimits {
    connect_secs: 10,
    total_secs: 30,
    max_body_bytes: 4 * 1024 * 1024,
    redirect_limit: 3,
    allow_scheme_change: false,
};

#[derive(Clone, PartialEq, Eq)]
pub struct HttpRequest {
    pub method: &'static str,
    pub url: String,
    /// Every header that reaches the wire. The transport adds none.
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
    pub limits: RequestLimits,
}

/// Header names whose **values** are secrets. A `Debug` that printed one would put a bearer
/// token into the rolling log the moment anyone debugged a request, which is the single most
/// likely way a token reaches stderr.
const SECRET_HEADERS: &[&str] = &["authorization", "proxy-authorization", "cookie"];

/// Written by hand: the derived `Debug` printed `Authorization: Bearer <token>` verbatim, and
/// `core/tests/accounts_redaction.rs` caught it. Header **names** are kept — which headers were
/// sent is diagnostic — and the values of the secret-bearing ones are replaced. The body is
/// summarised by length for the same reason: a form-encoded body carries the device code.
impl std::fmt::Debug for HttpRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let headers: Vec<(&str, &str)> = self
            .headers
            .iter()
            .map(|(name, value)| {
                if SECRET_HEADERS
                    .iter()
                    .any(|secret| name.eq_ignore_ascii_case(secret))
                {
                    (name.as_str(), "<redacted>")
                } else {
                    (name.as_str(), value.as_str())
                }
            })
            .collect();
        f.debug_struct("HttpRequest")
            .field("method", &self.method)
            .field("url", &self.url)
            .field("headers", &headers)
            .field("body_bytes", &self.body.as_ref().map(Vec::len))
            .field("limits", &self.limits)
            .finish()
    }
}

/// A response, whatever its status.
///
/// `headers` is a `Vec` and not a map: header names repeat legitimately, and folding them loses
/// the information a doubled `retry-after` carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl HttpResponse {
    /// The **first** value under `name`, matched case-insensitively.
    ///
    /// First, not last and not joined: a caller that wants to know a name arrived twice asks
    /// [`HttpResponse::header_count`], which is what keeps the duplication visible instead of
    /// silently resolved.
    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        let needle = name.to_ascii_lowercase();
        self.headers
            .iter()
            .find(|(k, _)| *k == needle)
            .map(|(_, v)| v.as_str())
    }

    /// How many times `name` arrived. Zero, one, or the count that makes a doubled header a
    /// fact the caller can act on.
    #[must_use]
    pub fn header_count(&self, name: &str) -> usize {
        let needle = name.to_ascii_lowercase();
        self.headers.iter().filter(|(k, _)| *k == needle).count()
    }
}

/// The three failures that carry no response at all, and therefore no headers to mirror.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TransportError {
    #[error("the request timed out")]
    Timeout,
    #[error("could not connect: {detail}")]
    Connect { detail: String },
    #[error("transport failure: {detail}")]
    Io { detail: String },
}

pub trait HttpTransport: Send + Sync + std::fmt::Debug {
    fn send(&self, req: &HttpRequest) -> Result<HttpResponse, TransportError>;
}

/// Header names lowercased, every line kept in arrival order.
///
/// Lowercasing on the way in means one spelling reaches every consumer; keeping every line means
/// a name that arrived twice is still two entries, which is the whole reason `headers` is a
/// `Vec`.
pub fn normalise_headers<'a, I>(raw: I) -> Vec<(String, String)>
where
    I: IntoIterator<Item = (&'a str, &'a str)>,
{
    raw.into_iter()
        .map(|(k, v)| (k.to_ascii_lowercase(), v.to_owned()))
        .collect()
}

/// Read at most `max` bytes, refusing a body that exceeds it.
///
/// It reads **`max + 1`** bytes at the very most, so a body ten times the cap costs one byte
/// over the cap rather than the whole transfer — the refusal is a bound on what is read, not a
/// check applied after reading everything.
pub fn read_capped<R: std::io::Read>(reader: R, max: usize) -> Result<Vec<u8>, TransportError> {
    let mut body = Vec::new();
    let read = reader
        .take(max as u64 + 1)
        .read_to_end(&mut body)
        .map_err(|e| TransportError::Io {
            detail: e.to_string(),
        })?;
    if read > max {
        return Err(TransportError::Io {
            detail: body_cap_detail(max),
        });
    }
    Ok(body)
}

/// The detail an over-cap body carries, **stated once**.
///
/// A consumer that renders *too large* differently from *could not be reached* has to tell the
/// two apart, and `TransportError` deliberately carries only the three failures that have no
/// response at all — widening that enum is a change to every match in the tree. So the string has
/// one owner instead: [`read_capped`] writes it and §25.5's asset fetch compares against this
/// same function rather than against a second copy of the words.
#[must_use]
pub fn body_cap_detail(max: usize) -> String {
    format!("response body exceeds {max} bytes")
}

/// `https` and nothing else. A plaintext URL carrying a bearer token is the failure this refuses
/// before a socket is opened.
pub fn require_https(url: &str) -> Result<(), TransportError> {
    if url.starts_with("https://") {
        return Ok(());
    }
    Err(TransportError::Connect {
        detail: "only https is allowed".to_owned(),
    })
}

/// Header names whose **values authenticate the caller**. They may never cross a host boundary.
///
/// `reqwest`'s own redirect policy strips these on a cross-origin hop; `Policy::none()` opts out
/// of that, and this transport follows redirects itself so that `redirect_limit` and
/// `allow_scheme_change` stay per-request. Opting out of the policy therefore means opting **in**
/// to doing the stripping here, and the first version of this loop did not.
pub const CREDENTIAL_HEADERS: &[&str] = &["authorization", "cookie", "proxy-authorization"];

/// Whether two URLs address the same host, comparing host and port and ignoring case.
///
/// # Errors
/// Fails when either URL cannot be parsed, because an unparseable next hop is not a hop this
/// transport is willing to guess about.
pub fn same_host(base: &str, next: &str) -> Result<bool, TransportError> {
    let parse = |u: &str| {
        reqwest::Url::parse(u).map_err(|e| TransportError::Io {
            detail: e.to_string(),
        })
    };
    let (a, b) = (parse(base)?, parse(next)?);
    Ok(
        a.host_str().map(str::to_ascii_lowercase) == b.host_str().map(str::to_ascii_lowercase)
            && a.port_or_known_default() == b.port_or_known_default(),
    )
}

/// The headers the next hop may carry.
///
/// **A credential is stripped the moment the host changes.** Without this a `302` to any host
/// receives the user's forge token, and the no-injection gate cannot see it because that gate
/// reads the client's *construction site* rather than what a later hop is handed.
///
/// # Errors
/// Propagates a URL that cannot be parsed.
pub fn headers_for_hop(
    headers: &[(String, String)],
    base: &str,
    next: &str,
) -> Result<Vec<(String, String)>, TransportError> {
    if same_host(base, next)? {
        return Ok(headers.to_vec());
    }
    Ok(headers
        .iter()
        .filter(|(name, _)| {
            !CREDENTIAL_HEADERS
                .iter()
                .any(|secret| name.eq_ignore_ascii_case(secret))
        })
        .cloned()
        .collect())
}

/// The redirect loop, over a caller-supplied hop.
///
/// It is a free function taking the hop as a closure so the loop itself is testable without a
/// socket: `ReqwestTransport` supplies the real hop, and a test supplies one that answers a
/// cross-host `302` and records what the second hop was handed.
///
/// # Errors
/// The hop's own failure, a plaintext or scheme-changing redirect, an unparseable location, or
/// more redirects than `redirect_limit` allows.
pub fn follow_redirects<F>(req: &HttpRequest, mut hop: F) -> Result<HttpResponse, TransportError>
where
    F: FnMut(
        &str,
        &str,
        &[(String, String)],
        Option<&Vec<u8>>,
    ) -> Result<HttpResponse, TransportError>,
{
    let mut method: &str = req.method;
    let mut url = req.url.clone();
    let mut headers = req.headers.clone();
    let mut body = req.body.clone();
    let mut hops = 0_u8;

    loop {
        require_https(&url)?;
        let response = hop(method, &url, &headers, body.as_ref())?;

        let Some(keeps_method) = redirect_keeps_method(response.status) else {
            return Ok(response);
        };
        // A redirect status with no `location` is not a redirect; it is a response.
        let Some(location) = response.header("location").map(str::to_owned) else {
            return Ok(response);
        };
        if hops >= req.limits.redirect_limit {
            return Err(TransportError::Io {
                detail: format!("more than {} redirects", req.limits.redirect_limit),
            });
        }

        let base = reqwest::Url::parse(&url).map_err(|e| TransportError::Io {
            detail: e.to_string(),
        })?;
        let next = base.join(&location).map_err(|e| TransportError::Io {
            detail: e.to_string(),
        })?;
        if next.scheme() != base.scheme() && !req.limits.allow_scheme_change {
            return Err(TransportError::Connect {
                detail: "a redirect changed scheme".to_owned(),
            });
        }
        let next = next.to_string();
        headers = headers_for_hop(&headers, &url, &next)?;
        if !keeps_method {
            method = "GET";
            body = None;
        }
        url = next;
        hops += 1;
    }
}

/// The connect timeout, and the one bound this transport cannot take per request.
///
/// `reqwest::blocking::RequestBuilder` has no `connect_timeout` — the setting lives on
/// `ClientBuilder`, and there is exactly one client in the product. So the client is built with
/// this value and [`ReqwestTransport::send`] **refuses** a request that asks for a different one,
/// naming it. A silent substitution would hand a later consumer a bound it did not choose.
pub const CONNECT_TIMEOUT_SECS: u64 = ACCOUNT_LIMITS.connect_secs;

/// The one production implementation, holding **the** `reqwest::blocking::Client`.
///
/// One client, because each owns its own runtime thread and connection pool: a second is both a
/// leak and a second budget's worth of sockets.
#[derive(Debug, Clone)]
pub struct ReqwestTransport {
    client: reqwest::blocking::Client,
}

impl ReqwestTransport {
    /// Builds the client.
    ///
    /// **No `default_headers` map is passed**, so nothing is added to a request that the caller
    /// did not put there. There is no cookie jar either, and that is enforced one level below a
    /// `.cookie_store(false)` call: `reqwest`'s `cookies` feature is not enabled in
    /// `core/Cargo.toml`, so the jar is not compiled in and no builder method exists to attach
    /// one. `core/tests/http_transport.rs` asserts the feature's absence.
    ///
    /// `Policy::none()`: redirects are followed by [`ReqwestTransport::send`] itself, so
    /// `redirect_limit` and `allow_scheme_change` are honoured **per request**. A client-level
    /// policy would apply one consumer's limit to every other's.
    pub fn new() -> Result<Self, TransportError> {
        let client = reqwest::blocking::Client::builder()
            .connect_timeout(Duration::from_secs(CONNECT_TIMEOUT_SECS))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| TransportError::Io {
                detail: e.to_string(),
            })?;
        Ok(Self { client })
    }

    /// One hop. Returns the response with its headers intact, whatever the status.
    fn hop(
        &self,
        method: reqwest::Method,
        url: &str,
        headers: &[(String, String)],
        body: Option<&Vec<u8>>,
        limits: RequestLimits,
    ) -> Result<HttpResponse, TransportError> {
        let mut builder = self
            .client
            .request(method, url)
            .timeout(Duration::from_secs(limits.total_secs));
        for (name, value) in headers {
            builder = builder.header(name, value);
        }
        if let Some(body) = body {
            builder = builder.body(body.clone());
        }

        // No `error_for_status`: a non-2xx is a response, and its headers are the input the rate
        // budget reads. Returning it as an error would drop exactly those headers.
        let response = builder.send().map_err(|e| classify(&e))?;
        let status = response.status().as_u16();
        let headers = normalise_headers(
            response
                .headers()
                .iter()
                .filter_map(|(k, v)| v.to_str().ok().map(|value| (k.as_str(), value))),
        );
        let body = read_capped(response, limits.max_body_bytes)?;
        Ok(HttpResponse {
            status,
            headers,
            body,
        })
    }
}

/// Whether a status redirects, and whether the next hop keeps the method and body.
///
/// 303 and the two legacy codes become a bodyless GET; 307 and 308 exist precisely to preserve
/// the method, and collapsing them would turn a POST into a GET without saying so.
#[must_use]
pub fn redirect_keeps_method(status: u16) -> Option<bool> {
    match status {
        301..=303 => Some(false),
        307 | 308 => Some(true),
        _ => None,
    }
}

impl HttpTransport for ReqwestTransport {
    fn send(&self, req: &HttpRequest) -> Result<HttpResponse, TransportError> {
        if req.limits.connect_secs != CONNECT_TIMEOUT_SECS {
            return Err(TransportError::Io {
                detail: format!(
                    "connect_secs is a client-level bound in this transport: the client is built \
                     with {CONNECT_TIMEOUT_SECS} and this request asks for {}",
                    req.limits.connect_secs
                ),
            });
        }
        follow_redirects(req, |method, url, headers, body| {
            let method =
                reqwest::Method::from_bytes(method.as_bytes()).map_err(|e| TransportError::Io {
                    detail: e.to_string(),
                })?;
            self.hop(method, url, headers, body, req.limits)
        })
    }
}

fn classify(error: &reqwest::Error) -> TransportError {
    if error.is_timeout() {
        return TransportError::Timeout;
    }
    if error.is_connect() {
        return TransportError::Connect {
            detail: error.to_string(),
        };
    }
    TransportError::Io {
        detail: error.to_string(),
    }
}

/// The transport a core with no working HTTP client holds.
///
/// It exists so the composition root can always hand `CoreDeps` a real `HttpTransport`: a core
/// that cannot build a client must still start, because the product is fully functional with
/// zero accounts and every remote value is *unknown* until one exists. Every call refuses by
/// name, which is a statement about the machine rather than a silent absence of network.
#[derive(Debug, Clone, Copy)]
pub struct RefusingTransport;

impl HttpTransport for RefusingTransport {
    fn send(&self, _req: &HttpRequest) -> Result<HttpResponse, TransportError> {
        Err(TransportError::Connect {
            detail: "no HTTP transport could be built on this machine".to_owned(),
        })
    }
}
