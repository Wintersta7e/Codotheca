//! J1.5's two ends: the identity set read out of the index, and the census written back into it.
//!
//! `tally` is pure and unit-tested beside itself. This file covers `load_identity_set`,
//! `census` over the real backend seam, and `persist` — including §5.5's `is_reference`
//! derivation, which happens in exactly one place and is what §4.1a's whole ordering exists for.

#![cfg(feature = "testkit")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::must_use_candidate,
    clippy::missing_panics_doc
)]

use codotheca_core::cancel::CancelToken;
use codotheca_core::git::{Authorship, CommitterTally, JobClass, JobContext, RepoHandle, StoreKey};
use codotheca_core::identity::user::{load_identity_set, IdentitySet};
use codotheca_core::index::migrate::{apply_all, MIGRATIONS};
use codotheca_core::index::{open_connection, Index};
use codotheca_core::jobs::j15_authorship::{census, persist, AuthorshipFacts};
use codotheca_core::mount::StoreClass;
use codotheca_core::protocol::ProjectId;
use codotheca_core::testing::{FakeGitBackend, GitReply};

fn fresh() -> (tempfile::TempDir, rusqlite::Connection) {
    let dir = tempfile::tempdir().unwrap();
    let mut conn = open_connection(&Index::db_path(dir.path())).unwrap();
    apply_all(&mut conn, MIGRATIONS).unwrap();
    (dir, conn)
}

fn insert_project(conn: &rusqlite::Connection, name: &str) -> ProjectId {
    conn.execute(
        "INSERT INTO project (name, seed_basename, created_at, updated_at) VALUES (?1, ?1, 0, 0)",
        [name],
    )
    .unwrap();
    ProjectId(conn.last_insert_rowid())
}

fn handle() -> RepoHandle {
    RepoHandle::bare(
        std::path::Path::new("/w/repo"),
        StoreKey::new("s"),
        StoreClass::Local,
    )
}

fn tally(email: &str, commits: u32) -> CommitterTally {
    CommitterTally {
        email: email.to_owned(),
        commits,
        days: std::collections::BTreeSet::new(),
        last_commit_at: 1_700_000_000,
    }
}

#[test]
fn the_identity_set_is_confirmed_identities_and_their_aliases() {
    let (_dir, conn) = fresh();
    conn.execute(
        "INSERT INTO identity (email, source, is_user) VALUES ('Me@Example.test', 'gitconfig', 1)",
        [],
    )
    .unwrap();
    let me = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO identity (email, source, is_user) VALUES ('other@example.test', 'manual', 0)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO identity_alias (identity_id, email, reason)
         VALUES (?1, '9999+me@users.noreply.test', 'local_part')",
        [me],
    )
    .unwrap();

    let set = load_identity_set(&conn).unwrap();
    assert!(set.contains("me@example.test"));
    assert!(set.contains("ME@EXAMPLE.TEST"), "matching is case-folded");
    assert!(
        set.contains("9999+me@users.noreply.test"),
        "an alias is the user too"
    );
    assert!(
        !set.contains("other@example.test"),
        "is_user = 0 is somebody else"
    );
}

#[test]
fn an_index_with_no_identities_yields_an_empty_set() {
    let (_dir, conn) = fresh();
    assert!(load_identity_set(&conn).unwrap().is_empty());
}

#[test]
fn the_census_reads_the_full_committer_walk_through_the_backend() {
    let git = FakeGitBackend::new();
    git.always_authorship(GitReply::Ok(Authorship {
        committers: vec![tally("me@x", 2), tally("them@x", 5)],
    }));
    let cancel = CancelToken::new();
    let ctx = JobContext::new(JobClass::Background, &cancel, None);

    let facts = census(
        &git,
        &handle(),
        &IdentitySet::from_emails(["me@x".to_owned()]),
        &ctx,
    )
    .unwrap();
    assert_eq!(facts.authored_by_user, Some(true));
    assert_eq!(
        facts.committers,
        vec![("them@x".to_owned(), 5), ("me@x".to_owned(), 2)]
    );
}

/// A backend that cannot answer must not be read as "nobody committed here": that would classify
/// the repository Reference on a failure.
#[test]
fn a_failed_walk_is_an_error_not_an_empty_census() {
    let git = FakeGitBackend::new();
    let cancel = CancelToken::new();
    let ctx = JobContext::new(JobClass::Background, &cancel, None);
    assert!(census(&git, &handle(), &IdentitySet::default(), &ctx).is_err());
}

#[test]
fn persisting_a_census_derives_is_reference_from_authorship() {
    let (_dir, mut conn) = fresh();
    let project = insert_project(&conn, "p");

    let tx = conn.transaction().unwrap();
    persist(
        &tx,
        project,
        &AuthorshipFacts {
            committers: vec![("them@x".to_owned(), 5)],
            authored_by_user: Some(false),
        },
    )
    .unwrap();
    tx.commit().unwrap();

    let (authored, reference): (Option<i64>, i64) = conn
        .query_row(
            "SELECT authored_by_user, is_reference FROM project WHERE id = ?1",
            [project.0],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(authored, Some(0));
    assert_eq!(reference, 1);
}

/// Not computed writes neither column. Otherwise a repository whose census has not run yet would
/// be indistinguishable from one that ran and found somebody else's code.
#[test]
fn a_not_computed_census_leaves_authorship_and_reference_alone() {
    let (_dir, mut conn) = fresh();
    let project = insert_project(&conn, "p");

    let tx = conn.transaction().unwrap();
    persist(&tx, project, &AuthorshipFacts::default()).unwrap();
    tx.commit().unwrap();

    let (authored, reference): (Option<i64>, i64) = conn
        .query_row(
            "SELECT authored_by_user, is_reference FROM project WHERE id = ?1",
            [project.0],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(authored, None, "NULL is not computed, never 0");
    assert_eq!(reference, 0);
}

/// §5.5 recomputes from history, so a second census replaces the rows rather than adding to them.
#[test]
fn a_second_census_replaces_the_committer_rows() {
    let (_dir, mut conn) = fresh();
    let project = insert_project(&conn, "p");

    for facts in [
        AuthorshipFacts {
            committers: vec![("a@x".to_owned(), 1), ("b@x".to_owned(), 1)],
            authored_by_user: Some(true),
        },
        AuthorshipFacts {
            committers: vec![("a@x".to_owned(), 4)],
            authored_by_user: Some(true),
        },
    ] {
        let tx = conn.transaction().unwrap();
        persist(&tx, project, &facts).unwrap();
        tx.commit().unwrap();
    }

    let rows: Vec<(String, i64)> = conn
        .prepare(
            "SELECT email, commits FROM project_committer WHERE project_id = ?1 ORDER BY email",
        )
        .unwrap()
        .query_map([project.0], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(rows, vec![("a@x".to_owned(), 4)]);
}
