//! §22.1 and §22.3 — the pure matcher, its four outcomes, and the asymmetry between a
//! provider id that is unknown and one that is known and different.
//!
//! **A listing↔project match is an identity equality or it does not happen.** Two bases only;
//! everything else — the repository name, the description, the default branch, any timestamp,
//! the directory basename — is not evidence, and here that is a property of the type rather than
//! a rule a reviewer has to check.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::sync::Arc;

use codotheca_core::identity::alias::HostAliases;
use codotheca_core::identity::candidates::{LinkCandidate, Suppressor};
use codotheca_core::identity::match_listing::{
    listing_parts_from, match_listing, ListingEvidence, ListingFacts, ListingMatch,
};
use codotheca_core::protocol::RemoteLinkBasis;
use codotheca_core::provider::listing::{RepoListing, GITHUB_CANONICAL_HOST};
use codotheca_core::provider::GitHubProvider;
use codotheca_core::testing::FakeTransport;

const PROVIDER: &str = "github";

fn evidence(provider_repo_id: &str, key: &str) -> ListingEvidence {
    ListingEvidence {
        provider: PROVIDER.to_owned(),
        provider_repo_id: provider_repo_id.to_owned(),
        remote_key: key.to_owned(),
        folded_key: key.to_owned(),
    }
}

fn candidate(project_id: i64, provider_repo_id: Option<&str>, key: Option<&str>) -> LinkCandidate {
    LinkCandidate {
        project_id,
        provider: provider_repo_id.map(|_| PROVIDER.to_owned()),
        provider_repo_id: provider_repo_id.map(ToOwned::to_owned),
        folded_key: key.map(ToOwned::to_owned),
        created_at: project_id,
    }
}

fn suppressor(project_id: i64, key: &str) -> Suppressor {
    Suppressor {
        project_id,
        folded_key: key.to_owned(),
        name: "a local copy".to_owned(),
    }
}

const KEY: &str = "forge.example/acme/widget";

#[test]
fn one_candidate_equal_on_the_provider_id_attaches_on_that_basis() {
    let outcome = match_listing(
        &evidence("42", KEY),
        &[candidate(7, Some("42"), Some("forge.example/acme/renamed"))],
        &[],
    );
    assert_eq!(
        outcome,
        ListingMatch::Attach {
            project_id: 7,
            basis: RemoteLinkBasis::ProviderId,
        },
        "the stable id survives a rename; the key does not"
    );
}

#[test]
fn one_candidate_equal_on_the_remote_key_attaches_on_that_basis() {
    let outcome = match_listing(&evidence("42", KEY), &[candidate(7, None, Some(KEY))], &[]);
    assert_eq!(
        outcome,
        ListingMatch::Attach {
            project_id: 7,
            basis: RemoteLinkBasis::RemoteKey,
        }
    );
}

#[test]
fn two_candidates_equal_on_the_winning_basis_are_ambiguous() {
    let outcome = match_listing(
        &evidence("42", KEY),
        &[candidate(7, None, Some(KEY)), candidate(9, None, Some(KEY))],
        &[],
    );
    assert_eq!(
        outcome,
        ListingMatch::Ambiguous {
            candidates: vec![7, 9]
        }
    );

    // …and the same on the id basis, which §22.5 reaches after a history rewrite.
    let on_ids = match_listing(
        &evidence("42", KEY),
        &[
            candidate(7, Some("42"), None),
            candidate(9, Some("42"), Some(KEY)),
        ],
        &[],
    );
    assert_eq!(
        on_ids,
        ListingMatch::Ambiguous {
            candidates: vec![7, 9]
        }
    );
}

#[test]
fn no_candidate_and_a_same_path_project_elsewhere_is_suppressed() {
    let outcome = match_listing(
        &evidence("42", KEY),
        &[],
        &[suppressor(3, "other.example/acme/widget")],
    );
    assert_eq!(outcome, ListingMatch::Suppress { blocked_by: 3 });
}

#[test]
fn no_candidate_and_no_suppressor_creates() {
    assert_eq!(
        match_listing(&evidence("42", KEY), &[], &[]),
        ListingMatch::Create
    );
}

/// **AC-P2-22-8.** A listing that shares a local project's name, description, default branch and
/// every timestamp — but not a basis — reaches `Create`.
///
/// **It is written against the type, not against a string.** The destructuring below has no `..`,
/// so a fifth field on `ListingEvidence` is a compile error rather than a review finding: the
/// matcher cannot consult a forbidden field because it cannot see one.
#[test]
fn nothing_but_the_two_bases() {
    let ev = evidence("42", KEY);
    let ListingEvidence {
        provider,
        provider_repo_id,
        remote_key,
        folded_key,
    } = &ev;
    assert_eq!(provider, PROVIDER);
    assert_eq!(provider_repo_id, "42");
    assert_eq!(remote_key, KEY);
    assert_eq!(folded_key, KEY);

    // Everything a softer matcher would reach for lives on the other half of the split and never
    // reaches `match_listing`.
    let facts = ListingFacts {
        name: "widget".to_owned(),
        is_fork: false,
        fork_parent_remote_key: Some("forge.example/upstream/widget".to_owned()),
    };
    let ListingFacts {
        name,
        is_fork,
        fork_parent_remote_key,
    } = &facts;
    assert_eq!(name, "widget");
    assert!(!is_fork);
    assert!(fork_parent_remote_key.is_some());

    // The local project shares the name and carries no id, and its key differs by one segment.
    let local = candidate(7, None, Some("forge.example/other/widget"));
    assert_eq!(match_listing(&ev, &[local], &[]), ListingMatch::Create);
}

#[test]
fn a_null_provider_repo_id_does_not_exclude_a_candidate() {
    // Unknown is not different. A project indexed from a clone has no id until a listing binds
    // one, so excluding it here would make the ordinary first sync create a second tile.
    let outcome = match_listing(&evidence("42", KEY), &[candidate(7, None, Some(KEY))], &[]);
    assert_eq!(
        outcome,
        ListingMatch::Attach {
            project_id: 7,
            basis: RemoteLinkBasis::RemoteKey,
        }
    );
}

#[test]
fn a_different_provider_repo_id_does_exclude_a_candidate() {
    // Known-and-different is different: the same URL after a history rewrite, or a transfer that
    // left the old key in place, is not this repository.
    let outcome = match_listing(
        &evidence("42", KEY),
        &[candidate(7, Some("99"), Some(KEY))],
        &[],
    );
    assert_eq!(
        outcome,
        ListingMatch::Create,
        "a candidate bound to another forge repository is not a key match"
    );
}

#[test]
fn a_candidate_on_another_provider_is_not_an_id_match() {
    let mut other = candidate(7, Some("42"), Some(KEY));
    other.provider = Some("other-forge".to_owned());
    assert_eq!(
        match_listing(&evidence("42", KEY), &[other], &[]),
        ListingMatch::Create,
        "the id basis is the pair, never the id alone"
    );
}

/// The one place a `RepoListing` is taken apart, so the evidence/facts split cannot be routed
/// around. A listing whose clone URL does not canonicalise produces neither half.
#[test]
fn a_listing_is_split_once_and_only_here() {
    let provider = GitHubProvider::new(
        Arc::new(FakeTransport::new()),
        GITHUB_CANONICAL_HOST.to_owned(),
    );
    let aliases = HostAliases::from_provider(&provider);

    let listing = RepoListing {
        provider: "github",
        provider_repo_id: "42".to_owned(),
        clone_url: format!("https://www.{GITHUB_CANONICAL_HOST}/acme/Widget.git"),
        owner: "acme".to_owned(),
        name: "Widget".to_owned(),
        can_push: Some(true),
        is_fork: true,
        fork_parent_clone_url: Some(format!(
            "https://{GITHUB_CANONICAL_HOST}/upstream/widget.git"
        )),
        is_archived: false,
        is_private: false,
        in_org: None,
    };
    let (ev, facts) = listing_parts_from(&listing, &aliases).unwrap();

    assert_eq!(ev.provider, "github");
    assert_eq!(ev.provider_repo_id, "42");
    assert_eq!(
        ev.remote_key,
        format!("www.{GITHUB_CANONICAL_HOST}/acme/widget")
    );
    assert_eq!(
        ev.folded_key,
        format!("{GITHUB_CANONICAL_HOST}/acme/widget"),
        "the stored key keeps the spelling git will contact; only the comparison form folds"
    );
    assert_eq!(facts.name, "Widget", "the BARE name, never owner/name");
    assert!(facts.is_fork);
    assert_eq!(
        facts.fork_parent_remote_key.as_deref(),
        Some(format!("{GITHUB_CANONICAL_HOST}/upstream/widget").as_str())
    );

    let local = RepoListing {
        clone_url: "/srv/git/widget.git".to_owned(),
        ..listing
    };
    assert_eq!(listing_parts_from(&local, &aliases), None);
}

// ---------------------------------------------------------------------------------------------
// Task 7 — the two loaders. What the matcher is given, read through an index.
// ---------------------------------------------------------------------------------------------

use codotheca_core::accounts::keychain::SecretToken;
use codotheca_core::identity::candidates::{
    load_link_candidates, load_suppressors, LINK_CANDIDATES_BY_ID_SQL, LINK_CANDIDATES_BY_KEY_SQL,
};
use codotheca_core::index::migrate::{apply_all, MIGRATIONS};
use codotheca_core::index::{open_connection, Index};
use codotheca_core::provider::listing::{OrgListing, Page, Viewer};
use codotheca_core::provider::{CiRunsRead, Observed, Provider, ProviderResult, RepoFactsRead};

/// A forge that declares an alias set and issues no request; the loaders reach no network.
#[derive(Debug)]
struct DeclaringForge;

impl Provider for DeclaringForge {
    fn viewer(&self, _t: &SecretToken) -> ProviderResult<Observed<Viewer>> {
        unreachable!("the loaders issue no request")
    }
    fn list_orgs(
        &self,
        _t: &SecretToken,
        _cur: Option<&str>,
    ) -> ProviderResult<Observed<Page<OrgListing>>> {
        unreachable!("the loaders issue no request")
    }
    fn list_repos(
        &self,
        _t: &SecretToken,
        _cur: Option<&str>,
    ) -> ProviderResult<Observed<Page<RepoListing>>> {
        unreachable!("the loaders issue no request")
    }
    fn lookup_repo(
        &self,
        _t: &SecretToken,
        _owner: &str,
        _name: &str,
    ) -> ProviderResult<Observed<Option<RepoListing>>> {
        unreachable!("this fixture forge issues no request")
    }
    // §25's two reads. This fixture is about identity, which never touches them.
    fn repo_facts(
        &self,
        _t: &SecretToken,
        _owner: &str,
        _name: &str,
        _etag: Option<&str>,
    ) -> ProviderResult<Observed<RepoFactsRead>> {
        unreachable!("this fixture forge issues no request")
    }
    fn ci_runs(
        &self,
        _t: &SecretToken,
        _owner: &str,
        _name: &str,
        _etag: Option<&str>,
    ) -> ProviderResult<Observed<CiRunsRead>> {
        unreachable!("this fixture forge issues no request")
    }
    fn advisories(
        &self,
        _ecosystem: codotheca_core::protocol::Ecosystem,
        _affects: &[codotheca_core::provider::PackageVersion],
        _cur: Option<&str>,
    ) -> ProviderResult<Observed<Page<codotheca_core::provider::AdvisoryPayload>>> {
        unreachable!("a fixture forge issues no request")
    }
    fn canonical_host(&self) -> &'static str {
        "forge.example"
    }
    fn host_aliases(&self) -> &[&str] {
        &["forge.example", "www.forge.example", "ssh.forge.example"]
    }
}

fn forge_aliases() -> HostAliases {
    HostAliases::from_provider(&DeclaringForge)
}

fn migrated() -> (tempfile::TempDir, rusqlite::Connection) {
    let dir = tempfile::tempdir().unwrap();
    let mut conn = open_connection(&Index::db_path(dir.path())).unwrap();
    apply_all(&mut conn, MIGRATIONS).unwrap();
    (dir, conn)
}

/// One `project` row, with only the columns these loaders read.
fn project(
    conn: &rusqlite::Connection,
    name: &str,
    remote_key: Option<&str>,
    binding: Option<(&str, &str)>,
    created_at: i64,
) -> i64 {
    conn.execute(
        "INSERT INTO project (name, seed_basename, remote_key, provider, provider_repo_id,
                              created_at, updated_at)
         VALUES (?1, ?1, ?2, ?3, ?4, ?5, ?5)",
        rusqlite::params![
            name,
            remote_key,
            binding.map(|b| b.0),
            binding.map(|b| b.1),
            created_at
        ],
    )
    .unwrap();
    conn.last_insert_rowid()
}

fn location(conn: &rusqlite::Connection, project_id: i64, path: &str) {
    conn.execute(
        "INSERT INTO location (project_id, kind, distro, path_bytes, path_key, path_display,
                               store_key, presence, repo_kind, scan_generation)
         VALUES (?1, 'linux', '', ?2, ?2, ?3, 'store', 'present', 'worktree', 1)",
        rusqlite::params![project_id, path.as_bytes(), path],
    )
    .unwrap();
}

#[test]
fn a_candidate_is_found_through_an_alias_host() {
    let (_dir, mut conn) = migrated();
    // The clone was taken over an alias host, so the stored key is the alias spelling.
    let stored = project(
        &conn,
        "widget",
        Some("ssh.forge.example/acme/widget"),
        None,
        100,
    );
    let tx = conn.transaction().unwrap();
    let found = load_link_candidates(
        &tx,
        &evidence("42", "forge.example/acme/widget"),
        &forge_aliases(),
    )
    .unwrap();
    tx.commit().unwrap();

    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].project_id, stored);
    assert_eq!(
        found[0].folded_key.as_deref(),
        Some("forge.example/acme/widget"),
        "the candidate is handed to the matcher in its comparison form"
    );
}

#[test]
fn a_tombstoned_project_is_never_a_candidate() {
    let (_dir, mut conn) = migrated();
    let survivor = project(
        &conn,
        "survivor",
        Some("forge.example/acme/widget"),
        None,
        100,
    );
    let absorbed = project(
        &conn,
        "absorbed",
        Some("forge.example/acme/widget"),
        None,
        101,
    );
    conn.execute(
        "UPDATE project SET merged_into = ?1 WHERE id = ?2",
        rusqlite::params![survivor, absorbed],
    )
    .unwrap();

    let tx = conn.transaction().unwrap();
    let found = load_link_candidates(
        &tx,
        &evidence("42", "forge.example/acme/widget"),
        &forge_aliases(),
    )
    .unwrap();
    let suppressors = load_suppressors(
        &tx,
        &evidence("42", "forge.example/acme/widget"),
        &forge_aliases(),
    )
    .unwrap();
    tx.commit().unwrap();

    assert_eq!(
        found.iter().map(|c| c.project_id).collect::<Vec<_>>(),
        vec![survivor]
    );
    assert!(suppressors.is_empty());
}

#[test]
fn candidates_come_back_ordered_by_created_at_then_id() {
    let (_dir, mut conn) = migrated();
    // Inserted newest-first, and two share a timestamp, so id breaks the tie.
    let late = project(&conn, "c", Some("forge.example/acme/widget"), None, 300);
    let early_a = project(&conn, "a", Some("www.forge.example/acme/widget"), None, 100);
    let early_b = project(&conn, "b", Some("forge.example/acme/widget"), None, 100);
    let by_id = project(&conn, "d", None, Some(("github", "42")), 200);

    let tx = conn.transaction().unwrap();
    let found = load_link_candidates(
        &tx,
        &evidence("42", "forge.example/acme/widget"),
        &forge_aliases(),
    )
    .unwrap();
    tx.commit().unwrap();

    assert_eq!(
        found.iter().map(|c| c.project_id).collect::<Vec<_>>(),
        vec![early_a, early_b, by_id, late],
        "the outcome must not depend on the order a walk reached a row in"
    );
}

#[test]
fn a_project_is_returned_once_even_when_both_reads_find_it() {
    let (_dir, mut conn) = migrated();
    let both = project(
        &conn,
        "widget",
        Some("forge.example/acme/widget"),
        Some(("github", "42")),
        100,
    );
    let tx = conn.transaction().unwrap();
    let found = load_link_candidates(
        &tx,
        &evidence("42", "forge.example/acme/widget"),
        &forge_aliases(),
    )
    .unwrap();
    tx.commit().unwrap();
    assert_eq!(
        found.iter().map(|c| c.project_id).collect::<Vec<_>>(),
        vec![both],
        "one project, one candidate — two of it would read as §22.5's ambiguity"
    );
}

/// §22.6's implicit clause, and it is the one that matters: **a not-cloned project is not a copy
/// on the user's disk and cannot be the doubt that withholds a claim about the user's disk.**
#[test]
fn a_not_cloned_project_never_suppresses() {
    let (_dir, mut conn) = migrated();
    let elsewhere = project(
        &conn,
        "widget",
        Some("other.example/acme/widget"),
        None,
        100,
    );

    let tx = conn.transaction().unwrap();
    let ev = evidence("42", "forge.example/acme/widget");
    let blockers = load_suppressors(&tx, &ev, &forge_aliases()).unwrap();
    assert!(blockers.is_empty(), "{blockers:?}");
    assert_eq!(match_listing(&ev, &[], &blockers), ListingMatch::Create);

    // Give it a copy on disk and the same fixture now withholds the claim — which is what makes
    // the assertion above about the location clause rather than about an empty fixture.
    location(&tx, elsewhere, "/w/widget");
    let blockers = load_suppressors(&tx, &ev, &forge_aliases()).unwrap();
    tx.commit().unwrap();

    assert_eq!(
        blockers.iter().map(|s| s.project_id).collect::<Vec<_>>(),
        vec![elsewhere]
    );
    assert_eq!(blockers[0].name, "widget");
    assert_eq!(
        match_listing(&ev, &[], &blockers),
        ListingMatch::Suppress {
            blocked_by: elsewhere
        }
    );
}

#[test]
fn a_copy_on_the_same_host_is_not_a_suppressor() {
    let (_dir, mut conn) = migrated();
    // The alias host folds to the canonical one, so this is the SAME host and the path-component
    // rule must not fire: it is a candidate, not a doubt.
    let same = project(
        &conn,
        "widget",
        Some("ssh.forge.example/acme/widget"),
        None,
        100,
    );
    let tx = conn.transaction().unwrap();
    location(&tx, same, "/w/widget");
    let ev = evidence("42", "forge.example/acme/widget");
    let blockers = load_suppressors(&tx, &ev, &forge_aliases()).unwrap();
    let found = load_link_candidates(&tx, &ev, &forge_aliases()).unwrap();
    tx.commit().unwrap();

    assert!(blockers.is_empty(), "{blockers:?}");
    assert_eq!(found.len(), 1);
}

#[test]
fn a_query_plan_uses_the_index() {
    let (_dir, conn) = migrated();
    let plan = |sql: &str, args: &[&dyn rusqlite::ToSql]| -> String {
        let mut st = conn
            .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
            .unwrap_or_else(|e| panic!("{sql}: {e}"));
        let rows = st
            .query_map(args, |r| r.get::<_, String>(3))
            .unwrap()
            .map(Result::unwrap)
            .collect::<Vec<_>>();
        assert!(!rows.is_empty(), "no plan for {sql}");
        rows.join(" | ")
    };

    let by_id = plan(LINK_CANDIDATES_BY_ID_SQL, &[&"github", &"42"]);
    assert!(
        by_id.contains("idx_project_provider_repo"),
        "the id read must not table-scan: {by_id}"
    );
    assert!(!by_id.contains("SCAN project"), "{by_id}");

    let by_key = plan(LINK_CANDIDATES_BY_KEY_SQL, &[&"forge.example/acme/widget"]);
    assert!(
        by_key.contains("idx_project_remote"),
        "the key read must not table-scan: {by_key}"
    );
    assert!(!by_key.contains("SCAN project"), "{by_key}");
    eprintln!("by id: {by_id}\nby key: {by_key}");
}
