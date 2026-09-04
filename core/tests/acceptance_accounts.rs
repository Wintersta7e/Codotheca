#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use codotheca_core::protocol::{Affiliation, ScopeTier};
use codotheca_core::provider::admit::{admit, Admission};
use codotheca_core::provider::listing::RepoListing;

const PROVIDER: &str = "forge.example.invalid";

fn key(id: &str) -> (String, String) {
    (PROVIDER.to_owned(), id.to_owned())
}

fn orgs(logins: &[&str]) -> BTreeSet<String> {
    logins.iter().map(|login| (*login).to_owned()).collect()
}

fn repo(id: &str, owner: &str, name: &str, can_push: Option<bool>) -> RepoListing {
    RepoListing {
        provider: PROVIDER,
        provider_repo_id: id.to_owned(),
        clone_url: format!("https://forge.example.invalid/{owner}/{name}.git"),
        owner: owner.to_owned(),
        name: name.to_owned(),
        can_push,
        is_fork: false,
        fork_parent_clone_url: None,
        is_archived: false,
        is_private: false,
        in_org: None,
    }
}

fn private_repo(id: &str, owner: &str, can_push: Option<bool>) -> RepoListing {
    let mut listing = repo(id, owner, id, can_push);
    listing.is_private = true;
    listing
}

fn fork_repo(id: &str, owner: &str, can_push: Option<bool>) -> RepoListing {
    let mut listing = repo(id, owner, id, can_push);
    listing.is_fork = true;
    listing.fork_parent_clone_url =
        Some(format!("https://forge.example.invalid/upstream/{id}.git"));
    listing
}

fn org_repo(id: &str, org: &str, can_push: Option<bool>) -> RepoListing {
    let mut listing = repo(id, org, id, can_push);
    listing.in_org = Some(org.to_owned());
    listing
}

fn admitted_ids(admission: &Admission) -> Vec<&str> {
    admission
        .admitted
        .keys()
        .map(|(_, provider_repo_id)| provider_repo_id.as_str())
        .collect()
}

fn assert_accounted(admission: &Admission, listing_count: usize) {
    let accounted = admission.admitted.len()
        + admission.skipped_unknown_permission
        + admission.skipped_no_push
        + admission.skipped_private_under_public_tier
        + admission.skipped_org_not_enabled;
    assert_eq!(
        accounted, listing_count,
        "admission outcomes must account for every non-duplicate listing"
    );
}

#[test]
fn ac_p2_20_1_admission_is_push_permission() {
    let listings = vec![
        repo("01-owned-public", "case-viewer", "owned-public", Some(true)),
        private_repo("02-owned-private", "case-viewer", Some(true)),
        fork_repo("03-owned-fork", "case-viewer", Some(true)),
        repo("04-push-collab", "sample-user", "push-collab", Some(true)),
        repo("05-read-only", "sample-user", "read-only", Some(false)),
        repo("06-starred", "sample-user", "starred", Some(false)),
        repo("07-unknown", "sample-user", "unknown", None),
    ];

    let admission = admit(
        &listings,
        "case-viewer",
        ScopeTier::Private,
        &BTreeSet::new(),
    );

    assert_eq!(admission.admitted.len(), 4);
    assert_eq!(
        admitted_ids(&admission),
        [
            "01-owned-public",
            "02-owned-private",
            "03-owned-fork",
            "04-push-collab",
        ]
    );
    assert!(!admission.admitted.contains_key(&key("05-read-only")));
    assert!(!admission.admitted.contains_key(&key("06-starred")));
    assert!(!admission.admitted.contains_key(&key("07-unknown")));
    assert_eq!(
        admission.skipped_unknown_permission,
        1,
        "missing permission must stay unknown; admitted count was {}",
        admission.admitted.len()
    );
    assert_eq!(admission.skipped_no_push, 2);
    assert_eq!(admission.skipped_private_under_public_tier, 0);
    assert_eq!(admission.skipped_org_not_enabled, 0);
    assert_accounted(&admission, listings.len());
}

#[test]
fn duplicate_provider_repo_key_collapses_to_one_admission() {
    let first = repo("shared-remote", "case-viewer", "shared-remote", Some(true));
    let mut second = repo("shared-remote", "case-viewer", "shared-remote", Some(true));
    second.clone_url = "https://forge.example.invalid/CASE-VIEWER/shared-remote.git".to_owned();

    let admission = admit(
        &[first, second],
        "case-viewer",
        ScopeTier::Private,
        &BTreeSet::new(),
    );

    assert_eq!(admission.admitted.len(), 1);
    assert!(admission.admitted.contains_key(&key("shared-remote")));
}

#[test]
fn org_repos_require_enabled_org_and_push_permission() {
    let push_repo = org_repo("org-push", "sample-org", Some(true));
    let read_repo = org_repo("org-read", "sample-org", Some(false));

    let disabled = admit(
        std::slice::from_ref(&push_repo),
        "case-viewer",
        ScopeTier::Private,
        &BTreeSet::new(),
    );
    assert_eq!(
        disabled.skipped_org_not_enabled, 1,
        "disabled org entries must be counted before permission checks"
    );
    assert_eq!(disabled.admitted.len(), 0);
    assert_eq!(disabled.skipped_no_push, 0);

    let enabled = admit(
        &[push_repo, read_repo],
        "case-viewer",
        ScopeTier::Private,
        &orgs(&["sample-org"]),
    );
    assert_eq!(admitted_ids(&enabled), ["org-push"]);
    assert_eq!(enabled.skipped_org_not_enabled, 0);
    assert_eq!(enabled.skipped_no_push, 1);
    assert_eq!(
        enabled.admitted[&key("org-push")].affiliation,
        Affiliation::OrganizationMember
    );
}

#[test]
fn archived_repo_is_admitted_and_remote_flag_is_carried() {
    let mut listing = repo("archived", "case-viewer", "archived", Some(true));
    listing.is_archived = true;

    let admission = admit(
        &[listing],
        "case-viewer",
        ScopeTier::Private,
        &BTreeSet::new(),
    );

    let admitted = admission
        .admitted
        .get(&key("archived"))
        .expect("archived remote is admitted");
    assert!(admitted.listing.is_archived);
}

#[test]
fn private_repo_requires_private_tier() {
    let listing = private_repo("private-only", "case-viewer", Some(true));

    let public = admit(
        std::slice::from_ref(&listing),
        "case-viewer",
        ScopeTier::Public,
        &BTreeSet::new(),
    );
    assert_eq!(public.admitted.len(), 0);
    assert_eq!(public.skipped_private_under_public_tier, 1);
    assert_eq!(public.skipped_no_push, 0);

    let private = admit(
        &[listing],
        "case-viewer",
        ScopeTier::Private,
        &BTreeSet::new(),
    );
    assert_eq!(admitted_ids(&private), ["private-only"]);
}

#[test]
fn owned_fork_is_admitted_and_fork_state_is_carried() {
    let listing = fork_repo("owned-fork", "case-viewer", Some(true));

    let admission = admit(
        &[listing],
        "case-viewer",
        ScopeTier::Private,
        &BTreeSet::new(),
    );

    let admitted = admission
        .admitted
        .get(&key("owned-fork"))
        .expect("owned fork is admitted");
    assert!(admitted.listing.is_fork);
    assert_eq!(
        admitted.listing.fork_parent_clone_url,
        Some("https://forge.example.invalid/upstream/owned-fork.git".to_owned())
    );
}

#[test]
fn affiliation_uses_case_insensitive_viewer_and_org_precedence() {
    let owner = repo("owner-case", "Case-Viewer", "owner-case", Some(true));
    let org = org_repo("org-member", "sample-org", Some(true));

    let admission = admit(
        &[owner, org],
        "case-viewer",
        ScopeTier::Private,
        &orgs(&["sample-org"]),
    );

    assert_eq!(
        admission.admitted[&key("owner-case")].affiliation,
        Affiliation::Owner
    );
    assert_eq!(
        admission.admitted[&key("org-member")].affiliation,
        Affiliation::OrganizationMember
    );
}

#[test]
fn accounting_identity_covers_mixed_non_duplicate_listings() {
    let listings = vec![
        repo(
            "accounted-owned",
            "case-viewer",
            "accounted-owned",
            Some(true),
        ),
        repo(
            "accounted-read",
            "sample-user",
            "accounted-read",
            Some(false),
        ),
        repo(
            "accounted-unknown",
            "sample-user",
            "accounted-unknown",
            None,
        ),
        private_repo("accounted-private", "case-viewer", Some(true)),
        org_repo("accounted-org", "sample-org", Some(true)),
    ];

    let admission = admit(
        &listings,
        "case-viewer",
        ScopeTier::Public,
        &BTreeSet::new(),
    );

    assert_eq!(admission.admitted.len(), 1);
    assert_eq!(admission.skipped_no_push, 1);
    assert_eq!(admission.skipped_unknown_permission, 1);
    assert_eq!(admission.skipped_private_under_public_tier, 1);
    assert_eq!(admission.skipped_org_not_enabled, 1);
    assert_accounted(&admission, listings.len());
}

fn source_root(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join(name)
}

fn relative_to_source(path: &Path) -> String {
    path.strip_prefix(Path::new(env!("CARGO_MANIFEST_DIR")).join("src"))
        .unwrap_or(path)
        .display()
        .to_string()
}

fn rust_sources_under(dir: &Path, out: &mut Vec<(PathBuf, String)>) {
    for entry in std::fs::read_dir(dir).expect("source directory is readable") {
        let entry = entry.expect("a readable source entry");
        let kind = entry.file_type().expect("a source entry kind");
        let path = entry.path();
        if kind.is_dir() {
            rust_sources_under(&path, out);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            match std::fs::read_to_string(&path) {
                Ok(text) => out.push((path, text)),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => panic!("{}: {error}", path.display()),
            }
        }
    }
}

fn strip_comment_lines(source: &str) -> String {
    source
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn remote_archived_fact_is_never_written_to_project_archived_flag() {
    let mut sources = Vec::new();
    rust_sources_under(&source_root("provider"), &mut sources);
    rust_sources_under(&source_root("accounts"), &mut sources);
    eprintln!(
        "acceptance_accounts: archival write gate scanned {} source file(s)",
        sources.len()
    );
    assert!(
        !sources.is_empty(),
        "the archival write gate read no files, so it proved nothing"
    );

    let mut offenders = Vec::new();
    for (path, source) in sources {
        let code = strip_comment_lines(&source);
        let squashed = code.split_whitespace().collect::<Vec<_>>().join(" ");
        if squashed.contains("UPDATE project") && squashed.contains("is_archived") {
            offenders.push(format!(
                "{} names is_archived in an UPDATE project statement",
                relative_to_source(&path)
            ));
        }
        if code.contains("projects.setFlags") {
            offenders.push(format!(
                "{} calls projects.setFlags",
                relative_to_source(&path)
            ));
        }
    }

    assert!(
        offenders.is_empty(),
        "remote archival state must not write project.is_archived: {offenders:?}"
    );
}

/// Org logins are case-insensitive on the forge, so `enabled_orgs` must match that way too. A
/// case-sensitive lookup silently excludes every repository in an org the user had enabled, and
/// the exclusion looks exactly like a correct org gate from the outside.
#[test]
fn an_enabled_org_matches_regardless_of_case() {
    let listings = [org_repo("1", "an-org", Some(true))];
    let admission = admit(&listings, "someone", ScopeTier::Private, &orgs(&["An-Org"]));
    assert_eq!(
        admitted_ids(&admission),
        ["1"],
        "a case difference excluded an enabled org"
    );
    assert_eq!(admission.skipped_org_not_enabled, 0);
    assert_accounted(&admission, listings.len());
}
