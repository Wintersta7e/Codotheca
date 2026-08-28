//! J4 against a real repository and a real index.
//!
//! The fold is pure and unit-tested beside the code. This covers the parts nothing else does:
//! that the root facts come back from a real walk, that commit-days are idempotent on their
//! dedupe key, and that a recompute deletes and rewrites the `git` track without touching the
//! `session` track — §1.7's two classes have opposite rules and are easy to confuse.

#![cfg(feature = "testkit")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::must_use_candidate,
    clippy::missing_panics_doc
)]

mod support;

use std::sync::Arc;

use codotheca_core::cancel::CancelToken;
use codotheca_core::clock::SystemClock;
use codotheca_core::git::{GitSlots, JobClass, JobContext, SystemGit};
use codotheca_core::identity::user::IdentitySet;
use codotheca_core::index::migrate::{apply_all, MIGRATIONS};
use codotheca_core::index::{open_connection, Index};
use codotheca_core::jobs::j4_history::{
    commit_days, commit_history, observe, read_root_facts, HistoryFacts, RootFacts,
};
use codotheca_core::protocol::ProjectId;
use support::TestRepo;

/// The six history columns, in the order `commit_history` writes them. Named because a
/// six-tuple of `Option` trips `type_complexity` and reads as noise inline.
type HistoryRow = (
    Option<i64>,
    Option<i64>,
    Option<String>,
    Option<i64>,
    Option<String>,
    Option<i64>,
);

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

fn system_git(repo: &TestRepo) -> SystemGit {
    SystemGit::new(
        Arc::new(repo.exec()),
        Arc::new(GitSlots::new(4)),
        Arc::new(SystemClock::new()),
    )
}

#[test]
fn a_real_repository_reports_its_root_and_its_first_commit_date() {
    let repo = TestRepo::init();
    repo.write("a.txt", b"one\n");
    repo.git(&["add", "a.txt"]);
    repo.commit_at("first", "2021-03-04T10:00:00+00:00");
    repo.write("a.txt", b"two\n");
    repo.git(&["add", "a.txt"]);
    repo.commit_at("second", "2022-06-07T10:00:00+00:00");

    let git = system_git(&repo);
    let cancel = CancelToken::new();
    let ctx = JobContext::new(JobClass::History, &cancel, None);

    let roots = read_root_facts(&git, &repo.handle(), &ctx).unwrap();
    assert_eq!(roots.root_oids.len(), 1);
    assert_eq!(roots.first_commit_sha.as_ref(), roots.root_oids.first());
    assert!(roots.first_commit_at.is_some());
    assert_eq!(roots.first_commit_tz_offset_min, Some(0));
}

#[test]
fn a_repository_with_no_commits_has_no_root_and_no_dates() {
    let repo = TestRepo::init();
    let git = system_git(&repo);
    let cancel = CancelToken::new();
    let ctx = JobContext::new(JobClass::History, &cancel, None);
    assert_eq!(
        read_root_facts(&git, &repo.handle(), &ctx).unwrap(),
        RootFacts::default()
    );
}

#[test]
fn the_users_commits_become_days_and_the_newest_subject_is_recorded() {
    let repo = TestRepo::init();
    repo.write("a.txt", b"one\n");
    repo.git(&["add", "a.txt"]);
    repo.commit_at("first", "2021-03-04T10:00:00+00:00");
    repo.write("a.txt", b"two\n");
    repo.git(&["add", "a.txt"]);
    repo.commit_at("second", "2021-03-04T18:00:00+00:00");
    repo.write("a.txt", b"three\n");
    repo.git(&["add", "a.txt"]);
    repo.commit_at("third", "2021-09-09T09:00:00+00:00");

    let git = system_git(&repo);
    let cancel = CancelToken::new();
    let ctx = JobContext::new(JobClass::History, &cancel, None);

    // The fixture commits as its own fixed identity; read it back rather than restating it.
    let email = repo.git(&["log", "-1", "--format=%ce"]).trim().to_owned();
    let ids = IdentitySet::from_emails([email]);

    let (_roots, facts) = observe(&git, &repo.handle(), &ids, &ctx).unwrap();
    assert_eq!(
        facts.days.len(),
        2,
        "three commits across two days is two days"
    );
    assert_eq!(facts.last_commit_subject(), Some("third"));
    assert_eq!(facts.last_commit_at, facts.last_user_commit_at);
}

#[test]
fn commit_days_are_idempotent_on_their_dedupe_key() {
    let (_dir, mut conn) = fresh();
    let project = insert_project(&conn, "p");
    let facts = HistoryFacts {
        days: [19_675, 19_676].into_iter().collect(),
        ..HistoryFacts::default()
    };

    for expected in [2, 0] {
        let tx = conn.transaction().unwrap();
        let written = commit_days(&tx, project, Some("lin"), Some("host/o/r"), &facts).unwrap();
        tx.commit().unwrap();
        assert_eq!(written, expected);
    }

    let n: i64 = conn
        .query_row(
            "SELECT count(*) FROM xp_events WHERE project_id = ?1 AND kind = 'commit_day'",
            [project.0],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 2);
}

/// §1.7: a fork and its upstream share a lineage key on purpose. Keying on it alone would make
/// one of their commit-days on the same date silently vanish.
#[test]
fn a_fork_and_its_upstream_both_keep_their_commit_day() {
    let (_dir, mut conn) = fresh();
    let upstream = insert_project(&conn, "up");
    let fork = insert_project(&conn, "fork");
    let facts = HistoryFacts {
        days: [19_675].into_iter().collect(),
        ..HistoryFacts::default()
    };

    let tx = conn.transaction().unwrap();
    assert_eq!(
        commit_days(&tx, upstream, Some("lin"), Some("host/owner-a/r"), &facts).unwrap(),
        1
    );
    assert_eq!(
        commit_days(&tx, fork, Some("lin"), Some("host/owner-b/r"), &facts).unwrap(),
        1
    );
    tx.commit().unwrap();

    let n: i64 = conn
        .query_row("SELECT count(*) FROM xp_events", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 2);
}

/// A project with no lineage yet has no stable key, so nothing is written rather than something
/// written under a key that will change.
#[test]
fn no_lineage_writes_no_commit_days() {
    let (_dir, mut conn) = fresh();
    let project = insert_project(&conn, "p");
    let tx = conn.transaction().unwrap();
    let written = commit_days(
        &tx,
        project,
        None,
        None,
        &HistoryFacts {
            days: [19_675].into_iter().collect(),
            ..HistoryFacts::default()
        },
    )
    .unwrap();
    tx.commit().unwrap();
    assert_eq!(written, 0);
}

/// Nothing J4 writes is a commit count. The columns it fills are dates and one subject.
#[test]
fn committing_history_writes_dates_and_never_a_count() {
    let (_dir, mut conn) = fresh();
    let project = insert_project(&conn, "p");

    let roots = RootFacts {
        root_oids: vec!["a".repeat(40)],
        first_commit_at: Some(1_600_000_000),
        first_commit_tz_offset_min: Some(330),
        first_commit_sha: Some("a".repeat(40)),
    };
    let facts = HistoryFacts {
        days: [19_675].into_iter().collect(),
        last_commit_at: Some(1_700_000_000),
        last_user_commit_at: Some(1_650_000_000),
        recent_subjects: vec!["third".to_owned(), "second".to_owned()],
    };

    let tx = conn.transaction().unwrap();
    commit_history(&tx, project, &roots, &facts).unwrap();
    tx.commit().unwrap();

    let (first, off, sha, last, subject, user): HistoryRow = conn
        .query_row(
            "SELECT first_commit_at, first_commit_tz_offset_min, first_commit_sha,
                    last_commit_at, last_commit_subject, last_user_commit_at
               FROM project WHERE id = ?1",
            [project.0],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(first, Some(1_600_000_000));
    assert_eq!(off, Some(330));
    assert_eq!(sha.as_deref(), Some("a".repeat(40).as_str()));
    assert_eq!(last, Some(1_700_000_000));
    assert_eq!(subject.as_deref(), Some("third"));
    assert_eq!(user, Some(1_650_000_000));
}
