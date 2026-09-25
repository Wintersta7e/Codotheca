#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
#![cfg(feature = "testkit")]
//! §24.3d: Install previews one stable identity and refuses every collision.

use std::path::{Path, PathBuf};

use codotheca_core::index::path::native_platform;
use codotheca_core::install::destination::{clone_url as install_clone_url, compose_destination};
use codotheca_core::install::handle_preview;
use codotheca_core::paths::{path_bytes, path_display, path_key};
use codotheca_core::proto::txguard::TxGuard;
use codotheca_core::protocol::{
    InstallDestination, InstallPreview, InstallRefusal, ProjectId, RemoteLinkKind, RootId,
    ScopeTier,
};
use codotheca_core::remote::weburl::{enterprise_hosts, web_url};
use codotheca_core::surfaces::SurfaceCtx;
use codotheca_core::testing::{FakeGitBackend, TempIndex};
use rusqlite::{params, OptionalExtension as _};

struct Fixture {
    index: TempIndex,
    _root_container: tempfile::TempDir,
    root: PathBuf,
    root_id: RootId,
}

impl Fixture {
    fn new() -> Self {
        let root_container = tempfile::tempdir().expect("root container");
        let root = root_container.path().join("library");
        std::fs::create_dir(&root).expect("library root");
        let index = TempIndex::new();
        let root_id = insert_scan_root(index.index().conn(), &root);
        Self {
            index,
            _root_container: root_container,
            root,
            root_id,
        }
    }

    fn insert_project(&self, seed: &str, remote_key: Option<&str>) -> ProjectId {
        let conn = self.index.index().conn();
        let next_id: i64 = conn
            .query_row("SELECT COALESCE(MAX(id), 0) + 1 FROM project", [], |row| {
                row.get(0)
            })
            .expect("next project id");
        let provider_repo_id = format!("fixture-{next_id}");
        conn.execute(
            "INSERT INTO project
               (name, seed_basename, remote_key, provider, provider_repo_id,
                created_at, updated_at)
             VALUES (?1, ?1, ?2, 'github', ?3, 0, 0)",
            params![seed, remote_key, provider_repo_id],
        )
        .expect("project fixture");
        ProjectId(conn.last_insert_rowid())
    }

    fn set_visibility(&self, project: ProjectId, visibility: Option<&str>) {
        let conn = self.index.index().conn();
        let (provider, repo_id): (String, String) = conn
            .query_row(
                "SELECT provider, provider_repo_id FROM project WHERE id = ?1",
                [project.0],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("project binding");
        conn.execute(
            "INSERT INTO remote_repo (provider, provider_repo_id, visibility)
             VALUES (?1, ?2, ?3)",
            params![provider, repo_id, visibility],
        )
        .expect("remote facts fixture");
    }

    fn insert_account(&self, tier: ScopeTier) {
        let encoded = serde_json::to_value(tier).expect("scope tier encodes");
        let scope_tier = encoded.as_str().expect("scope tier is text");
        self.index
            .index()
            .conn()
            .execute(
                "INSERT INTO account
                   (provider, host, login, auth_kind, scope_tier, granted_scopes,
                    token_ref, connected_at)
                 VALUES ('github', 'github.com', 'fixture-user', 'pat', ?1, '[]',
                         'fixture-token-ref', 0)",
                [scope_tier],
            )
            .expect("account fixture");
    }

    fn destination(&self, seed: &str) -> PathBuf {
        self.root.join(seed)
    }

    fn insert_location(&self, project: ProjectId, path: &Path, removed_at: Option<i64>) {
        let bytes = path_bytes(path);
        let key = path_key(path, native_platform());
        let display = path_display(path);
        self.index
            .index()
            .conn()
            .execute(
                "INSERT INTO location
                   (project_id, kind, distro, path_bytes, path_key, path_display, store_key,
                    presence, repo_kind, removed_at)
                 VALUES (?1, ?2, '', ?3, ?4, ?5, 'fixture-store', 'present', 'worktree', ?6)",
                params![project.0, native_kind(), bytes, key, display, removed_at],
            )
            .expect("location fixture");
    }

    // A read transaction the fixture cannot open is a broken fixture, not a refusal the composer
    // could answer with, so it panics like every other fixture failure in this file.
    #[allow(clippy::unwrap_in_result)]
    fn compose(&self, project: ProjectId) -> Result<InstallDestination, InstallRefusal> {
        let _tx_guard = TxGuard::enter();
        let tx = self
            .index
            .index()
            .conn()
            .unchecked_transaction()
            .expect("read transaction");
        let answer = compose_destination(&tx, project, self.root_id);
        drop(tx);
        answer
    }

    fn preview(&self, project: ProjectId) -> InstallPreview {
        let ctx = SurfaceCtx {
            index: self.index.index(),
            now: 0,
        };
        handle_preview(
            &ctx,
            serde_json::json!({"projectId": project.0, "rootId": self.root_id.0}),
        )
        .expect("preview answers")
    }

    fn basenames(&self) -> Vec<(i64, Vec<u8>)> {
        let mut statement = self
            .index
            .index()
            .conn()
            .prepare("SELECT id, seed_basename FROM project ORDER BY id")
            .expect("basename query");
        statement
            .query_map([], |row| {
                let id = row.get(0)?;
                let seed: String = row.get(1)?;
                Ok((id, seed.into_bytes()))
            })
            .expect("basename rows")
            .collect::<Result<Vec<_>, _>>()
            .expect("basename values")
    }

    fn stored_seed(&self, project: ProjectId) -> String {
        self.index
            .index()
            .conn()
            .query_row(
                "SELECT seed_basename FROM project WHERE id = ?1",
                [project.0],
                |row| row.get(0),
            )
            .expect("stored seed")
    }

    fn assert_refusal(&self, project: ProjectId, expected: InstallRefusal) {
        let before = self.basenames();
        match self.compose(project) {
            Err(actual) => assert_eq!(actual, expected),
            Ok(_) => panic!("expected {expected:?} refusal, got a destination"),
        }
        assert_eq!(
            self.basenames(),
            before,
            "a refusal must not rewrite any project seed_basename"
        );
    }
}

const fn native_kind() -> &'static str {
    if cfg!(windows) {
        "win"
    } else {
        "linux"
    }
}

fn insert_scan_root(conn: &rusqlite::Connection, path: &Path) -> RootId {
    let bytes = path_bytes(path);
    let key = path_key(path, native_platform());
    let display = path_display(path);
    conn.execute(
        "INSERT INTO scan_root
           (kind, distro, path_bytes, path_key, path_display, enabled, added_by,
            descend_into_repos, added_at)
         VALUES (?1, '', ?2, ?3, ?4, 1, 'user', 0, 0)",
        params![native_kind(), bytes, key, display],
    )
    .expect("scan-root fixture");
    RootId(conn.last_insert_rowid())
}

fn assert_returned_identity(fixture: &Fixture, project: ProjectId) {
    let stored = fixture.stored_seed(project);
    if let Ok(destination) = fixture.compose(project) {
        assert_eq!(
            destination.seed_basename.as_bytes(),
            stored.as_bytes(),
            "a successful composition must return the stored seed byte-for-byte"
        );
    }
}

#[test]
fn install_destination_missing_root_is_root_unavailable_and_changes_no_seed() {
    let fixture = Fixture::new();
    let project = fixture.insert_project("widget", Some("github.com/acme/widget"));
    std::fs::remove_dir(&fixture.root).expect("remove fixture root");

    fixture.assert_refusal(project, InstallRefusal::RootUnavailable);
}

#[test]
fn install_destination_key_with_no_shared_https_url_is_refused_and_changes_no_seed() {
    let fixture = Fixture::new();
    let project = fixture.insert_project("widget", Some("forge.invalid/acme/widget"));

    fixture.assert_refusal(project, InstallRefusal::NoCloneUrl);
}

#[test]
fn install_destination_case_varied_reserved_names_are_unsafe_and_change_no_seed() {
    let fixture = Fixture::new();
    for seed in ["aUx", "COM1"] {
        let project = fixture.insert_project(seed, Some("github.com/acme/widget"));
        fixture.assert_refusal(project, InstallRefusal::UnsafeName);
    }
}

/// AC-P2-24-23. The fake records every `GitBackend` call and has no replies configured. The
/// production path accepts no `GitBackend` at all; the repository-wide no-unaudited-spawn gate is
/// the structural complement that confines direct process construction to the audited seams.
#[test]
fn install_destination_private_remote_on_public_tier_refuses_without_git() {
    let fixture = Fixture::new();
    let project = fixture.insert_project("widget", Some("github.com/acme/widget"));
    fixture.set_visibility(project, Some("private"));
    fixture.insert_account(ScopeTier::Public);
    let git = FakeGitBackend::new();

    fixture.assert_refusal(project, InstallRefusal::PrivateNeedsUpgrade);
    assert!(
        git.calls().is_empty(),
        "private_needs_upgrade must not invoke Git: {:?}",
        git.calls()
    );
}

#[test]
fn install_destination_unknown_visibility_is_not_public_or_private() {
    let fixture = Fixture::new();
    let project = fixture.insert_project("widget", Some("github.com/acme/widget"));
    fixture.set_visibility(project, None);
    fixture.insert_account(ScopeTier::Public);

    let destination = fixture
        .compose(project)
        .expect("unknown visibility proceeds");
    assert_eq!(destination.seed_basename, "widget");
}

#[test]
fn install_destination_unindexed_collision_is_destination_exists_and_changes_no_seed() {
    let fixture = Fixture::new();
    let project = fixture.insert_project("widget", Some("github.com/acme/widget"));
    std::fs::create_dir(fixture.destination("widget")).expect("collision fixture");

    fixture.assert_refusal(project, InstallRefusal::DestinationExists);
}

#[test]
fn install_destination_persisted_non_removed_location_is_already_installed() {
    let fixture = Fixture::new();
    let project = fixture.insert_project("widget", Some("github.com/acme/widget"));
    let destination = fixture.destination("widget");
    std::fs::create_dir(&destination).expect("existing destination");
    fixture.insert_location(project, &destination, None);

    fixture.assert_refusal(project, InstallRefusal::AlreadyInstalled);
}

#[test]
fn install_destination_removed_location_is_not_installed_evidence() {
    let fixture = Fixture::new();
    let project = fixture.insert_project("widget", Some("github.com/acme/widget"));
    let destination = fixture.destination("widget");
    std::fs::create_dir(&destination).expect("existing destination");
    fixture.insert_location(project, &destination, Some(1));

    fixture.assert_refusal(project, InstallRefusal::DestinationExists);
}

#[test]
fn install_destination_registered_scan_root_is_refused_even_when_absent() {
    let fixture = Fixture::new();
    let project = fixture.insert_project("nested-root", Some("github.com/acme/nested-root"));
    let destination = fixture.destination("nested-root");
    let _nested_root = insert_scan_root(fixture.index.index().conn(), &destination);

    fixture.assert_refusal(project, InstallRefusal::DestinationExists);
}

#[test]
fn install_destination_success_preserves_stored_seed_and_uses_lossy_display() {
    let fixture = Fixture::new();
    let project = fixture.insert_project("MyRepo", Some("github.com/acme/myrepo"));

    let destination = fixture.compose(project).expect("destination composes");
    assert_eq!(destination.root_id, fixture.root_id);
    assert_eq!(destination.seed_basename.as_bytes(), b"MyRepo");
    assert_eq!(
        destination.display,
        path_display(&fixture.destination("MyRepo"))
    );
    assert_returned_identity(&fixture, project);
}

#[test]
fn install_destination_clone_url_is_the_shared_web_url_for_the_same_key() {
    let fixture = Fixture::new();
    let key = "github.com/acme/widget";
    let project = fixture.insert_project("widget", Some(key));
    let _tx_guard = TxGuard::enter();
    let tx = fixture
        .index
        .index()
        .conn()
        .unchecked_transaction()
        .expect("read transaction");
    let actual = install_clone_url(&tx, project).expect("clone URL query");
    let hosts = enterprise_hosts(&tx).expect("enterprise host query");
    let expected = web_url(key, RemoteLinkKind::Repository, &hosts);

    assert_eq!(actual, expected);
}

/// This is the identity half of the no-suffix mutation check. Correct code returns `Err` for
/// the collision; a mutant that returns a suffixed success enters the assertion and fails.
#[test]
fn install_destination_every_success_keeps_stored_identity_at_a_collision() {
    let fixture = Fixture::new();
    let ordinary = fixture.insert_project("ordinary", Some("github.com/acme/ordinary"));
    assert_returned_identity(&fixture, ordinary);

    let colliding = fixture.insert_project("collision", Some("github.com/acme/collision"));
    std::fs::create_dir(fixture.destination("collision")).expect("collision fixture");
    assert_returned_identity(&fixture, colliding);
}

#[test]
fn install_destination_preview_returns_disjoint_destination_or_refusal() {
    let fixture = Fixture::new();
    let available = fixture.insert_project("available", Some("github.com/acme/available"));
    let success = fixture.preview(available);
    assert!(success.destination.is_some());
    assert_eq!(success.refused_because, None);

    let blocked = fixture.insert_project("blocked", Some("github.com/acme/blocked"));
    std::fs::create_dir(fixture.destination("blocked")).expect("collision fixture");
    let refusal = fixture.preview(blocked);
    assert!(
        refusal.destination.is_none(),
        "a refused preview must expose no destination"
    );
    assert_eq!(
        refusal.refused_because,
        Some(InstallRefusal::DestinationExists)
    );
}

#[test]
fn install_destination_no_root_choice_remains_task_19() {
    let fixture = Fixture::new();
    let project = fixture.insert_project("widget", Some("github.com/acme/widget"));
    let stored_setting: Option<String> = fixture
        .index
        .index()
        .conn()
        .query_row(
            "SELECT v FROM app_meta WHERE k = 'install_root_id'",
            [],
            |row| row.get(0),
        )
        .optional()
        .expect("setting query");
    assert_eq!(stored_setting, None, "the chooser has not stored a root");

    assert!(
        fixture.compose(project).is_ok(),
        "Task 10 previews the supplied RootId; Task 19 owns the no-choice control state"
    );
}
