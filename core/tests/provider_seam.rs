#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use codotheca_core::accounts::keychain::SecretToken;
use codotheca_core::http::{normalise_headers, HttpResponse, HttpTransport};
use codotheca_core::provider::listing::GITHUB_CANONICAL_HOST;
use codotheca_core::provider::scopes::{SCOPES_PRIVATE, SCOPES_PUBLIC};
use codotheca_core::provider::{GitHubProvider, Provider, ProviderError, PROVIDER_REQUEST_METHODS};
use codotheca_core::testing::FakeTransport;

const fn same_str(left: &str, right: &str) -> bool {
    let left = left.as_bytes();
    let right = right.as_bytes();
    if left.len() != right.len() {
        return false;
    }
    let mut index = 0;
    while index < left.len() {
        if left[index] != right[index] {
            return false;
        }
        index += 1;
    }
    true
}

const _: () = assert!(same_str(PROVIDER_REQUEST_METHODS[0], "viewer"));
const _: () = assert!(same_str(PROVIDER_REQUEST_METHODS[1], "list_orgs"));
const _: () = assert!(same_str(PROVIDER_REQUEST_METHODS[2], "list_repos"));
const _: () = assert!(same_str(PROVIDER_REQUEST_METHODS[3], "lookup_repo"));

fn token() -> SecretToken {
    SecretToken::new("provider-seam-token".to_owned())
}

fn ok(body: &[u8]) -> HttpResponse {
    HttpResponse {
        status: 200,
        headers: Vec::new(),
        body: body.to_vec(),
    }
}

fn ok_with_headers(headers: Vec<(String, String)>, body: &[u8]) -> HttpResponse {
    HttpResponse {
        status: 200,
        headers,
        body: body.to_vec(),
    }
}

fn user_body() -> &'static [u8] {
    br#"{"login":"fixture-login","name":"Fixture Login"}"#
}

/// One repository object, as §22.7's lookup reads it back.
fn repo_body() -> &'static [u8] {
    br#"{"id":909,"clone_url":"https://github.com/acme/widget.git",
         "owner":{"login":"acme","type":"User"},"name":"widget",
         "fork":false,"archived":false,"private":false}"#
}

fn provider_with_transport(host: &str) -> (Arc<FakeTransport>, Arc<dyn Provider>) {
    let transport = Arc::new(FakeTransport::new());
    let http_transport: Arc<dyn HttpTransport> = transport.clone();
    let provider: Arc<dyn Provider> =
        Arc::new(GitHubProvider::new(http_transport, host.to_owned()));
    (transport, provider)
}

fn provider_sources_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src/provider")
}

/// Always `/`-separated, so a reported name reads the same on both platforms.
fn relative_to_provider(path: &Path) -> String {
    path.strip_prefix(provider_sources_dir())
        .unwrap_or(path)
        .display()
        .to_string()
        .replace('\\', "/")
}

fn rust_provider_sources() -> Vec<(PathBuf, String)> {
    fn walk(dir: &Path, out: &mut Vec<(PathBuf, String)>) {
        for entry in std::fs::read_dir(dir).expect("provider dir is readable") {
            let entry = entry.expect("a readable provider entry");
            let kind = entry.file_type().expect("a provider entry kind");
            let path = entry.path();
            if kind.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                match std::fs::read_to_string(&path) {
                    Ok(text) => out.push((path, text)),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => panic!("{}: {error}", path.display()),
                }
            }
        }
    }

    let mut out = Vec::new();
    walk(&provider_sources_dir(), &mut out);
    eprintln!(
        "provider_seam: bypass gate scanned {} provider source file(s)",
        out.len()
    );
    assert!(
        !out.is_empty(),
        "the provider source walk read no files, so it proved nothing"
    );
    out
}

fn strip_comment_lines(source: &str) -> String {
    source
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn provider_trait_source() -> String {
    let source_path = provider_sources_dir().join("mod.rs");
    std::fs::read_to_string(&source_path).expect("provider mod source is readable")
}

/// The request-shaped methods the trait actually declares, classified by **signature** and not
/// by position.
///
/// An earlier version stopped scanning at `fn canonical_host`, on the assumption that helpers
/// come last. A fifth request method declared *after* the helpers then slipped past the whole
/// tripwire with the suite green — a bar written past its defect, in the one seam four rulings
/// have already been spent on. The trait body is now read whole, and a method is a request
/// method because of the shape of what it returns.
fn provider_trait_request_method_names() -> Vec<String> {
    let code = strip_comment_lines(&provider_trait_source()).replace('\n', " ");
    let trait_body = code
        .split("pub trait Provider")
        .nth(1)
        .expect("provider trait is declared");
    // Newlines are already spaces, so the closing brace at the start of its own line reads as
    // `" }"` — and a `"\n}"` needle can never match here, which is what the previous pair of
    // alternatives quietly relied on. Missing is a failure, not a reason to scan to end of file:
    // running past the trait would collect methods from whatever is declared below it.
    let end = trait_body
        .find(" }")
        .expect("the Provider trait body has a closing brace");
    let trait_body = &trait_body[..end];
    let mut names = Vec::new();
    for segment in trait_body.split("fn ").skip(1) {
        let signature = segment
            .split(';')
            .next()
            .expect("provider trait method has a semicolon");
        if signature.contains("ProviderResult<Observed<") {
            let name = signature
                .split('(')
                .next()
                .expect("provider trait method has a name")
                .trim();
            names.push(name.to_owned());
        }
    }
    eprintln!(
        "provider_seam: trait request signature scan found {} method(s): {:?}",
        names.len(),
        names
    );
    assert!(
        !names.is_empty(),
        "the provider request signature scan read no methods"
    );
    names
}

fn assert_one_request_for<F>(call: F)
where
    F: FnOnce(Arc<dyn Provider>, &SecretToken),
{
    let (transport, provider) = provider_with_transport(GITHUB_CANONICAL_HOST);
    transport.push(ok(user_body()));
    call(provider, &token());
    assert_eq!(transport.request_count(), 1);
}

#[test]
fn request_method_tripwire_is_enumerated_and_callable() {
    // R79 collapsed `verify_token` into `viewer`, leaving three; §22.7's `lookup_repo` is the
    // fourth. The count moves in the same commit as the method, or the tripwire fails — which is
    // the tripwire working. **Six by the end of phase 2**, the last two §25's.
    assert_eq!(
        PROVIDER_REQUEST_METHODS,
        ["viewer", "list_orgs", "list_repos", "lookup_repo"]
    );

    let (transport, provider) = provider_with_transport(GITHUB_CANONICAL_HOST);
    transport.push(ok(user_body()));
    transport.push(ok(b"[]"));
    transport.push(ok(b"[]"));
    transport.push(ok(repo_body()));
    let secret = token();

    provider.viewer(&secret).unwrap();
    provider.list_orgs(&secret, None).unwrap();
    provider.list_repos(&secret, None).unwrap();
    provider.lookup_repo(&secret, "acme", "widget").unwrap();
    let declared = provider_trait_request_method_names();
    assert_eq!(
        transport.request_count(),
        declared.len(),
        "the enumerated calls did not cover every request-shaped Provider method"
    );
    assert_eq!(
        declared,
        PROVIDER_REQUEST_METHODS
            .iter()
            .map(|name| (*name).to_owned())
            .collect::<Vec<_>>()
    );
}

#[test]
fn provider_is_object_safe() {
    let (_transport, provider) = provider_with_transport(GITHUB_CANONICAL_HOST);
    assert_eq!(provider.canonical_host(), GITHUB_CANONICAL_HOST);
}

#[test]
fn helpers_issue_no_requests_and_request_methods_issue_one_each() {
    let (transport, provider) = provider_with_transport(GITHUB_CANONICAL_HOST);
    assert_eq!(provider.canonical_host(), GITHUB_CANONICAL_HOST);
    assert!(provider.host_aliases().contains(&GITHUB_CANONICAL_HOST));
    assert_eq!(transport.request_count(), 0);

    assert_one_request_for(|provider, secret| {
        provider.viewer(secret).unwrap();
    });

    let (transport, provider) = provider_with_transport(GITHUB_CANONICAL_HOST);
    transport.push(ok(b"[]"));
    provider.list_orgs(&token(), None).unwrap();
    assert_eq!(transport.request_count(), 1);

    let (transport, provider) = provider_with_transport(GITHUB_CANONICAL_HOST);
    transport.push(ok(b"[]"));
    provider.list_repos(&token(), None).unwrap();
    assert_eq!(transport.request_count(), 1);

    let (transport, provider) = provider_with_transport(GITHUB_CANONICAL_HOST);
    transport.push(ok(repo_body()));
    provider.lookup_repo(&token(), "acme", "widget").unwrap();
    assert_eq!(transport.request_count(), 1);
    assert_eq!(
        transport.requests()[0].url,
        "https://api.github.com/repos/acme/widget"
    );
}

/// §22.7's lookup answers *not found* rather than failing: a repository that does not exist and
/// one this token cannot see are the same 404, and neither is an error the sync reports.
#[test]
fn a_lookup_that_finds_nothing_is_unknown_and_not_a_failure() {
    let (transport, provider) = provider_with_transport(GITHUB_CANONICAL_HOST);
    transport.push(HttpResponse {
        status: 404,
        headers: Vec::new(),
        body: br#"{"message":"Not Found"}"#.to_vec(),
    });
    let observed = provider
        .lookup_repo(&token(), "acme", "gone")
        .expect("a 404 is an answer, not an error");
    assert_eq!(observed.value, None);

    // A 403 is still an error the caller classifies; it is not folded into "not found".
    let (transport, provider) = provider_with_transport(GITHUB_CANONICAL_HOST);
    transport.push(HttpResponse {
        status: 403,
        headers: Vec::new(),
        body: Vec::new(),
    });
    assert!(provider.lookup_repo(&token(), "acme", "widget").is_err());
}

#[test]
fn the_production_impl_exists_beside_the_trait() {
    let source_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/provider/github.rs");
    let source = std::fs::read_to_string(&source_path).expect("github provider source is readable");
    eprintln!(
        "provider_seam: read {} byte(s) from {}",
        source.len(),
        source_path.display()
    );
    assert!(
        !source.is_empty(),
        "github provider source was empty, so the assertion proved nothing"
    );
    assert!(
        source.contains("impl Provider for GitHubProvider"),
        "the production provider impl is not beside the trait"
    );
}

#[test]
fn provider_sources_never_construct_or_name_an_http_client() {
    let mut offenders = Vec::new();
    for (path, source) in rust_provider_sources() {
        let code = strip_comment_lines(&source);
        for needle in [
            "reqwest",
            "blocking::Client::builder",
            "blocking::Client::new",
            "Client::builder",
            "Client::new",
        ] {
            if code.contains(needle) {
                offenders.push(format!("{} contains {needle}", relative_to_provider(&path)));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "provider source bypasses the injected transport: {offenders:?}"
    );
}

#[test]
fn the_viewer_call_round_trips_observed_scopes_from_the_response() {
    let (transport, provider) = provider_with_transport(GITHUB_CANONICAL_HOST);
    transport.push(ok_with_headers(
        normalise_headers([(
            "X-OAuth-Scopes",
            "read:user, user:email, some:invented:scope",
        )]),
        user_body(),
    ));

    let observed = provider.viewer(&token()).unwrap();
    let expected = vec![
        "read:user".to_owned(),
        "user:email".to_owned(),
        "some:invented:scope".to_owned(),
    ];
    // One place the grant lives, not two. `Verified` carried a second, lossy copy of this and
    // R79 removed it with the method that returned it.
    assert_eq!(observed.granted_scopes, Some(expected));
}

#[test]
fn an_absent_oauth_scope_header_stays_unknown() {
    let (transport, provider) = provider_with_transport(GITHUB_CANONICAL_HOST);
    transport.push(ok(user_body()));

    let observed = provider.viewer(&token()).unwrap();
    assert_eq!(observed.granted_scopes, None);
}

#[test]
fn http_status_errors_keep_status_and_headers() {
    let (transport, provider) = provider_with_transport(GITHUB_CANONICAL_HOST);
    transport.push(HttpResponse {
        status: 403,
        headers: normalise_headers([("X-RateLimit-Remaining", "7")]),
        body: b"{}".to_vec(),
    });

    let error = provider.viewer(&token()).unwrap_err();
    match error {
        ProviderError::Http { status, headers } => {
            assert_eq!(status, 403);
            assert_eq!(
                headers,
                vec![("x-ratelimit-remaining".to_owned(), "7".to_owned())]
            );
        }
        other => panic!("expected an HTTP error, got {other:?}"),
    }

    let (transport, provider) = provider_with_transport(GITHUB_CANONICAL_HOST);
    transport.push(HttpResponse {
        status: 401,
        headers: Vec::new(),
        body: b"{}".to_vec(),
    });
    assert!(matches!(
        provider.viewer(&token()).unwrap_err(),
        ProviderError::Http { status: 401, .. }
    ));
}

#[test]
fn list_repos_pages_by_link_header_and_preserves_listing_fields() {
    let next = "https://api.github.com/user/repos?per_page=100&page=2";
    let (transport, provider) = provider_with_transport(GITHUB_CANONICAL_HOST);
    transport.push(ok_with_headers(
        normalise_headers([(
            "Link",
            r#"<https://api.github.com/user/repos?per_page=100&page=9>; rel="last", <https://api.github.com/user/repos?per_page=100&page=2>; rel="next""#,
        )]),
        br#"[
            {
              "id": 101,
              "clone_url": "https://github.com/someone/a-repo.git",
              "owner": { "login": "someone", "type": "User" },
              "name": "a-repo",
              "full_name": "someone/a-repo",
              "fork": false,
              "archived": false,
              "private": false
            }
          ]"#,
    ));
    transport.push(ok(br#"[
            {
              "id": 202,
              "clone_url": "https://github.com/sample-org/b-repo.git",
              "owner": { "login": "sample-org", "type": "Organization" },
              "name": "b-repo",
              "full_name": "sample-org/b-repo",
              "permissions": { "push": true },
              "fork": true,
              "parent": { "clone_url": "https://github.com/parent-org/b-repo.git" },
              "archived": true,
              "private": true
            }
          ]"#));

    let first = provider.list_repos(&token(), None).unwrap();
    assert_eq!(first.value.next_cursor, Some(next.to_owned()));
    let second = provider
        .list_repos(&token(), first.value.next_cursor.as_deref())
        .unwrap();
    assert_eq!(second.value.next_cursor, None);

    let mut items = first.value.items;
    items.extend(second.value.items);
    assert_eq!(items.len(), 2);
    assert_eq!(transport.request_count(), 2);
    assert_eq!(transport.requests()[1].url, next);
    assert!(items.iter().all(|repo| !repo.provider_repo_id.is_empty()));
    assert!(items.iter().all(|repo| !repo.clone_url.is_empty()));
    assert_eq!(items[0].can_push, None);
    assert_eq!(items[0].name, "a-repo");
    assert!(!items[0].name.contains('/'));
    assert_eq!(items[1].can_push, Some(true));
    assert_eq!(items[1].in_org, Some("sample-org".to_owned()));
    assert_eq!(
        items[1].fork_parent_clone_url,
        Some("https://github.com/parent-org/b-repo.git".to_owned())
    );
}

#[test]
fn github_provider_uses_the_expected_api_base_for_each_host() {
    let (transport, provider) = provider_with_transport(GITHUB_CANONICAL_HOST);
    transport.push(ok(user_body()));
    provider.viewer(&token()).unwrap();
    assert!(transport.requests()[0]
        .url
        .starts_with("https://api.github.com"));

    let (transport, provider) = provider_with_transport("forge.example.invalid");
    transport.push(ok(user_body()));
    provider.viewer(&token()).unwrap();
    assert!(transport.requests()[0]
        .url
        .starts_with("https://forge.example.invalid/api/v3"));
}

#[test]
fn github_provider_sends_exactly_the_account_headers() {
    let (transport, provider) = provider_with_transport(GITHUB_CANONICAL_HOST);
    transport.push(ok(user_body()));
    provider.viewer(&token()).unwrap();
    let requests = transport.requests();
    let header_names = requests[0]
        .headers
        .iter()
        .map(|(name, _)| name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        header_names,
        [
            "authorization",
            "accept",
            "x-github-api-version",
            "user-agent"
        ]
    );
}

#[test]
fn github_scope_tiers_are_exact_and_non_destructive() {
    assert_eq!(SCOPES_PUBLIC, ["read:user", "user:email"]);
    assert_eq!(
        SCOPES_PRIVATE,
        ["read:user", "user:email", "repo", "read:org"]
    );

    for scope in SCOPES_PUBLIC.iter().chain(SCOPES_PRIVATE) {
        assert!(
            !["delete_repo", "workflow", "gist", "notifications"].contains(scope),
            "destructive scope was present: {scope}"
        );
        assert!(
            !scope.starts_with("admin:"),
            "admin scope was present: {scope}"
        );
        assert!(
            !scope.starts_with("write:"),
            "write scope was present: {scope}"
        );
    }
}

/// An alias host is folded at construction, so `www.github.com` and `github.com` are one
/// provider. Without the fold `canonical_host` answers whichever spelling the caller happened to
/// hold, `host_aliases` is empty for every alias but one, and `api_base` builds
/// `https://www.github.com/api/v3` for a host that is not an Enterprise install.
#[test]
fn an_alias_host_folds_to_the_canonical_one() {
    for alias in ["www.github.com", "ssh.github.com", "github.com"] {
        let (transport, provider) = provider_with_transport(alias);
        assert_eq!(
            provider.canonical_host(),
            GITHUB_CANONICAL_HOST,
            "{alias} did not fold"
        );
        assert_eq!(
            provider.host_aliases().len(),
            3,
            "{alias} lost the declared alias set"
        );

        transport.push(ok(user_body()));
        provider.viewer(&token()).unwrap();
        assert!(
            transport.requests()[0]
                .url
                .starts_with("https://api.github.com"),
            "{alias} was treated as an Enterprise install: {}",
            transport.requests()[0].url
        );
    }
}

/// A genuine Enterprise host keeps its own name and declares no aliases: the alias set is
/// provider-declared for github.com and guessing one for a customer's server would be exactly
/// the pattern-matching §22.2 forbids.
#[test]
fn an_enterprise_host_keeps_its_name_and_declares_no_aliases() {
    let (_transport, provider) = provider_with_transport("forge.example.invalid");
    assert_eq!(provider.canonical_host(), "forge.example.invalid");
    assert!(provider.host_aliases().is_empty());
}

/// A `permissions` object that carries no `push` key is **unknown**, not `false`, and not a
/// decode failure that would lose every other entry on the page with it.
#[test]
fn a_permissions_object_without_push_is_unknown_and_costs_no_other_entry() {
    let (transport, provider) = provider_with_transport(GITHUB_CANONICAL_HOST);
    transport.push(ok(
        br#"[{"id":1,"clone_url":"https://forge.example.invalid/someone/one.git",
              "owner":{"login":"someone","type":"User"},"name":"one",
              "permissions":{},"fork":false,"archived":false,"private":false},
             {"id":2,"clone_url":"https://forge.example.invalid/someone/two.git",
              "owner":{"login":"someone","type":"User"},"name":"two",
              "permissions":{"push":true},"fork":false,"archived":false,"private":false}]"#,
    ));
    let page = provider.list_repos(&token(), None).unwrap().value;
    assert_eq!(page.items.len(), 2, "one unknown permission lost the page");
    assert_eq!(page.items[0].can_push, None, "unknown is never false");
    assert_eq!(page.items[1].can_push, Some(true));
}

/// The `Link` header is walked as `<...>` pairs, never split on `,`. The repos endpoint's own
/// URL carries `affiliation=owner,collaborator,organization_member`, so a comma split tears it
/// apart, finds no `rel="next"`, and paginates exactly once — against the real API only.
#[test]
fn the_next_link_survives_a_url_that_contains_commas() {
    let next = "https://api.github.com/user/repos?per_page=100\
                &affiliation=owner,collaborator,organization_member&page=2";
    let last = "https://api.github.com/user/repos?per_page=100\
                &affiliation=owner,collaborator,organization_member&page=9";
    let (transport, provider) = provider_with_transport(GITHUB_CANONICAL_HOST);
    transport.push(ok_with_headers(
        normalise_headers([(
            "Link",
            format!("<{last}>; rel=\"last\", <{next}>; rel=\"next\"").as_str(),
        )]),
        b"[]",
    ));
    let page = provider.list_repos(&token(), None).unwrap().value;
    assert_eq!(
        page.next_cursor.as_deref(),
        Some(next),
        "the Link parser tore the URL apart at its commas"
    );
}

/// A `Link` header naming **another host** must not become a next page: this provider fetches
/// that URL with the account's token attached, so following one off-host hands the credential to
/// whatever the header named. Pagination ends instead.
#[test]
fn a_cross_host_next_link_is_not_followed() {
    let (transport, provider) = provider_with_transport(GITHUB_CANONICAL_HOST);
    transport.push(ok_with_headers(
        normalise_headers([(
            "Link",
            "<https://evil.example.invalid/user/repos?page=2>; rel=\"next\"",
        )]),
        b"[]",
    ));
    let page = provider.list_repos(&token(), None).unwrap().value;
    assert_eq!(
        page.next_cursor, None,
        "a next page on another host was accepted, and the token would follow it"
    );

    // The same header on the same host is still a next page — this bounds, it does not disable.
    let (transport, provider) = provider_with_transport(GITHUB_CANONICAL_HOST);
    let same = "https://api.github.com/user/repos?page=2";
    transport.push(ok_with_headers(
        normalise_headers([("Link", format!("<{same}>; rel=\"next\"").as_str())]),
        b"[]",
    ));
    assert_eq!(
        provider
            .list_repos(&token(), None)
            .unwrap()
            .value
            .next_cursor
            .as_deref(),
        Some(same)
    );
}
