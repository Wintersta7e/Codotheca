#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! §32.4's fourth widening: **the first unauthenticated forge read in the product.**
//!
//! Every other method on the provider seam takes `t: &SecretToken` **non-optionally**. This one is
//! unauthenticated by ruling, so its signature carries **no token parameter at all** — not
//! `Option<&SecretToken>`, which would leave a later author a place to put one. The refusal is
//! structural, not conditional, and the scan below is what keeps it so.
//!
//! **It returns `ProviderResult<Observed<_>>` or it slips past the census.** `AC-P2-25-5`
//! classifies a request method by exactly that return shape, so a request method returning
//! anything else would pass the census unseen.

use std::sync::Arc;

use codotheca_core::http::{HttpResponse, HttpTransport};
use codotheca_core::protocol::Ecosystem;
use codotheca_core::provider::{
    GitHubProvider, PackageVersion, Provider, PROVIDER_REQUEST_METHODS,
};
use codotheca_core::testing::FakeTransport;

/// The trait's own source, compiled in. Reading the file at runtime would depend on the working
/// directory a test runner happens to use.
const PROVIDER_SOURCE: &str = include_str!("../src/provider/mod.rs");

fn body(json: &str) -> HttpResponse {
    HttpResponse {
        status: 200,
        headers: codotheca_core::http::normalise_headers([("x-ratelimit-resource", "core")]),
        body: json.as_bytes().to_vec(),
    }
}

/// One advisory, as the endpoint actually shapes it — measured against the live response on
/// 2026-09-19, not invented.
const ONE_ADVISORY: &str = r#"[
  {
    "ghsa_id": "GHSA-aaaa-bbbb-cccc",
    "cve_id": "CVE-2026-0001",
    "summary": "a package is vulnerable to something",
    "html_url": "https://forge.example.invalid/advisories/GHSA-aaaa-bbbb-cccc",
    "severity": "critical",
    "withdrawn_at": null,
    "identifiers": [
      { "value": "GHSA-aaaa-bbbb-cccc", "type": "GHSA" },
      { "value": "CVE-2026-0001", "type": "CVE" },
      { "value": "CVE-2026-0002", "type": "CVE" }
    ],
    "vulnerabilities": [
      {
        "package": { "ecosystem": "npm", "name": "left" },
        "vulnerable_version_range": "< 2.0.0",
        "first_patched_version": "2.0.0"
      },
      {
        "package": { "ecosystem": "npm", "name": "right" },
        "vulnerable_version_range": "< 9.9.9",
        "first_patched_version": null
      }
    ]
  }
]"#;

fn provider() -> (Arc<FakeTransport>, GitHubProvider) {
    let transport = Arc::new(FakeTransport::new());
    let provider = GitHubProvider::new(
        Arc::clone(&transport) as Arc<dyn HttpTransport>,
        "forge.example.invalid".to_owned(),
    );
    (transport, provider)
}

fn pkg(name: &str, version: &str) -> PackageVersion {
    PackageVersion {
        name: name.to_owned(),
        version: version.to_owned(),
    }
}

/// **AC-P3-32-4.** The trait's own signature, scanned: every census entry is a method on it, and
/// `advisories` takes no token of any kind while still returning the shape the census classifies
/// by.
///
/// **A scan over zero methods is a failing scan**, asserted explicitly.
#[test]
fn every_request_method_is_on_the_trait_and_the_advisory_read_takes_no_token() {
    let mut found = 0usize;
    for method in PROVIDER_REQUEST_METHODS {
        assert!(
            PROVIDER_SOURCE.contains(&format!("fn {method}(")),
            "{method} is in the census but is not a method on the trait"
        );
        found += 1;
    }
    eprintln!("advisory_provider: {found} request methods scanned on the trait");
    assert_eq!(found, PROVIDER_REQUEST_METHODS.len());
    assert!(found > 0, "a scan over zero methods proves nothing");

    let signature = PROVIDER_SOURCE
        .split("fn advisories(")
        .nth(1)
        .expect("the trait declares advisories")
        .split(");")
        .next()
        .expect("the signature ends");
    assert!(
        !signature.contains("SecretToken"),
        "the advisory read is unauthenticated by ruling and has nowhere to put a token: {signature}"
    );
    assert!(
        signature.contains("ProviderResult<Observed<"),
        "a request method returning anything else passes AC-P2-25-5's census unseen: {signature}"
    );
    assert_eq!(
        PROVIDER_REQUEST_METHODS.last(),
        Some(&"advisories"),
        "the seventh entry, in position"
    );
}

/// **AC-P3-32-5.** The **production** implementation issues a real request through the transport.
///
/// A trait declared for testability that gets its fake and never its real impl is the recorded
/// defect with five instances; each compiled, each passed against the fake, and each failed at
/// assembly. A fake provider produces no request and fails here.
#[test]
fn the_production_implementation_issues_the_request_and_parses_the_answer() {
    let (transport, provider) = provider();
    transport.push(body(ONE_ADVISORY));

    let observed = provider
        .advisories(
            Ecosystem::Npm,
            &[pkg("left", "1.0.0"), pkg("right", "3.2.1")],
            None,
        )
        .expect("the read answers");

    let requests = transport.requests();
    eprintln!(
        "advisory_provider: {} request(s) reached the transport",
        requests.len()
    );
    assert_eq!(requests.len(), 1, "a fake provider issues none of these");
    let url = &requests[0].url;
    assert!(url.contains("forge.example.invalid"), "{url}");
    assert!(url.contains("/advisories"), "{url}");
    assert!(url.contains("ecosystem=npm"), "{url}");
    assert!(
        url.contains("left%401.0.0"),
        "the asked pair travels: {url}"
    );
    assert!(
        url.contains("right%403.2.1"),
        "the asked pair travels: {url}"
    );

    // **No credential leaves this call**, whatever one is in the process.
    for (name, _) in &requests[0].headers {
        assert_ne!(name, "authorization", "the advisory read carries no token");
    }

    let page = observed.value;
    assert_eq!(page.items.len(), 1);
    let advisory = &page.items[0];
    assert_eq!(advisory.advisory_id, "GHSA-aaaa-bbbb-cccc");
    // R118: **every** CVE id, because one GHSA carries several or none.
    assert_eq!(advisory.cve_ids, vec!["CVE-2026-0001", "CVE-2026-0002"]);
    // §32.13: the source's own word, byte for byte, with no enum to refuse it.
    assert_eq!(advisory.severity.as_deref(), Some("critical"));
    assert_eq!(advisory.withdrawn_at, None);
    assert_eq!(advisory.summary, "a package is vulnerable to something");

    // **Fix availability is per affected package, not per advisory.** One advisory fixed in one
    // package and not in another must not flip both items together.
    assert_eq!(advisory.affects.len(), 2);
    assert_eq!(advisory.affects[0].name, "left");
    assert!(advisory.affects[0].fix_available);
    assert_eq!(advisory.affects[0].fixed_version.as_deref(), Some("2.0.0"));
    assert_eq!(advisory.affects[1].name, "right");
    assert!(!advisory.affects[1].fix_available);
    assert_eq!(advisory.affects[1].fixed_version, None);
}

/// **AC-P3-32-5, at assembly.** The composition root hands out the **production** provider, and a
/// request through it reaches the transport with the expected host and path.
///
/// This is the shape the recorded defect needs: a trait declared for testability whose fake is
/// wired in place of the real implementation compiles, passes every unit test, and answers nothing
/// at runtime. A fake here produces zero requests and fails on the printed count.
#[test]
fn assembly_hands_out_the_production_provider() {
    let transport = Arc::new(FakeTransport::new());
    let clock: Arc<dyn codotheca_core::clock::Clock> =
        Arc::new(codotheca_core::testing::FakeClock::new(1_800_000_000));
    let (provider, _observing) = codotheca_core::assembly::sync::build_forge(
        Arc::clone(&transport) as Arc<dyn HttpTransport>,
        clock,
        "forge.example.invalid".to_owned(),
    );
    transport.push(body(ONE_ADVISORY));

    let observed = provider
        .advisories(Ecosystem::Npm, &[pkg("left", "1.0.0")], None)
        .expect("the assembled provider answers");
    let requests = transport.requests();
    eprintln!(
        "advisory_provider: {} request(s) reached the transport from assembly",
        requests.len()
    );
    assert_eq!(requests.len(), 1, "a fake provider issues none of these");
    assert!(
        requests[0]
            .url
            .starts_with("https://forge.example.invalid/api/v3/advisories?"),
        "{}",
        requests[0].url
    );
    assert_eq!(observed.value.items.len(), 1);
}

/// A response carrying no `X-OAuth-Scopes` yields `granted_scopes: None`, which is **unknown** and
/// not an empty grant. For this method it is `None` for ever, which is correct: nothing was
/// granted to observe.
#[test]
fn an_unauthenticated_read_observes_no_grant_rather_than_an_empty_one() {
    let (transport, provider) = provider();
    transport.push(body("[]"));
    let observed = provider
        .advisories(Ecosystem::Rust, &[pkg("serde", "1.0.0")], None)
        .expect("answers");
    assert_eq!(observed.granted_scopes, None);
    assert!(observed.value.items.is_empty());
    assert_eq!(observed.value.next_cursor, None);
}

/// A severity spelling this build does not recognise is **stored as sent**. A closed mirror of a
/// third party's vocabulary is R26 by construction, already ruled for `CiRunPayload.conclusion`.
#[test]
fn an_unrecognised_severity_is_carried_verbatim() {
    let (transport, provider) = provider();
    transport.push(body(
        r#"[{"ghsa_id":"GHSA-x","summary":"s","html_url":"u","severity":"catastrophic",
            "withdrawn_at":"2026-01-02T03:04:05Z","identifiers":[],"vulnerabilities":[]}]"#,
    ));
    let observed = provider
        .advisories(Ecosystem::Pip, &[pkg("requests", "2.0.0")], None)
        .expect("answers");
    let advisory = &observed.value.items[0];
    assert_eq!(advisory.severity.as_deref(), Some("catastrophic"));
    assert_eq!(advisory.cve_ids, Vec::<String>::new());
    assert_eq!(advisory.withdrawn_at, Some(1_767_323_045));
}
