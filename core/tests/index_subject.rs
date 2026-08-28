#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use codotheca_core::index::migrate::{apply_all, MIGRATIONS};
use codotheca_core::index::path::{PathPlatform, StoredPath};
use codotheca_core::index::subject::{resolve_subject, subject_for_project, ProjectSubject};
use codotheca_core::index::{open_connection, Index};
use codotheca_core::protocol::ProjectId;

fn fresh() -> (tempfile::TempDir, rusqlite::Connection) {
    let dir = tempfile::tempdir().unwrap();
    let mut conn = open_connection(&Index::db_path(dir.path())).unwrap();
    apply_all(&mut conn, MIGRATIONS).unwrap();
    (dir, conn)
}

fn project(
    conn: &rusqlite::Connection,
    name: &str,
    lineage: Option<&str>,
    remote: Option<&str>,
) -> ProjectId {
    conn.execute(
        "INSERT INTO project (name, seed_basename, lineage_key, remote_key, created_at, updated_at)
         VALUES (?1, ?1, ?2, ?3, 1, 1)",
        rusqlite::params![name, lineage, remote],
    )
    .unwrap();
    ProjectId(conn.last_insert_rowid())
}

fn location(conn: &rusqlite::Connection, p: ProjectId, raw: &[u8]) {
    let sp = StoredPath::from_bytes(raw.to_vec(), PathPlatform::Unix);
    let (bytes, key, display) = sp.as_params();
    conn.execute(
        "INSERT INTO location
           (project_id, kind, path_bytes, path_key, path_display, volume_key, store_key,
            presence, repo_kind)
         VALUES (?1, 'linux', ?2, ?3, ?4, 'vol', 'store', 'present', 'worktree')",
        rusqlite::params![p.0, bytes, key, display],
    )
    .unwrap();
}

#[test]
fn keys_round_trip_through_parse() {
    let cases = [
        ProjectSubject::Lineage {
            lineage_key: "abc123".into(),
            remote_key: Some("host/owner/name".into()),
        },
        ProjectSubject::Lineage {
            lineage_key: "abc123".into(),
            remote_key: None,
        },
        ProjectSubject::Path {
            kind: "linux".into(),
            distro: String::new(),
            path_key: b"/home/u/thing".to_vec(),
        },
        ProjectSubject::Path {
            kind: "wsl".into(),
            distro: "a-distro".into(),
            path_key: b"/home/u/pro\xffject".to_vec(),
        },
    ];
    for c in cases {
        let key = c.to_key();
        assert_eq!(
            ProjectSubject::parse(&key),
            Some(c.clone()),
            "key was {key}"
        );
    }
}

#[test]
fn the_key_strings_are_the_documented_shapes() {
    assert_eq!(
        ProjectSubject::Lineage {
            lineage_key: "abc".into(),
            remote_key: Some("h/o/n".into())
        }
        .to_key(),
        "lineage:abc|remote:h/o/n"
    );
    assert_eq!(
        ProjectSubject::Lineage {
            lineage_key: "abc".into(),
            remote_key: None
        }
        .to_key(),
        "lineage:abc|remote:"
    );
    assert_eq!(
        ProjectSubject::Path {
            kind: "linux".into(),
            distro: String::new(),
            path_key: b"/a".to_vec()
        }
        .to_key(),
        "path:linux::2f61"
    );
}

#[test]
fn a_fork_and_its_upstream_do_not_share_a_subject() {
    let (_d, conn) = fresh();
    let upstream = project(&conn, "up", Some("shared"), Some("h/upstream/n"));
    let fork = project(&conn, "fork", Some("shared"), Some("h/fork/n"));

    let a = subject_for_project(&conn, upstream).unwrap().unwrap();
    let b = subject_for_project(&conn, fork).unwrap().unwrap();
    assert_ne!(
        a.to_key(),
        b.to_key(),
        "§1.7: the remote component is what separates a fork from its upstream"
    );
    assert_eq!(resolve_subject(&conn, &a).unwrap(), Some(upstream));
    assert_eq!(resolve_subject(&conn, &b).unwrap(), Some(fork));
}

#[test]
fn a_project_with_no_commits_is_keyed_on_its_path() {
    let (_d, conn) = fresh();
    let p = project(&conn, "empty", None, None);
    location(&conn, p, b"/home/u/empty");

    let s = subject_for_project(&conn, p).unwrap().unwrap();
    match &s {
        ProjectSubject::Path { kind, distro, .. } => {
            assert_eq!(kind, "linux");
            assert_eq!(distro, "");
        }
        ProjectSubject::Lineage { .. } => panic!("expected a path subject, got {s:?}"),
    }
    assert_eq!(resolve_subject(&conn, &s).unwrap(), Some(p));
}

#[test]
fn a_subject_survives_ids_being_reassigned_by_a_rebuild() {
    let (_d, conn) = fresh();
    let before = project(&conn, "thing", Some("lineage-1"), Some("h/o/n"));
    let subject = subject_for_project(&conn, before).unwrap().unwrap();
    let key = subject.to_key();

    // A rebuild: everything is re-derived from disk and the ids come out different.
    conn.execute("DELETE FROM project", []).unwrap();
    for _ in 0..3 {
        project(&conn, "filler", Some("other"), None);
    }
    let after = project(&conn, "thing", Some("lineage-1"), Some("h/o/n"));
    assert_ne!(before, after, "the fixture must actually reassign the id");

    let parsed = ProjectSubject::parse(&key).unwrap();
    assert_eq!(resolve_subject(&conn, &parsed).unwrap(), Some(after));
}

#[test]
fn a_subject_with_no_match_resolves_to_nothing_rather_than_to_the_first_row() {
    let (_d, conn) = fresh();
    project(&conn, "thing", Some("lineage-1"), Some("h/o/n"));
    let orphan = ProjectSubject::Lineage {
        lineage_key: "lineage-9".into(),
        remote_key: None,
    };
    assert_eq!(resolve_subject(&conn, &orphan).unwrap(), None);
}
