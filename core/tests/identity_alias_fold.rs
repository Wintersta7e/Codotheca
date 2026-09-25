//! §22.2 — the provider-declared host-alias fold. **AC-P2-22-11.**
//!
//! `host_of` lowercases and strips a numeric port and does nothing else
//! (`core/src/identity/remote.rs:81-93`), so a clone taken over an alias host and a listing
//! published on the canonical one are three different keys today. The fold is the fix §1.1 names
//! and does not supply.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::sync::Arc;

use codotheca_core::accounts::keychain::SecretToken;
use codotheca_core::identity::alias::{fold_host, fold_key, HostAliases};
use codotheca_core::identity::remote::canonical_remote_key;
use codotheca_core::provider::listing::{
    OrgListing, Page, RepoListing, Viewer, GITHUB_CANONICAL_HOST, GITHUB_HOST_ALIASES,
};
use codotheca_core::provider::{
    CiRunsRead, GitHubProvider, Observed, Provider, ProviderResult, RepoFactsRead,
};
use codotheca_core::testing::{FakeTransport, TempIndex};

/// A provider that declares an alias set and issues no request. The fold is a pure comparison
/// form, so every request method here is unreachable by construction.
#[derive(Debug)]
struct DeclaringForge {
    canonical: &'static str,
    aliases: &'static [&'static str],
}

impl Provider for DeclaringForge {
    fn viewer(&self, _t: &SecretToken) -> ProviderResult<Observed<Viewer>> {
        unreachable!("the alias fold issues no request")
    }
    fn list_orgs(
        &self,
        _t: &SecretToken,
        _cur: Option<&str>,
    ) -> ProviderResult<Observed<Page<OrgListing>>> {
        unreachable!("the alias fold issues no request")
    }
    fn list_repos(
        &self,
        _t: &SecretToken,
        _cur: Option<&str>,
    ) -> ProviderResult<Observed<Page<RepoListing>>> {
        unreachable!("the alias fold issues no request")
    }
    fn lookup_repo(
        &self,
        _t: &SecretToken,
        _owner: &str,
        _name: &str,
    ) -> ProviderResult<Observed<Option<RepoListing>>> {
        unreachable!("the alias fold issues no request")
    }
    // §25's two reads. This fixture is about identity, which never touches them.
    fn repo_facts(
        &self,
        _t: &SecretToken,
        _owner: &str,
        _name: &str,
        _etag: Option<&str>,
    ) -> ProviderResult<Observed<RepoFactsRead>> {
        unreachable!("the alias fold issues no request")
    }
    fn ci_runs(
        &self,
        _t: &SecretToken,
        _owner: &str,
        _name: &str,
        _etag: Option<&str>,
    ) -> ProviderResult<Observed<CiRunsRead>> {
        unreachable!("the alias fold issues no request")
    }
    fn advisories(
        &self,
        _ecosystem: codotheca_core::protocol::Ecosystem,
        _affects: &[codotheca_core::provider::PackageVersion],
        _cur: Option<&str>,
    ) -> ProviderResult<Observed<Page<codotheca_core::provider::AdvisoryPayload>>> {
        unreachable!("a fixture forge issues no request")
    }
    fn canonical_host(&self) -> &str {
        self.canonical
    }
    fn host_aliases(&self) -> &[&str] {
        self.aliases
    }
}

const FORGE: DeclaringForge = DeclaringForge {
    canonical: "forge.example",
    aliases: &["forge.example", "www.forge.example", "ssh.forge.example"],
};

fn aliases() -> HostAliases {
    HostAliases::from_provider(&FORGE)
}

fn key_of(url: &str) -> String {
    canonical_remote_key(url).unwrap().key
}

/// **AC-P2-22-11.** A clone taken over a declared alias host and a listing published on the
/// canonical host are one repository, and the comparison is asserted in **both argument orders**
/// so a one-sided fold is caught.
#[test]
fn a_clone_on_a_declared_alias_host_folds_equal_to_the_canonical_listing() {
    let set = aliases();
    let clone = key_of("ssh://git@ssh.forge.example/acme/widget.git");
    let listing = key_of("https://forge.example/acme/widget.git");
    assert_ne!(
        clone, listing,
        "the unfolded keys differ, or nothing is proved"
    );

    let folded_clone = fold_key(&clone, &set).unwrap();
    let folded_listing = fold_key(&listing, &set).unwrap();
    assert_eq!(folded_clone, folded_listing);
    assert_eq!(folded_listing, folded_clone);
    assert_eq!(folded_clone, "forge.example/acme/widget");

    // The `www.` spelling folds to the same value, and so does the port-stripped form.
    for url in [
        "https://www.forge.example/acme/widget.git",
        "https://forge.example:443/acme/widget",
        "git@ssh.forge.example:acme/widget.git",
    ] {
        assert_eq!(
            fold_key(&key_of(url), &set).unwrap(),
            folded_listing,
            "{url}"
        );
    }
}

#[test]
fn an_undeclared_host_folds_to_itself_and_compares_unequal() {
    let set = aliases();
    // An SSH-config alias is unresolvable without reading ~/.ssh/config, which the app does not
    // read. It folds to itself and reaches §22.6's suppression rather than producing a match.
    let unknown = key_of("git@forge-work:acme/widget.git");
    let canonical = key_of("https://forge.example/acme/widget.git");

    let folded_unknown = fold_key(&unknown, &set).unwrap();
    assert_eq!(
        folded_unknown, unknown,
        "an undeclared host folds to itself"
    );
    assert_ne!(folded_unknown, fold_key(&canonical, &set).unwrap());
    assert_ne!(fold_key(&canonical, &set).unwrap(), folded_unknown);
    assert!(!set.contains("forge-work"));
    assert_eq!(fold_host("forge-work", &set), "forge-work");
}

#[test]
fn the_fold_touches_only_the_host() {
    let set = aliases();
    let folded = fold_key(
        &key_of("https://ssh.forge.example/group/sub/widget.git"),
        &set,
    )
    .unwrap();
    assert_eq!(folded, "forge.example/group/sub/widget");
    assert!(
        folded.ends_with("group/sub/widget"),
        "the path component is byte-identical"
    );
    // And a key with no path at all is not a key.
    assert_eq!(fold_key("forge.example", &set), None);
}

/// The fold is a **comparison form**. Nothing stored is rewritten — a stored `remote_key` is what
/// `git config` will produce again on the next scan, and rewriting it would make the column
/// disagree with the repository on disk.
#[test]
fn the_fold_writes_nothing() {
    let set = aliases();
    let temp = TempIndex::new();
    let project = temp.insert_project();
    let stored = key_of("https://ssh.forge.example/acme/widget.git");
    temp.index()
        .conn()
        .execute(
            "UPDATE project SET remote_key = ?2 WHERE id = ?1",
            rusqlite::params![project.0, stored],
        )
        .unwrap();

    let folded = fold_key(&stored, &set).unwrap();
    assert_ne!(folded, stored);

    let after: Option<String> = temp
        .index()
        .conn()
        .query_row(
            "SELECT remote_key FROM project WHERE id = ?1",
            rusqlite::params![project.0],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        after.as_deref(),
        Some(stored.as_str()),
        "the fold rewrote a stored key"
    );
}

/// The set's contents are the provider's, never this module's. A gist host is excluded there —
/// a gist is not a repository — and this asserts the exclusion travels rather than restating it.
#[test]
fn the_set_comes_from_the_provider_and_excludes_gist_hosts() {
    let provider = GitHubProvider::new(
        Arc::new(FakeTransport::new()),
        GITHUB_CANONICAL_HOST.to_owned(),
    );
    let set = HostAliases::from_provider(&provider);
    assert_eq!(
        fold_host(GITHUB_CANONICAL_HOST, &set),
        GITHUB_CANONICAL_HOST
    );
    for alias in GITHUB_HOST_ALIASES {
        assert!(set.contains(alias), "{alias} is declared and must fold");
        assert_eq!(fold_host(alias, &set), GITHUB_CANONICAL_HOST);
    }
    let gist = format!("gist.{GITHUB_CANONICAL_HOST}");
    assert!(!set.contains(&gist), "a gist is not a repository");
    assert_eq!(fold_host(&gist, &set), gist);
}

/// A provider whose host is not the canonical one declares no aliases, and the canonical host is
/// still a member of its own set — otherwise an Enterprise install would fold nothing, including
/// itself, and `contains` would answer `false` for the only host it has.
#[test]
fn a_provider_with_no_aliases_still_owns_its_canonical_host() {
    let forge = DeclaringForge {
        canonical: "forge.example",
        aliases: &[],
    };
    let set = HostAliases::from_provider(&forge);
    assert!(set.contains("forge.example"));
    assert_eq!(fold_host("forge.example", &set), "forge.example");
    assert_eq!(fold_host("ssh.forge.example", &set), "ssh.forge.example");
}
