#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! §25.1's four states, read back out of a **real migrated database**.
//!
//! The distinction the whole section rests on is here: a measured zero survives as `Some(0)` and
//! an unobserved value is `None`. One reader that collapses the two renders unknown as zero on
//! every surface at once.

use codotheca_core::index::Index;
use codotheca_core::protocol::{ProjectId, RemoteFactsState, RemoteVisibility};
use codotheca_core::remote::facts::remote_facts;

const NOW: i64 = 1_781_179_200;

/// One project, its binding, and whatever the caller wants on top.
fn seeded() -> (tempfile::TempDir, Index) {
    let dir = tempfile::tempdir().expect("tempdir");
    let index = Index::open(dir.path()).expect("open");
    index
        .conn()
        .execute(
            "INSERT INTO project (id, name, seed_basename, remote_key, last_touched_at,
                                  created_at, updated_at)
             VALUES (1, 'widget', 'widget', 'github.com/acme/widget', ?1, ?1, ?1)",
            rusqlite::params![NOW],
        )
        .expect("seed project");
    (dir, index)
}

fn bind(index: &Index, project: i64, repo_id: &str) {
    index
        .conn()
        .execute(
            "UPDATE project SET provider = 'github', provider_repo_id = ?2,
                                remote_link_basis = 'provider_id' WHERE id = ?1",
            rusqlite::params![project, repo_id],
        )
        .expect("bind");
}

fn connect_account(index: &Index) {
    index
        .conn()
        .execute(
            "INSERT INTO account (provider, host, login, auth_kind, scope_tier, granted_scopes,
                                  token_ref, connected_at)
             VALUES ('github', 'github.com', 'someone', 'device', 'private', '[]',
                     'github:github.com:someone', ?1)",
            rusqlite::params![NOW],
        )
        .expect("account");
}

/// A full facts row, with a **measured zero** in one of the counts.
fn full_facts_row(index: &Index, repo_id: &str) {
    index
        .conn()
        .execute(
            "INSERT INTO remote_repo
               (provider, provider_repo_id, visibility, description, fork_parent_remote_key,
                stars, open_issues, good_first_issues, open_prs, open_prs_from_user,
                permitted, observed_at, etag, ci_observed_at, ci_etag)
             VALUES ('github', ?1, 'public', 'a widget', 'github.com/upstream/widget',
                     41, 7, 0, 2, 0, 1, ?2, 'W/\"abc\"', ?3, 'W/\"def\"')",
            rusqlite::params![repo_id, NOW - 60, NOW - 30],
        )
        .expect("facts row");
}

#[test]
fn a_project_with_no_remote_key_has_no_facts_at_all() {
    let (_dir, index) = seeded();
    index
        .conn()
        .execute("UPDATE project SET remote_key = NULL WHERE id = 1", [])
        .expect("clear key");
    assert_eq!(
        remote_facts(index.conn(), ProjectId(1)).expect("read"),
        None,
        "NULL remote_key is the tab's presence predicate and must answer None"
    );
}

#[test]
fn a_key_with_no_account_connected_reads_no_account() {
    let (_dir, index) = seeded();
    let facts = remote_facts(index.conn(), ProjectId(1))
        .expect("read")
        .expect("a key produces facts");
    assert_eq!(facts.state, RemoteFactsState::NoAccount);
    assert_eq!(facts.ci.state, RemoteFactsState::NoAccount);
    assert_eq!(facts.key, "github.com/acme/widget");
    assert!(facts.linkable, "github.com is on the base allowlist");
}

#[test]
fn an_account_with_no_facts_row_reads_not_observed() {
    let (_dir, index) = seeded();
    connect_account(&index);
    bind(&index, 1, "909");
    let facts = remote_facts(index.conn(), ProjectId(1))
        .expect("read")
        .expect("facts");
    assert_eq!(facts.state, RemoteFactsState::NotObserved);
    assert_eq!(facts.observed_at, None);
    assert_eq!(facts.stars, None, "unobserved is never zero");
    assert_eq!(facts.ci.state, RemoteFactsState::NotObserved);
    assert!(facts.ci.runs.is_empty());
}

/// A binding is what the facts row is keyed on. A project with a key and no binding has no row
/// to find, which is *not observed* and not an error.
#[test]
fn an_account_with_no_binding_reads_not_observed() {
    let (_dir, index) = seeded();
    connect_account(&index);
    let facts = remote_facts(index.conn(), ProjectId(1))
        .expect("read")
        .expect("facts");
    assert_eq!(facts.state, RemoteFactsState::NotObserved);
}

#[test]
fn a_row_the_token_may_not_see_reads_not_permitted() {
    let (_dir, index) = seeded();
    connect_account(&index);
    bind(&index, 1, "909");
    full_facts_row(&index, "909");
    index
        .conn()
        .execute("UPDATE remote_repo SET permitted = 0", [])
        .expect("refuse");
    let facts = remote_facts(index.conn(), ProjectId(1))
        .expect("read")
        .expect("facts");
    assert_eq!(facts.state, RemoteFactsState::NotPermitted);
}

#[test]
fn a_full_row_reads_observed_with_every_count_and_its_clock_intact() {
    let (_dir, index) = seeded();
    connect_account(&index);
    bind(&index, 1, "909");
    full_facts_row(&index, "909");
    index
        .conn()
        .execute(
            "INSERT INTO remote_topic (provider, provider_repo_id, topic)
             VALUES ('github', '909', 'rust'), ('github', '909', 'cli')",
            [],
        )
        .expect("topics");
    index
        .conn()
        .execute(
            "INSERT INTO remote_ci_run
               (provider, provider_repo_id, run_id, workflow_name, conclusion, branch,
                run_number, started_at)
             VALUES ('github', '909', 11, 'ci', 'success', 'main', 41, ?1),
                    ('github', '909', 12, 'release', NULL, 'main', 42, ?2)",
            rusqlite::params![NOW - 900, NOW - 300],
        )
        .expect("runs");

    let facts = remote_facts(index.conn(), ProjectId(1))
        .expect("read")
        .expect("facts");
    assert_eq!(facts.state, RemoteFactsState::Observed);
    assert_eq!(facts.observed_at, Some(NOW - 60));
    assert_eq!(facts.visibility, Some(RemoteVisibility::Public));
    assert_eq!(
        facts.fork_parent_key.as_deref(),
        Some("github.com/upstream/widget")
    );
    assert_eq!(facts.stars, Some(41));
    assert_eq!(facts.open_issues, Some(7));
    assert_eq!(facts.open_prs, Some(2));
    // The whole of AC-P2-25-4 on the core side: a measured zero is `Some(0)`, never `None`.
    assert_eq!(facts.good_first_issues, Some(0));
    assert_eq!(facts.open_prs_from_user, Some(0));
    // Ordered by topic, so the rail does not re-sort a set the core already has an order for.
    assert_eq!(facts.topics, vec!["cli".to_owned(), "rust".to_owned()]);

    // The CI list carries its own state and its own clock: two reads, two instants.
    assert_eq!(facts.ci.state, RemoteFactsState::Observed);
    assert_eq!(facts.ci.observed_at, Some(NOW - 30));
    assert_eq!(facts.ci.runs.len(), 2);
    // Most recent first.
    assert_eq!(facts.ci.runs[0].run_id, 12);
    assert_eq!(facts.ci.runs[0].conclusion, None);
    assert_eq!(facts.ci.runs[1].workflow, "ci");
    assert_eq!(facts.ci.runs[1].conclusion.as_deref(), Some("success"));
    assert_eq!(facts.ci.runs[1].branch, "main");
    assert_eq!(facts.ci.runs[1].run_number, 41);
}

/// The Actions read has not run while the repo-facts read has. A CI list dated by the other
/// read would be the staleness marker lying, so the two states answer independently.
#[test]
fn the_ci_list_answers_not_observed_while_the_counts_read_observed() {
    let (_dir, index) = seeded();
    connect_account(&index);
    bind(&index, 1, "909");
    full_facts_row(&index, "909");
    index
        .conn()
        .execute(
            "UPDATE remote_repo SET ci_observed_at = NULL, ci_etag = NULL",
            [],
        )
        .expect("clear ci clock");
    let facts = remote_facts(index.conn(), ProjectId(1))
        .expect("read")
        .expect("facts");
    assert_eq!(facts.state, RemoteFactsState::Observed);
    assert_eq!(facts.ci.state, RemoteFactsState::NotObserved);
    assert_eq!(facts.ci.observed_at, None);
}

/// §25.2: `linkable` is the core's answer, so the renderer holds no second copy of the host
/// allowlist. A key on an unlisted host is rendered as text with no link affordance.
#[test]
fn a_key_on_an_unlisted_host_is_not_linkable() {
    let (_dir, index) = seeded();
    index
        .conn()
        .execute(
            "UPDATE project SET remote_key = 'forge.example.invalid/acme/widget' WHERE id = 1",
            [],
        )
        .expect("rewrite key");
    let facts = remote_facts(index.conn(), ProjectId(1))
        .expect("read")
        .expect("facts");
    assert!(!facts.linkable);
    assert_eq!(facts.key, "forge.example.invalid/acme/widget");
}

// ---------------------------------------------------------------------------------------------
// §25.1's two provider reads, against a stubbed transport. The production caller is p2-21's
// `run_project_remote`, which arrives in **wave 5** (R65) — until it lands, every `RemoteFacts`
// on every surface reads `not_observed`, which is the honest render and not a gap.
// ---------------------------------------------------------------------------------------------

use codotheca_core::accounts::keychain::SecretToken;
use codotheca_core::http::{normalise_headers, HttpResponse, HttpTransport};
use codotheca_core::provider::listing::GITHUB_CANONICAL_HOST;
use codotheca_core::provider::{GitHubProvider, Provider, PROVIDER_REQUEST_METHODS};
use codotheca_core::testing::FakeTransport;
use std::sync::Arc;

fn forge() -> (Arc<FakeTransport>, Arc<dyn Provider>) {
    let transport = Arc::new(FakeTransport::new());
    let http: Arc<dyn HttpTransport> = transport.clone();
    let provider: Arc<dyn Provider> =
        Arc::new(GitHubProvider::new(http, GITHUB_CANONICAL_HOST.to_owned()));
    (transport, provider)
}

fn token() -> SecretToken {
    SecretToken::new("remote-facts-token".to_owned())
}

fn answer(status: u16, headers: Vec<(String, String)>, body: &[u8]) -> HttpResponse {
    HttpResponse {
        status,
        headers,
        body: body.to_vec(),
    }
}

const REPO_BODY: &[u8] = br#"{"id":909,"clone_url":"https://github.com/acme/widget.git",
     "owner":{"login":"acme","type":"User"},"name":"widget","fork":true,
     "parent":{"clone_url":"https://github.com/upstream/widget.git"},
     "archived":false,"private":false,"description":"a widget",
     "stargazers_count":41,"topics":["rust","cli"]}"#;

const RUNS_BODY: &[u8] = br#"{"total_count":1,"workflow_runs":[
     {"id":11,"name":"ci","conclusion":"success","head_branch":"main",
      "run_number":41,"run_started_at":"2026-09-01T10:00:00Z"}]}"#;

#[test]
fn a_two_hundred_yields_facts_and_the_response_etag() {
    let (transport, provider) = forge();
    transport.push(answer(
        200,
        normalise_headers([("ETag", "W/\"abc\"")]),
        REPO_BODY,
    ));
    let read = provider
        .repo_facts(&token(), "acme", "widget", None)
        .expect("a 200 is an answer")
        .value;
    assert_eq!(read.status, 200);
    assert_eq!(read.etag.as_deref(), Some("W/\"abc\""));
    let facts = read.facts.expect("a 200 carries facts");
    assert_eq!(facts.stars, Some(41));
    assert_eq!(facts.visibility.as_deref(), Some("public"));
    assert_eq!(facts.description.as_deref(), Some("a widget"));
    assert_eq!(
        facts.fork_parent_remote_key.as_deref(),
        Some("github.com/upstream/widget")
    );
    assert_eq!(facts.topics, vec!["rust".to_owned(), "cli".to_owned()]);
    // `open_issues_count` counts issues **and** pull requests, so it answers neither block
    // §25.1 draws. Unobserved renders `—`; a wrong number under a labelled block renders as a
    // fact. See this plan's report for who closes it.
    assert_eq!(facts.open_issues, None);
    assert_eq!(facts.open_prs, None);
    assert_eq!(facts.good_first_issues, None);
    assert_eq!(facts.open_prs_from_user, None);
}

/// A14: a `304` is a completed conditional read. It carries no body, and the validator the
/// caller sent is the one that is still current.
#[test]
fn a_three_oh_four_yields_no_body_and_carries_the_callers_validator() {
    let (transport, provider) = forge();
    transport.push(answer(304, Vec::new(), b""));
    let read = provider
        .repo_facts(&token(), "acme", "widget", Some("W/\"abc\""))
        .expect("a 304 is an answer, not an error")
        .value;
    assert_eq!(read.status, 304);
    assert_eq!(read.etag.as_deref(), Some("W/\"abc\""));
    assert!(read.facts.is_none());

    // …and the validator reached the wire as a conditional request rather than being dropped.
    let sent = &transport.requests()[0];
    assert!(sent
        .headers
        .iter()
        .any(|(name, value)| name == "if-none-match" && value == "W/\"abc\""));
}

#[test]
fn a_four_oh_three_and_a_four_oh_four_observe_the_access_state_and_no_facts() {
    for status in [403_u16, 404] {
        let (transport, provider) = forge();
        transport.push(answer(status, Vec::new(), b"{}"));
        let read = provider
            .repo_facts(&token(), "acme", "widget", Some("W/\"abc\""))
            .unwrap_or_else(|e| panic!("{status} must be an answer, not an error: {e}"))
            .value;
        assert_eq!(read.status, status);
        assert!(read.facts.is_none());
        // It observed no representation, so it carries no validator forward either.
        assert_eq!(read.etag, None);
    }
}

#[test]
fn an_unparseable_body_is_a_failure_and_not_a_panic() {
    let (transport, provider) = forge();
    transport.push(answer(200, Vec::new(), b"{not json"));
    assert!(provider
        .repo_facts(&token(), "acme", "widget", None)
        .is_err());

    let (transport, provider) = forge();
    transport.push(answer(200, Vec::new(), b"{not json"));
    assert!(provider.ci_runs(&token(), "acme", "widget", None).is_err());
}

#[test]
fn the_actions_read_carries_its_own_validator_and_its_own_rows() {
    let (transport, provider) = forge();
    transport.push(answer(
        200,
        normalise_headers([("ETag", "W/\"def\"")]),
        RUNS_BODY,
    ));
    let read = provider
        .ci_runs(&token(), "acme", "widget", None)
        .expect("a 200 is an answer")
        .value;
    assert_eq!(read.etag.as_deref(), Some("W/\"def\""));
    let runs = read.runs.expect("a 200 carries runs");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].run_id, 11);
    assert_eq!(runs[0].workflow_name, "ci");
    assert_eq!(runs[0].conclusion.as_deref(), Some("success"));
    assert_eq!(runs[0].branch, "main");
    assert_eq!(runs[0].run_number, 41);
    // 2026-09-01T10:00:00Z. A time that cannot be read is unknown, never the epoch.
    assert_eq!(runs[0].started_at, Some(1_788_256_800));
}

/// AC-P2-25-5's census, and it runs over the only two methods that read §25's field set.
///
/// §25.1 makes `BEHIND` the **local** figure deliberately: one owner per value, and a compare
/// call per project per sync is a rate-budget cost for a number already stored. There is no sync
/// runner in wave 4 (R65) and there is nowhere else such a call could originate — `repo_facts`
/// and `ci_runs` are the whole of this section's forge surface — so the census runs over them.
#[test]
fn no_request_this_section_issues_asks_the_forge_for_divergence() {
    let (transport, provider) = forge();
    transport.push(answer(200, Vec::new(), REPO_BODY));
    transport.push(answer(200, Vec::new(), RUNS_BODY));
    provider
        .repo_facts(&token(), "acme", "widget", None)
        .expect("facts");
    provider
        .ci_runs(&token(), "acme", "widget", None)
        .expect("runs");

    let requests = transport.requests();
    eprintln!(
        "remote_facts_read: divergence census examined {} request(s)",
        requests.len()
    );
    assert!(
        !requests.is_empty(),
        "a census that examined zero requests proves nothing"
    );
    for request in &requests {
        assert!(
            !request.url.contains("/compare"),
            "a divergence call reached the forge: {}",
            request.url
        );
        assert!(
            !request.url.contains("basehead"),
            "a divergence parameter reached the forge: {}",
            request.url
        );
        assert!(
            !request.url.contains("..."),
            "a base...head range reached the forge: {}",
            request.url
        );
    }
}

/// The census names both of this section's methods. The **count** is pinned once, in
/// `core/tests/provider_seam.rs`, against the trait's own signature scan — a second copy of the
/// number here is the drift this project keeps paying for.
#[test]
fn the_request_census_names_both_of_this_sections_methods() {
    assert!(PROVIDER_REQUEST_METHODS.contains(&"repo_facts"));
    assert!(PROVIDER_REQUEST_METHODS.contains(&"ci_runs"));
}
