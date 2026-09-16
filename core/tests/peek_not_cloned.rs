#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! §25.3a — `Space` on a **not-cloned** row, from the core's side.
//!
//! Peek renders only what is actually known, which is a **different set** from §8.4.1's five
//! facts and not those five with dashes in them. What the core owes the surface is the absence
//! itself — no location, no commits, no README promise — plus §25.3a's positive half.

use codotheca_core::art::testsupport::CollectingSink;
use codotheca_core::index::Index;
use codotheca_core::projects::peek::load_peek;
use codotheca_core::projects::ProjectsCtx;
use codotheca_core::protocol::{ProjectId, ReadmeStateKind, RemoteFactsState, RemoteVisibility};

const NOW: i64 = 1_781_179_200;

/// A project with a remote and **no location** — §23's shape, which is the only way this row
/// exists at all.
fn not_cloned() -> (tempfile::TempDir, Index) {
    let dir = tempfile::tempdir().expect("tempdir");
    let index = Index::open(dir.path()).expect("open");
    index
        .conn()
        .execute(
            "INSERT INTO project (id, name, seed_basename, remote_key, provider,
                                  provider_repo_id, remote_link_basis, last_touched_at,
                                  created_at, updated_at)
             VALUES (1, 'widget', 'widget', 'github.com/acme/widget', 'github', '909',
                     'provider_id', ?1, ?1, ?1)",
            rusqlite::params![NOW],
        )
        .expect("seed project");
    (dir, index)
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

fn observed_facts(index: &Index) {
    index
        .conn()
        .execute(
            "INSERT INTO remote_repo
               (provider, provider_repo_id, visibility, stars, open_issues, open_prs,
                permitted, observed_at)
             VALUES ('github', '909', 'public', 41, 7, 2, 1, ?1)",
            rusqlite::params![NOW - 60],
        )
        .expect("facts");
}

fn peek_of(index: &Index, project: i64) -> codotheca_core::protocol::Peek {
    let sink = CollectingSink::default();
    let jobs = codotheca_core::jobs::NullJobSink;
    let mounts = codotheca_core::testing::FakeMountResolver::default();
    let ctx = ProjectsCtx {
        index,
        events: &sink,
        jobs: &jobs,
        mounts: &mounts,
        now: NOW,
        tz_offset_min: 0,
    };
    load_peek(&ctx, ProjectId(project)).expect("peek")
}

#[test]
fn a_not_cloned_row_carries_no_location_and_no_commit_history() {
    let (_dir, index) = not_cloned();
    let peek = peek_of(&index, 1);
    assert!(peek.location.is_none(), "there is no path and no copy");
    assert!(peek.commits.is_empty());
    assert_eq!(peek.birth_year, None);
    assert_eq!(peek.size_tracked_bytes, None);
    assert_eq!(peek.last_commit_at, None);
    assert_eq!(peek.worktree.observed_at, None);
    assert_eq!(peek.worktree.is_dirty, None);
}

/// §23.3: *"for a not-cloned project J6 can never look, so `not_indexed` is a promise the app
/// cannot keep"*, and `ReadmeStateKind` gains no fourth variant. The surface renders **no README
/// element at all** for this row, so the value reaches no sentence.
#[test]
fn a_not_cloned_row_never_promises_a_readme_pass() {
    let (_dir, index) = not_cloned();
    let peek = peek_of(&index, 1);
    assert_ne!(
        peek.readme.state,
        ReadmeStateKind::NotIndexed,
        "a promise no pass can keep"
    );
    assert_eq!(peek.readme.text, None);
    assert_eq!(peek.readme.read_at, None);
}

/// §25.3a's **positive half**, and the thing that makes the row worth opening at all.
#[test]
fn a_not_cloned_row_answers_the_facts_it_does_have() {
    let (_dir, index) = not_cloned();
    connect_account(&index);
    observed_facts(&index);

    let remote = peek_of(&index, 1).remote.expect("a key produces facts");
    assert_eq!(remote.key, "github.com/acme/widget");
    assert_eq!(remote.state, RemoteFactsState::Observed);
    assert_eq!(remote.visibility, Some(RemoteVisibility::Public));
    assert_eq!(remote.stars, Some(41));
    assert_eq!(remote.open_issues, Some(7));
    assert_eq!(remote.open_prs, Some(2));
    assert_eq!(remote.observed_at, Some(NOW - 60));
}

/// NULL **iff** `remote_key` is NULL — the same predicate `ProjectDetail.remote` carries, from
/// the same producer.
#[test]
fn a_project_with_no_remote_key_answers_no_facts_on_peek_either() {
    let (_dir, index) = not_cloned();
    index
        .conn()
        .execute("UPDATE project SET remote_key = NULL WHERE id = 1", [])
        .expect("clear key");
    assert!(peek_of(&index, 1).remote.is_none());
}

/// A **cloned** project's Peek gains the same field, and that is not a regression: the five
/// facts are unchanged for a row with a location.
#[test]
fn a_cloned_row_keeps_its_five_facts_and_gains_the_same_field() {
    let (_dir, index) = not_cloned();
    connect_account(&index);
    observed_facts(&index);
    index
        .conn()
        .execute(
            "INSERT INTO location (id, project_id, kind, distro, path_bytes, path_key,
                                   path_display, volume_key, store_key, presence, repo_kind)
             VALUES (1, 1, 'linux', '', X'2f77', X'2f77', '<project-path>', 'v', 's',
                     'present', 'worktree')",
            [],
        )
        .expect("location");

    let peek = peek_of(&index, 1);
    assert!(peek.location.is_some());
    assert!(peek.remote.is_some(), "a cloned row carries the field too");
}
