//! §22.7 — the rename repair. **AC-P2-22-13.**
//!
//! A repository that was renamed or transferred keeps its stable forge id and loses its path, so
//! the listing's key no longer equals the local clone's. One lookup of the stored path, once.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::sync::{Arc, Mutex};

use codotheca_core::accounts::keychain::SecretToken;
use codotheca_core::http::TransportError;
use codotheca_core::identity::alias::HostAliases;
use codotheca_core::identity::rename_repair::{repair_renames, RepairReport};
use codotheca_core::index::Index;
use codotheca_core::provider::listing::{OrgListing, Page, RepoListing, Viewer};
use codotheca_core::provider::{Observed, Provider, ProviderError, ProviderResult};

/// What a scripted lookup answers.
#[derive(Debug, Clone)]
enum Answer {
    Found(&'static str),
    NotFound,
    Forbidden,
    RateLimited,
    Offline,
}

/// A forge that records every lookup and answers from a script.
#[derive(Debug)]
struct RecordingForge {
    answer: Answer,
    calls: Mutex<Vec<(String, String)>>,
}

impl RecordingForge {
    fn new(answer: Answer) -> Self {
        Self {
            answer,
            calls: Mutex::new(Vec::new()),
        }
    }

    fn calls(&self) -> Vec<(String, String)> {
        self.calls
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    fn request_count(&self) -> usize {
        self.calls().len()
    }
}

impl Provider for RecordingForge {
    fn viewer(&self, _t: &SecretToken) -> ProviderResult<Observed<Viewer>> {
        unreachable!("the repair calls lookup_repo and nothing else")
    }
    fn list_orgs(
        &self,
        _t: &SecretToken,
        _cur: Option<&str>,
    ) -> ProviderResult<Observed<Page<OrgListing>>> {
        unreachable!("the repair calls lookup_repo and nothing else")
    }
    fn list_repos(
        &self,
        _t: &SecretToken,
        _cur: Option<&str>,
    ) -> ProviderResult<Observed<Page<RepoListing>>> {
        unreachable!("the repair calls lookup_repo and nothing else")
    }
    fn lookup_repo(
        &self,
        _t: &SecretToken,
        owner: &str,
        name: &str,
    ) -> ProviderResult<Observed<Option<RepoListing>>> {
        self.calls
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push((owner.to_owned(), name.to_owned()));
        let value = match self.answer {
            Answer::Found(id) => Some(RepoListing {
                provider: "github",
                provider_repo_id: id.to_owned(),
                // The forge answers with the CURRENT path, which is the whole point of the
                // repair: the stored key is the old one.
                clone_url: format!("https://forge.example/{owner}/renamed.git"),
                owner: owner.to_owned(),
                name: "renamed".to_owned(),
                can_push: Some(true),
                is_fork: false,
                fork_parent_clone_url: None,
                is_archived: false,
                is_private: false,
                in_org: None,
            }),
            Answer::NotFound => None,
            Answer::Forbidden => {
                return Err(ProviderError::Http {
                    status: 403,
                    headers: Vec::new(),
                })
            }
            Answer::RateLimited => {
                return Err(ProviderError::Http {
                    status: 429,
                    headers: vec![("retry-after".to_owned(), "60".to_owned())],
                })
            }
            Answer::Offline => {
                return Err(ProviderError::Transport(TransportError::Io {
                    detail: "no route to host".to_owned(),
                }))
            }
        };
        Ok(Observed {
            value,
            granted_scopes: None,
        })
    }
    fn canonical_host(&self) -> &'static str {
        "forge.example"
    }
    fn host_aliases(&self) -> &[&str] {
        &["forge.example", "www.forge.example", "ssh.forge.example"]
    }
}

fn aliases() -> HostAliases {
    HostAliases::from_provider(&RecordingForge::new(Answer::NotFound))
}

fn token() -> SecretToken {
    SecretToken::new("rename-repair-token".to_owned())
}

/// An index with `keys.len()` projects, each carrying a `remote_key` and no binding.
fn index_with(keys: &[&str]) -> (tempfile::TempDir, Arc<Mutex<Index>>) {
    let dir = tempfile::tempdir().unwrap();
    let index = Index::open_at(dir.path(), 0).unwrap();
    for (i, key) in keys.iter().enumerate() {
        let name = format!("project-{i}");
        index
            .conn()
            .execute(
                "INSERT INTO project (name, seed_basename, remote_key, created_at, updated_at)
                 VALUES (?1, ?1, ?2, ?3, ?3)",
                rusqlite::params![name, key, 100 + i64::try_from(i).unwrap()],
            )
            .unwrap();
    }
    (dir, Arc::new(Mutex::new(index)))
}

fn binding_of(index: &Arc<Mutex<Index>>, project_id: i64) -> (Option<String>, Option<String>) {
    let guard = index.lock().unwrap();
    guard
        .conn()
        .query_row(
            "SELECT provider_repo_id, remote_link_basis FROM project WHERE id = ?1",
            [project_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap()
}

fn remote_key_of(index: &Arc<Mutex<Index>>, project_id: i64) -> String {
    let guard = index.lock().unwrap();
    guard
        .conn()
        .query_row(
            "SELECT remote_key FROM project WHERE id = ?1",
            [project_id],
            |r| r.get(0),
        )
        .unwrap()
}

/// **AC-P2-22-13.** N unmatched projects produce at most N lookups, and then zero.
#[test]
fn n_unmatched_projects_produce_at_most_n_lookups_and_then_zero() {
    let keys = [
        "forge.example/acme/one",
        "forge.example/acme/two",
        "forge.example/acme/three",
    ];
    let (_dir, index) = index_with(&keys);
    let forge = RecordingForge::new(Answer::Found("909"));

    let first = repair_renames(&index, &forge, &token(), &aliases(), 500).unwrap();
    let after_first = forge.request_count();
    let second = repair_renames(&index, &forge, &token(), &aliases(), 600).unwrap();
    let after_second = forge.request_count() - after_first;

    eprintln!(
        "rename repair: {} project(s), {after_first} lookup(s) on the first run, \
         {after_second} on the second",
        keys.len()
    );
    assert!(after_first > 0, "a run that issued nothing proves nothing");
    assert!(
        after_first <= keys.len(),
        "one request per unmatched key, at most"
    );
    assert_eq!(after_second, 0, "a resolved project is never asked again");
    assert_eq!(
        first,
        RepairReport {
            attempted: 3,
            resolved: 3,
            unknown: 0
        }
    );
    assert_eq!(second, RepairReport::default());
}

#[test]
fn a_403_a_429_and_an_offline_lookup_each_leave_the_fields_unknown() {
    for answer in [Answer::Forbidden, Answer::RateLimited, Answer::Offline] {
        let (_dir, index) = index_with(&["forge.example/acme/one"]);
        let forge = RecordingForge::new(answer.clone());
        let report = repair_renames(&index, &forge, &token(), &aliases(), 500)
            .unwrap_or_else(|e| panic!("{answer:?} must not be an error: {e:?}"));

        assert_eq!(
            report,
            RepairReport {
                attempted: 1,
                resolved: 0,
                unknown: 1
            },
            "{answer:?}"
        );
        assert_eq!(
            binding_of(&index, 1),
            (None, None),
            "{answer:?} left a value behind"
        );
        assert_eq!(
            forge.request_count(),
            1,
            "{answer:?} was retried inside the run"
        );
    }
}

/// The `owner` and `name` come from the **stored `remote_key`**, never from `project.owner` and
/// `project.name` — a display name and a directory basename, and neither is a forge path.
#[test]
fn the_lookup_uses_the_stored_remote_key_not_the_display_name() {
    let (_dir, index) = index_with(&["forge.example/acme/widget"]);
    {
        let guard = index.lock().unwrap();
        guard
            .conn()
            .execute(
                "UPDATE project SET name = 'widget-2', owner = 'a display name' WHERE id = 1",
                [],
            )
            .unwrap();
    }
    let forge = RecordingForge::new(Answer::Found("909"));
    repair_renames(&index, &forge, &token(), &aliases(), 500).unwrap();

    assert_eq!(
        forge.calls(),
        vec![("acme".to_owned(), "widget".to_owned())]
    );
}

#[test]
fn a_resolved_project_takes_the_provider_id_basis_and_keeps_its_remote_key() {
    let (_dir, index) = index_with(&["forge.example/acme/widget"]);
    let forge = RecordingForge::new(Answer::Found("909"));
    repair_renames(&index, &forge, &token(), &aliases(), 500).unwrap();

    assert_eq!(
        binding_of(&index, 1),
        (Some("909".to_owned()), Some("provider_id".to_owned()))
    );
    assert_eq!(
        remote_key_of(&index, 1),
        "forge.example/acme/widget",
        "the repair never rewrites remote_key, even when the forge answered a new path"
    );
}

/// A host no adapter declares is not this provider's to ask about, and the repair must not spend
/// a request on it.
#[test]
fn a_project_on_an_undeclared_host_is_never_looked_up() {
    let (_dir, index) = index_with(&["other.example/acme/widget", "forge-work/acme/widget"]);
    let forge = RecordingForge::new(Answer::Found("909"));
    let report = repair_renames(&index, &forge, &token(), &aliases(), 500).unwrap();

    assert_eq!(report, RepairReport::default());
    assert_eq!(forge.request_count(), 0);
}

/// A clone taken over a declared alias host still belongs to this provider.
#[test]
fn a_project_on_an_alias_host_is_looked_up() {
    let (_dir, index) = index_with(&["ssh.forge.example/acme/widget"]);
    let forge = RecordingForge::new(Answer::Found("909"));
    repair_renames(&index, &forge, &token(), &aliases(), 500).unwrap();
    assert_eq!(
        forge.calls(),
        vec![("acme".to_owned(), "widget".to_owned())]
    );
}

#[test]
fn a_project_that_already_carries_an_id_is_not_asked_about() {
    let (_dir, index) = index_with(&["forge.example/acme/widget"]);
    {
        let guard = index.lock().unwrap();
        guard
            .conn()
            .execute(
                "UPDATE project SET provider = 'github', provider_repo_id = '7',
                                    remote_link_basis = 'remote_key' WHERE id = 1",
                [],
            )
            .unwrap();
    }
    let forge = RecordingForge::new(Answer::Found("909"));
    let report = repair_renames(&index, &forge, &token(), &aliases(), 500).unwrap();
    assert_eq!(report, RepairReport::default());
    assert_eq!(forge.request_count(), 0);
}
