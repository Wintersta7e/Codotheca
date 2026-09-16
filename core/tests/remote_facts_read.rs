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
