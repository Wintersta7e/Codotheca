//! The fixtures the `debt_day` payout-site tests share: one project per site, built the way its
//! production caller finds it, and the scripted git and forge answers that move each site to a
//! closure.

use std::fmt::Write as _;
use std::path::Path;
use std::sync::Mutex;

use codotheca_core::advisories::lockfiles::read_lockfiles;
use codotheca_core::git::TreeEntry;
use codotheca_core::health::switches::write_switches;
use codotheca_core::http::HttpResponse;
use codotheca_core::index::Index;
use codotheca_core::protocol::{DebtSource, HealthCheckSwitch, LocationId, ProjectId};
use codotheca_core::testing::{FakeGitBackend, FakeTransport, GitReply};

use super::TestRepo;

/// The scheduler site's world: a real repository and an index holding one project on it.
pub(crate) struct ReleaseFixture {
    pub(crate) repo: TestRepo,
    pub(crate) dir: tempfile::TempDir,
    pub(crate) index: Index,
    pub(crate) project: ProjectId,
    pub(crate) location: LocationId,
}

/// A repository with one commit and an entry point — so J3 classifies it `cli`, for which
/// `release` applies — and an index opened at `t0` holding one enrolled, authored, non-Reference
/// project on one present copy of it. The README and licence checks are off, so `no_release` is
/// the only item a J1 settle opens on it.
pub(crate) fn release_fixture(t0: i64) -> ReleaseFixture {
    let repo = TestRepo::init();
    repo.write("main.py", b"print('one')\n");
    repo.commit("first");
    let dir = tempfile::tempdir().unwrap();
    let mut index = Index::open_at(dir.path(), t0).unwrap();
    let path = repo.path().to_string_lossy().into_owned();
    let (project, location) = index
        .with_tx(|tx| {
            tx.execute(
                "INSERT INTO project (name, seed_basename, lineage_key, authored_by_user,
                                      is_reference, acknowledged_at, created_at, updated_at)
                 VALUES ('p', 'p', 'fixture', 1, 0, ?1, 0, 0)",
                [t0 - 100],
            )?;
            let project = ProjectId(tx.last_insert_rowid());
            tx.execute(
                "INSERT INTO location (project_id, kind, path_bytes, path_key, path_display,
                                       store_key, presence, repo_kind)
                 VALUES (?1, 'linux', ?2, ?2, ?3, 'store', 'present', 'worktree')",
                rusqlite::params![project.0, path.as_bytes(), path],
            )?;
            let location = LocationId(tx.last_insert_rowid());
            write_switches(
                tx,
                &[
                    HealthCheckSwitch {
                        check: DebtSource::MissingReadme,
                        enabled: false,
                    },
                    HealthCheckSwitch {
                        check: DebtSource::MissingLicense,
                        enabled: false,
                    },
                ],
            )?;
            Ok((project, location))
        })
        .unwrap();
    ReleaseFixture {
        repo,
        dir,
        index,
        project,
        location,
    }
}

/// J7's world: an index holding one project on one present copy at HEAD `head-one`.
pub(crate) struct J7Fixture {
    pub(crate) dir: tempfile::TempDir,
    pub(crate) index: Index,
    pub(crate) project: ProjectId,
    pub(crate) location: LocationId,
}

/// An index opened at `now`, the content scan on, and one enrolled, authored, non-Reference
/// project on store `store-a`, whose HEAD [`marker_tree`] scripts.
pub(crate) fn j7_fixture(now: i64) -> J7Fixture {
    let dir = tempfile::tempdir().unwrap();
    let mut index = Index::open_at(dir.path(), now).unwrap();
    let (project, location) = index
        .with_tx(|tx| {
            tx.execute(
                "INSERT INTO project (name, seed_basename, lineage_key, authored_by_user,
                                      is_reference, acknowledged_at, created_at, updated_at)
                 VALUES ('p', 'p', 'fixture', 1, 0, ?1, 0, 0)",
                [now - 1_000],
            )?;
            let project = ProjectId(tx.last_insert_rowid());
            tx.execute(
                "INSERT INTO app_meta (k, v) VALUES ('content_scan_enabled', '1')
                 ON CONFLICT(k) DO UPDATE SET v = excluded.v",
                [],
            )?;
            tx.execute(
                "INSERT INTO location
                   (project_id, kind, path_bytes, path_key, path_display, store_key,
                    presence, repo_kind, head_oid)
                 VALUES (?1, 'linux', x'2f70', x'2f70', '/p', 'store-a', 'present',
                         'worktree', 'head-one')",
                [project.0],
            )?;
            Ok((project, LocationId(tx.last_insert_rowid())))
        })
        .unwrap();
    J7Fixture {
        dir,
        index,
        project,
        location,
    }
}

/// `location` at HEAD `head`: one file carrying `markers` distinct TODO lines.
pub(crate) fn marker_tree(
    git: &FakeGitBackend,
    index: &Mutex<Index>,
    location: LocationId,
    head: &str,
    markers: usize,
) {
    let mut bytes = String::new();
    for i in 0..markers {
        let _ = writeln!(bytes, "// TODO: marker number {i}");
    }
    let oid = format!("{head:a>40}");
    git.script_blob(&oid, bytes.as_bytes());
    git.always_head_tree(GitReply::Ok(vec![TreeEntry {
        mode: "100644".to_owned(),
        kind: "blob".to_owned(),
        oid,
        path: b"src/a.rs".to_vec(),
    }]));
    let guard = index.lock().unwrap();
    guard
        .conn()
        .execute(
            "UPDATE location SET head_oid = ?2 WHERE id = ?1",
            rusqlite::params![location.0, head],
        )
        .unwrap();
    drop(guard);
}

/// The advisory site's project: enrolled and authored, created at `now`. `located` gives it a
/// lineage and one location row at `work`; without one it has no subject.
pub(crate) fn advisory_project(
    index: &Mutex<Index>,
    work: &Path,
    now: i64,
    located: bool,
) -> ProjectId {
    let mut guard = index.lock().unwrap();
    let project = guard
        .with_tx(|tx| {
            tx.execute(
                "INSERT INTO project (name, seed_basename, lineage_key, acknowledged_at,
                                      authored_by_user, is_reference, created_at, updated_at)
                 VALUES ('alpha', 'alpha', ?1, ?2, 1, 0, 1, 1)",
                rusqlite::params![located.then_some("alpha"), now],
            )?;
            let project = ProjectId(tx.last_insert_rowid());
            if located {
                let path = work.to_string_lossy().into_owned();
                tx.execute(
                    "INSERT INTO location (project_id, kind, path_bytes, path_key,
                                           path_display, store_key, presence, repo_kind)
                     VALUES (?1, 'linux', ?2, ?2, ?3, 'store', 'present', 'worktree')",
                    rusqlite::params![project.0, path.as_bytes(), path],
                )?;
            }
            Ok(project)
        })
        .unwrap();
    drop(guard);
    project
}

/// `work` resolves `left` at `version`, and the lockfile read J6 runs has seen it at `at`.
pub(crate) fn lock_left(
    index: &Mutex<Index>,
    project: ProjectId,
    work: &Path,
    version: &str,
    at: i64,
) {
    std::fs::write(
        work.join("package-lock.json"),
        format!(
            r#"{{"lockfileVersion":3,"packages":{{"":{{}},"node_modules/left":{{"version":"{version}"}}}}}}"#
        ),
    )
    .unwrap();
    let mut guard = index.lock().unwrap();
    guard
        .with_tx(|tx| {
            read_lockfiles(tx, project, work, at).unwrap();
            Ok(())
        })
        .unwrap();
    drop(guard);
}

/// The forge's next answer: `body`, on the core rate-limit resource.
pub(crate) fn answer(transport: &FakeTransport, body: &str) {
    transport.push(HttpResponse {
        status: 200,
        headers: codotheca_core::http::normalise_headers([("x-ratelimit-resource", "core")]),
        body: body.as_bytes().to_vec(),
    });
}
