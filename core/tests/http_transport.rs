//! The one HTTP client (R66): what it surfaces, what it must never inject, and the two gates.
//!
//! Both gates walk `core/src/` and **print the number of files they scanned**. A gate whose
//! passing run scanned zero files is a failing gate, and this repository has shipped one.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use codotheca_core::http::{
    normalise_headers, read_capped, redirect_keeps_method, require_https, HttpRequest,
    HttpResponse, HttpTransport, RequestLimits, ReqwestTransport, TransportError, ACCOUNT_LIMITS,
    CONNECT_TIMEOUT_SECS,
};
use codotheca_core::testing::FakeTransport;

fn core_src() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

/// Every `.rs` under `core/src/`, with its text. Panics on an empty walk: the assertions below
/// are all of the form "no file contains X", which an empty set satisfies vacuously.
fn rust_sources() -> Vec<(std::path::PathBuf, String)> {
    fn walk(dir: &std::path::Path, out: &mut Vec<(std::path::PathBuf, String)>) {
        for entry in std::fs::read_dir(dir).expect("core/src is readable") {
            let entry = entry.expect("a readable entry");
            // The entry kind comes from readdir, never a second stat: a file that vanishes
            // between the two takes the gate down instead of being skipped.
            let kind = entry.file_type().expect("an entry kind");
            let path = entry.path();
            if kind.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                match std::fs::read_to_string(&path) {
                    Ok(text) => out.push((path, text)),
                    // Skipped BEFORE it is counted, so "scanned nothing" keeps meaning what it
                    // says while another gate's probe file races this walk.
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => panic!("{}: {e}", path.display()),
                }
            }
        }
    }
    let mut out = Vec::new();
    walk(&core_src(), &mut out);
    assert!(
        !out.is_empty(),
        "the walk read no file, so it proved nothing"
    );
    out
}

/// A path relative to `core/src/`, **always with `/` separators**.
///
/// Windows renders the same path as `http\\mod.rs`, and a literal written `http/mod.rs` then
/// fails a comparison that passes in WSL. That defect class — a POSIX path literal compared
/// against a native path — has now fired three times in this repository, and it is reachable by
/// no gate that runs only on one platform.
fn relative(path: &std::path::Path) -> String {
    path.strip_prefix(core_src())
        .unwrap_or(path)
        .display()
        .to_string()
        .replace('\\', "/")
}

// ---------------------------------------------------------------------------
// Gate 1: exactly one client
// ---------------------------------------------------------------------------

/// Each `reqwest::blocking::Client` owns its own runtime thread and connection pool, so a second
/// one is both a leak and a second budget's worth of sockets against the same forge.
///
/// The needles are the **unqualified** ones `provider_seam.rs` already uses. Matching only
/// `blocking::Client::…` reads the path spelling rather than the call: `use reqwest::blocking::
/// Client;` followed by `Client::builder()` is the same client and matched neither. Every
/// qualified spelling ends in one of these, so the narrower pair was strictly weaker.
///
/// Occurrences are counted, not files: two constructions in one file are two sites. Comment
/// lines are stripped first, because this file's own neighbours document the rule and a gate
/// that greps the prose about a rule reports the documentation.
#[test]
fn exactly_one_reqwest_client_is_constructed_in_the_core() {
    let sources = rust_sources();
    let mut sites: Vec<String> = Vec::new();
    for (path, text) in &sources {
        let code = text
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        for needle in ["Client::builder", "Client::new"] {
            for _ in code.matches(needle) {
                sites.push(format!("{}: {needle}", relative(path)));
            }
        }
    }
    eprintln!(
        "http_transport: one-client gate scanned {} files, found {} construction site(s)",
        sources.len(),
        sites.len()
    );
    assert_eq!(sites.len(), 1, "one client, and only one: {sites:?}");
    assert!(sites[0].starts_with("http/mod.rs"), "{:?}", sites[0]);
}

// ---------------------------------------------------------------------------
// Gate 2: no async of ours
// ---------------------------------------------------------------------------

/// `reqwest`'s blocking client starts a background tokio runtime, so tokio is in the tree. What
/// is not in the tree is any async code of **ours**, which is the only part of "the core is
/// blocking threads" that is enforceable.
#[test]
fn the_core_contains_no_async_fn_and_no_await() {
    let sources = rust_sources();
    let mut offenders: Vec<String> = Vec::new();
    for (path, text) in &sources {
        for (line_no, line) in text.lines().enumerate() {
            let code = line.trim_start();
            if code.starts_with("//") || code.starts_with("//!") {
                continue;
            }
            if code.contains("async fn ") || code.contains(".await") {
                offenders.push(format!("{}:{}", relative(path), line_no + 1));
            }
        }
    }
    eprintln!(
        "http_transport: no-async gate scanned {} files",
        sources.len()
    );
    assert!(
        offenders.is_empty(),
        "the core is blocking threads: {offenders:?}"
    );
}

// ---------------------------------------------------------------------------
// The no-injection rule, in the two forms it takes
// ---------------------------------------------------------------------------

/// (a) The contract's shape: a request built with an empty header list arrives with one.
#[test]
fn a_request_with_no_headers_reaches_the_transport_with_none() {
    let transport = FakeTransport::new();
    transport.push(HttpResponse {
        status: 200,
        headers: Vec::new(),
        body: Vec::new(),
    });
    let request = HttpRequest {
        method: "GET",
        url: "https://forge.example.invalid/api".to_owned(),
        headers: Vec::new(),
        body: None,
        limits: ACCOUNT_LIMITS,
    };
    transport.send(&request).unwrap();

    let sent = transport.requests();
    assert_eq!(sent.len(), 1);
    assert!(
        sent[0].headers.is_empty(),
        "the transport added a header nobody asked for: {:?}",
        sent[0].headers
    );
    assert!(sent[0]
        .headers
        .iter()
        .all(|(k, _)| !k.eq_ignore_ascii_case("authorization")));
}

/// (b) The one that catches a wave-1 injection, because it reads the **construction site**
/// rather than a request that never went through it.
///
/// There is no cookie jar to switch off: `reqwest`'s `cookies` feature is not enabled, so the
/// jar is not compiled in and `ClientBuilder::cookie_store` does not exist. That is a stronger
/// statement than a `.cookie_store(false)` call, and this asserts the feature list that makes it
/// true as well as the absence of `default_headers`.
#[test]
fn the_client_builder_injects_nothing() {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let toml = std::fs::read_to_string(&manifest).expect("Cargo.toml is readable");
    let source = std::fs::read_to_string(core_src().join("http/mod.rs")).expect("http/mod.rs");
    assert!(!toml.is_empty() && !source.is_empty());
    // Comments stripped before the scan. Grepping a name also matches the prose *about* it, and
    // this module's own documentation names both of the calls it forbids — the first version of
    // this test failed on its own doc comment.
    let code = strip_comments(&source);
    assert!(
        code.len() < source.len(),
        "nothing was stripped, so the scan is still reading prose"
    );
    eprintln!(
        "http_transport: read {} bytes of Cargo.toml and {} bytes of http/mod.rs ({} of code)",
        toml.len(),
        source.len(),
        code.len()
    );

    let reqwest_entry = toml
        .split("reqwest = ")
        .nth(1)
        .expect("core/Cargo.toml declares reqwest");
    let features = reqwest_entry
        .split(']')
        .next()
        .expect("the feature list is bracketed");
    assert!(
        !features.contains("cookies"),
        "the cookie jar is compiled in, so a builder can attach one: {features}"
    );

    assert!(
        !code.contains("default_headers"),
        "the client sets default headers, which reach every consumer's requests"
    );
    assert!(
        !code.contains("error_for_status"),
        "a non-2xx must stay Ok: error_for_status drops the headers the rate budget reads"
    );
}

/// Source with every `//`-comment line removed. Nothing cleverer: the files this reads carry no
/// block comments, and a half-correct comment parser would be a second thing to get wrong.
fn strip_comments(source: &str) -> String {
    source
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

// ---------------------------------------------------------------------------
// What the transport surfaces
// ---------------------------------------------------------------------------

/// §21.6 and §21.8: a `403` carrying rate-limit headers is a *response*. Returning it as an
/// error with the headers dropped destroys the only input the classification has.
#[test]
fn a_403_carrying_rate_limit_headers_is_ok_and_keeps_all_four() {
    let transport = FakeTransport::new();
    transport.push(HttpResponse {
        status: 403,
        headers: normalise_headers([
            ("X-RateLimit-Remaining", "0"),
            ("X-RateLimit-Reset", "1750000000"),
            ("X-RateLimit-Resource", "core"),
            ("Date", "Mon, 01 Jan 2035 00:00:00 GMT"),
        ]),
        body: b"{}".to_vec(),
    });
    let response = transport
        .send(&HttpRequest {
            method: "GET",
            url: "https://forge.example.invalid/api".to_owned(),
            headers: Vec::new(),
            body: None,
            limits: ACCOUNT_LIMITS,
        })
        .expect("a non-2xx is a response, not a transport error");

    assert_eq!(response.status, 403);
    assert_eq!(response.header("x-ratelimit-remaining"), Some("0"));
    assert_eq!(response.header("X-RateLimit-Reset"), Some("1750000000"));
    assert_eq!(response.header("x-ratelimit-resource"), Some("core"));
    assert!(response.header("date").is_some());
    // Lowercased on the way in, so one spelling reaches every consumer.
    assert!(response
        .headers
        .iter()
        .all(|(k, _)| k.chars().all(|c| !c.is_ascii_uppercase())));
}

#[test]
fn a_doubled_header_keeps_the_first_value_and_stays_countable() {
    let response = HttpResponse {
        status: 429,
        headers: normalise_headers([("Retry-After", "30"), ("Retry-After", "120")]),
        body: Vec::new(),
    };
    assert_eq!(response.header("retry-after"), Some("30"));
    assert_eq!(
        response.header_count("Retry-After"),
        2,
        "a doubled header must stay visible rather than being silently resolved"
    );
}

#[test]
fn a_transport_error_is_the_only_thing_that_is_not_a_response() {
    let transport = FakeTransport::new();
    transport.push_err(TransportError::Timeout);
    let outcome = transport.send(&HttpRequest {
        method: "GET",
        url: "https://forge.example.invalid/api".to_owned(),
        headers: Vec::new(),
        body: None,
        limits: ACCOUNT_LIMITS,
    });
    assert_eq!(outcome.unwrap_err(), TransportError::Timeout);
}

// ---------------------------------------------------------------------------
// The bounds
// ---------------------------------------------------------------------------

/// A counting reader, so the assertion is about what was **read** rather than about the length
/// that came back. A cap applied after reading the whole body is not a cap.
#[derive(Debug)]
struct CountingReader {
    remaining: usize,
    read: std::rc::Rc<std::cell::Cell<usize>>,
}

impl std::io::Read for CountingReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = buf.len().min(self.remaining);
        for slot in &mut buf[..n] {
            *slot = b'x';
        }
        self.remaining -= n;
        self.read.set(self.read.get() + n);
        Ok(n)
    }
}

#[test]
fn a_body_over_the_cap_is_refused_without_reading_it() {
    let counted = std::rc::Rc::new(std::cell::Cell::new(0_usize));
    let body_len = 1_000_000_usize;
    let cap = 1_024_usize;
    let reader = CountingReader {
        remaining: body_len,
        read: std::rc::Rc::clone(&counted),
    };

    let outcome = read_capped(reader, cap);
    assert!(matches!(outcome, Err(TransportError::Io { .. })));
    eprintln!(
        "http_transport: read {} bytes before refusing a {body_len}-byte body capped at {cap}",
        counted.get()
    );
    assert!(
        counted.get() < body_len,
        "the cap read the whole body before refusing it"
    );
    assert!(
        counted.get() <= cap + 1,
        "the cap read {} bytes, more than one byte past the limit",
        counted.get()
    );
}

#[test]
fn a_body_at_the_cap_is_returned_whole() {
    let counted = std::rc::Rc::new(std::cell::Cell::new(0_usize));
    let reader = CountingReader {
        remaining: 1_024,
        read: std::rc::Rc::clone(&counted),
    };
    assert_eq!(read_capped(reader, 1_024).unwrap().len(), 1_024);
}

#[test]
fn only_https_reaches_a_socket() {
    require_https("https://forge.example.invalid/api").unwrap();
    for refused in [
        "http://forge.example.invalid/api",
        "ftp://forge.example.invalid",
        "//forge.example.invalid",
    ] {
        assert!(
            matches!(require_https(refused), Err(TransportError::Connect { .. })),
            "{refused} was not refused"
        );
    }
}

/// The production transport refuses a plaintext URL before it opens a socket, so the rule is a
/// property of `send` and not only of the helper.
#[test]
fn the_production_transport_refuses_a_plaintext_url() {
    let transport = ReqwestTransport::new().expect("the client builds");
    let outcome = transport.send(&HttpRequest {
        method: "GET",
        url: "http://forge.example.invalid/api".to_owned(),
        headers: Vec::new(),
        body: None,
        limits: ACCOUNT_LIMITS,
    });
    assert!(matches!(outcome, Err(TransportError::Connect { .. })));
}

/// `connect_secs` is a client-level bound in `reqwest::blocking`, so a request asking for a
/// different one is **refused by name** rather than silently given the client's.
#[test]
fn a_request_asking_for_another_connect_timeout_is_refused_by_name() {
    let transport = ReqwestTransport::new().expect("the client builds");
    let limits = RequestLimits {
        connect_secs: CONNECT_TIMEOUT_SECS + 1,
        ..ACCOUNT_LIMITS
    };
    let error = transport
        .send(&HttpRequest {
            method: "GET",
            url: "https://forge.example.invalid/api".to_owned(),
            headers: Vec::new(),
            body: None,
            limits,
        })
        .expect_err("a bound the transport cannot honour must be named, not substituted");
    let TransportError::Io { detail } = error else {
        panic!("expected an Io refusal naming the bound");
    };
    assert!(detail.contains("connect_secs"), "{detail}");
}

#[test]
fn the_account_limits_are_the_section_s_and_no_redirect_may_change_scheme() {
    assert_eq!(ACCOUNT_LIMITS.connect_secs, 10);
    assert_eq!(ACCOUNT_LIMITS.total_secs, 30);
    // Read through a binding: clippy refuses both `assert!` and `assert_eq!` on a constant, and
    // a `#[allow]` here would be the third way of writing one assertion.
    let limits = ACCOUNT_LIMITS;
    assert!(
        !limits.allow_scheme_change,
        "an account call may never be redirected off https"
    );
    assert_eq!(CONNECT_TIMEOUT_SECS, ACCOUNT_LIMITS.connect_secs);
}

/// 307 and 308 exist to preserve the method. Collapsing them into 301's behaviour would turn a
/// POST into a GET without saying so.
#[test]
fn a_temporary_redirect_keeps_the_method_and_a_see_other_does_not() {
    assert_eq!(redirect_keeps_method(301), Some(false));
    assert_eq!(redirect_keeps_method(302), Some(false));
    assert_eq!(redirect_keeps_method(303), Some(false));
    assert_eq!(redirect_keeps_method(307), Some(true));
    assert_eq!(redirect_keeps_method(308), Some(true));
    for not_a_redirect in [200, 304, 403, 404, 500] {
        assert_eq!(redirect_keeps_method(not_a_redirect), None);
    }
}

// ---------------------------------------------------------------------------
// A credential never crosses a host boundary.
// ---------------------------------------------------------------------------

/// One hop as it reached the wire.
#[derive(Debug, Clone)]
struct SeenHop {
    url: String,
    headers: Vec<(String, String)>,
}

/// Records what every hop was handed, and answers a scripted status per hop.
#[derive(Debug, Default)]
struct HopRecorder {
    seen: std::cell::RefCell<Vec<SeenHop>>,
}

impl HopRecorder {
    fn answer(
        &self,
        url: &str,
        headers: &[(String, String)],
        location: Option<&str>,
    ) -> HttpResponse {
        self.seen.borrow_mut().push(SeenHop {
            url: url.to_owned(),
            headers: headers.to_vec(),
        });
        match location {
            Some(next) => HttpResponse {
                status: 302,
                headers: normalise_headers([("Location", next)]),
                body: Vec::new(),
            },
            None => HttpResponse {
                status: 200,
                headers: Vec::new(),
                body: b"{}".to_vec(),
            },
        }
    }

    fn headers_of(&self, hop: usize) -> Vec<(String, String)> {
        self.seen.borrow()[hop].headers.clone()
    }

    fn url_of(&self, hop: usize) -> String {
        self.seen.borrow()[hop].url.clone()
    }
}

fn bearer_request(url: &str) -> HttpRequest {
    HttpRequest {
        method: "GET",
        url: url.to_owned(),
        headers: normalise_headers([
            ("Authorization", "Bearer SENTINEL-TOKEN-0000"),
            ("Cookie", "session=SENTINEL-TOKEN-0000"),
            // All three of `CREDENTIAL_HEADERS`, not two: a header named by the constant and by
            // no fixture is a header the strip is never observed to remove.
            ("Proxy-Authorization", "Basic SENTINEL-TOKEN-0000"),
            ("Accept", "application/vnd.github+json"),
        ]),
        body: None,
        limits: ACCOUNT_LIMITS,
    }
}

/// **A `302` to another host must not receive the caller's token.**
///
/// `reqwest`'s own redirect policy strips `Authorization` and `Cookie` cross-origin; this
/// transport sets `Policy::none()` so that `redirect_limit` and `allow_scheme_change` stay
/// per-request, which opts out of that stripping and makes it this loop's job.
#[test]
fn a_cross_host_redirect_receives_no_credential() {
    let recorder = HopRecorder::default();
    let response = codotheca_core::http::follow_redirects(
        &bearer_request("https://api.forge.example.invalid/user"),
        |_method, url, headers, _body| {
            let location = if url.starts_with("https://api.forge.example.invalid") {
                Some("https://evil.example.invalid/collect")
            } else {
                None
            };
            Ok(recorder.answer(url, headers, location))
        },
    )
    .expect("the redirect is followed");
    assert_eq!(response.status, 200);

    assert_eq!(recorder.url_of(0), "https://api.forge.example.invalid/user");
    assert_eq!(recorder.url_of(1), "https://evil.example.invalid/collect");
    // Non-vacuity, and over **every** named header: if the first hop did not carry all three,
    // their absence on the second would prove nothing about the strip.
    let first = recorder.headers_of(0);
    for name in codotheca_core::http::CREDENTIAL_HEADERS {
        assert!(
            first.iter().any(|(k, _)| k == name),
            "the first hop, on the original host, must still carry {name}"
        );
    }

    let second = recorder.headers_of(1);
    let leaked: Vec<&(String, String)> = second
        .iter()
        .filter(|(k, v)| {
            v.contains("SENTINEL-TOKEN-0000")
                || codotheca_core::http::CREDENTIAL_HEADERS.contains(&k.as_str())
        })
        .collect();
    assert!(
        leaked.is_empty(),
        "a credential reached another host across a redirect: {leaked:?}"
    );
    // The non-credential header survives: this strips secrets, not everything.
    assert!(second.iter().any(|(k, _)| k == "accept"));
}

/// Same host, so nothing is stripped — otherwise the fix would break paginated reads that
/// legitimately redirect within the forge.
#[test]
fn a_same_host_redirect_keeps_the_credential() {
    let recorder = HopRecorder::default();
    codotheca_core::http::follow_redirects(
        &bearer_request("https://api.forge.example.invalid/user"),
        |_method, url, headers, _body| {
            let location = if url.ends_with("/user") {
                Some("https://api.forge.example.invalid/user/moved")
            } else {
                None
            };
            Ok(recorder.answer(url, headers, location))
        },
    )
    .expect("the redirect is followed");
    assert!(recorder
        .headers_of(1)
        .iter()
        .any(|(k, _)| k == "authorization"));
}

#[test]
fn the_host_comparison_ignores_case_and_reads_the_port() {
    use codotheca_core::http::{headers_for_hop, same_host};
    assert!(same_host(
        "https://API.Forge.Example.Invalid/a",
        "https://api.forge.example.invalid/b"
    )
    .unwrap());
    assert!(same_host(
        "https://a.example.invalid/x",
        "https://a.example.invalid:443/y"
    )
    .unwrap());
    assert!(!same_host(
        "https://a.example.invalid/x",
        "https://a.example.invalid:8443/y"
    )
    .unwrap());
    assert!(!same_host("https://a.example.invalid/x", "https://b.example.invalid/y").unwrap());

    let headers = normalise_headers([("Authorization", "Bearer x"), ("Accept", "j")]);
    let kept = headers_for_hop(
        &headers,
        "https://a.example.invalid",
        "https://a.example.invalid/2",
    )
    .unwrap();
    assert_eq!(kept.len(), 2);
    let stripped = headers_for_hop(
        &headers,
        "https://a.example.invalid",
        "https://b.example.invalid",
    )
    .unwrap();
    assert_eq!(stripped.len(), 1);
    assert_eq!(stripped[0].0, "accept");
}
