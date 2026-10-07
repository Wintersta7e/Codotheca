//! Three user decisions from phases 1–3 the sidecar never carried survive a rebuild: a check's
//! N/A ruling, a copy's trust, and a connected account with its organisation switches.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};

use codotheca_core::accounts::keychain::{SecretToken, TokenStore as _};
use codotheca_core::accounts::store::{
    insert_account, list_accounts, list_orgs, set_org_enabled, upsert_orgs, NewAccount,
};
use codotheca_core::assembly::handoff::{hand_off_discovered, HandoffCtx, Indexed};
use codotheca_core::cancel::CancelToken;
use codotheca_core::clock::SystemClock;
use codotheca_core::completion::{evaluate_and_write, set_check_na};
use codotheca_core::git::{GitBackend, GitExec, GitSlots, RepoHandle, StoreKey, SystemGit};
use codotheca_core::index::rebuild::{rebuild_in_place, RebuildOutcome};
use codotheca_core::index::Index;
use codotheca_core::mount::StoreClass;
use codotheca_core::paths::{path_bytes, path_display, path_key};
use codotheca_core::protocol::{AuthKind, CompletionCheck, LocationId, ProjectId, ScopeTier};
use codotheca_core::provider::listing::OrgListing;
use codotheca_core::scan::discover::{RepoCandidate, RepoKind};
use codotheca_core::scan::run::{platform_of, Discovered};
use codotheca_core::surfaces::repair::set_trusted;
use codotheca_core::testing::FakeTokenStore;

const NOW: i64 = 1_760_000_000;
/// When the user trusted a copy: a time distinct from every clock the tests run under.
const TRUSTED_AT: i64 = 1_700_000_123;

/// A neutral git, so the machine's own config cannot change what a fixture is.
fn git_at(cwd: &Path, home: &Path, args: &[&str]) {
    let out = Command::new("git")
        .current_dir(cwd)
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", home)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("LC_ALL", "C")
        .env("GIT_AUTHOR_NAME", "Fixture")
        .env("GIT_AUTHOR_EMAIL", "fixture@example.invalid")
        .env("GIT_AUTHOR_DATE", "2024-01-02T03:04:05+00:00")
        .env("GIT_COMMITTER_NAME", "Fixture")
        .env("GIT_COMMITTER_EMAIL", "fixture@example.invalid")
        .env("GIT_COMMITTER_DATE", "2024-01-02T03:04:05+00:00")
        .args(["-c", "commit.gpgsign=false", "-c", "core.autocrlf=false"])
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "fixture git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A data directory, a git home and repositories on disk, indexed through the real hand-off.
struct Library {
    dir: tempfile::TempDir,
    git: Arc<dyn GitBackend>,
}

impl Library {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("home")).unwrap();
        let hooks = codotheca_core::git::ensure_empty_hooks_dir(&dir.path().join("hooks")).unwrap();
        let git: Arc<dyn GitBackend> = Arc::new(SystemGit::new(
            Arc::new(GitExec::system(hooks)),
            Arc::new(GitSlots::for_machine()),
            Arc::new(SystemClock::new()),
        ));
        Self { dir, git }
    }

    fn data(&self) -> PathBuf {
        self.dir.path().join("data")
    }

    fn home(&self) -> PathBuf {
        self.dir.path().join("home")
    }

    /// One repository with one commit of its own — so its own lineage — and a remote.
    fn repo(&self, name: &str) -> PathBuf {
        let path = self.dir.path().join("repos").join(name);
        std::fs::create_dir_all(&path).unwrap();
        git_at(&path, &self.home(), &["init", "-b", "main", "."]);
        std::fs::write(path.join("a.txt"), name.as_bytes()).unwrap();
        git_at(&path, &self.home(), &["add", "-A"]);
        git_at(&path, &self.home(), &["commit", "-m", "first"]);
        let url = format!("https://example.invalid/owner/{name}.git");
        git_at(&path, &self.home(), &["remote", "add", "origin", &url]);
        path
    }

    /// A second copy of `repo`, the same history and the same remote.
    fn copy_of(&self, repo: &Path, name: &str) -> PathBuf {
        let path = self.dir.path().join("repos").join(name);
        git_at(
            &self.dir.path().join("repos"),
            &self.home(),
            &["clone", "-q", &repo.to_string_lossy(), name],
        );
        let url = format!(
            "https://example.invalid/owner/{}.git",
            repo.file_name().unwrap().to_string_lossy()
        );
        git_at(&path, &self.home(), &["remote", "set-url", "origin", &url]);
        path
    }

    fn open(&self) -> Arc<Mutex<Index>> {
        Arc::new(Mutex::new(Index::open_at(&self.data(), NOW).unwrap()))
    }

    fn hand_off(&self, index: &Mutex<Index>, path: &Path) -> Indexed {
        let cancel = CancelToken::new();
        let ctx = HandoffCtx {
            git: self.git.as_ref(),
            cancel: &cancel,
            store_class: StoreClass::Local,
            generation: 1,
            now: NOW,
        };
        hand_off_discovered(index, &ctx, &discovered_at(path)).unwrap()
    }

    /// Export, close, overwrite the index with bytes SQLite reads as no database, and rebuild.
    fn export_corrupt_and_rebuild(&self, index: Arc<Mutex<Index>>) {
        index.lock().unwrap().export_sidecar(NOW).unwrap();
        drop(Arc::try_unwrap(index).unwrap());
        std::fs::write(Index::db_path(&self.data()), b"this is not a database").unwrap();
        match rebuild_in_place(&self.data(), NOW + 1) {
            Ok(RebuildOutcome::Rebuilt(report)) => eprintln!(
                "rebuilt: restored {:?}, {} pending",
                report.restored, report.pending
            ),
            other => panic!("expected a rebuild, got {other:?}"),
        }
    }
}

/// What the walk hands on, built from what `classify` would have resolved.
fn discovered_at(path: &Path) -> Discovered {
    let handle = RepoHandle::resolve(path, StoreKey::new("store-a"), StoreClass::Local).unwrap();
    Discovered {
        candidate: RepoCandidate {
            path: path.to_path_buf(),
            kind: RepoKind::WorkTree,
            git_dir: handle.git_dir.clone(),
            common_dir: handle.common_dir,
        },
        root_id: 1,
        kind: "linux".to_owned(),
        distro: String::new(),
        path_bytes: path_bytes(path),
        path_key: path_key(path, platform_of("linux")),
        path_display: path_display(path),
        store_key: "store-a".to_owned(),
        volume_key: Some("vol-a".to_owned()),
    }
}

/// What the jobs have written by the time a settle hook evaluates a project: its authorship, and
/// a copy whose refs were read, so `remote` is answered rather than unknown.
fn mark_scorable(index: &Mutex<Index>, project: ProjectId) {
    let guard = index.lock().unwrap();
    guard
        .conn()
        .execute(
            "UPDATE project SET authored_by_user = 1 WHERE id = ?1",
            [project.0],
        )
        .unwrap();
    guard
        .conn()
        .execute(
            "UPDATE location SET refstate_observed_at = ?2 WHERE project_id = ?1",
            [project.0, NOW],
        )
        .unwrap();
}

/// The evaluator, as a settle hook runs it.
fn evaluate(index: &Mutex<Index>, project: ProjectId) {
    index
        .lock()
        .unwrap()
        .with_tx(|tx| evaluate_and_write(tx, project, NOW))
        .unwrap();
}

/// The user's stored ruling on `remote` for `project`; `None` when there is none or no row.
fn remote_ruling(index: &Mutex<Index>, project: ProjectId) -> Option<bool> {
    index
        .lock()
        .unwrap()
        .conn()
        .query_row(
            "SELECT user_na FROM project_check WHERE project_id = ?1 AND check_key = 'remote'",
            [project.0],
            |r| r.get::<_, Option<i64>>(0),
        )
        .ok()
        .flatten()
        .map(|v| v != 0)
}

/// The projection the shelf renders: `(completion_lit, completion_applicable)`.
fn projection(index: &Mutex<Index>, project: ProjectId) -> (Option<i64>, Option<i64>) {
    index
        .lock()
        .unwrap()
        .conn()
        .query_row(
            "SELECT completion_lit, completion_applicable FROM project WHERE id = ?1",
            [project.0],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap()
}

fn trusted_at(index: &Mutex<Index>, location: LocationId) -> Option<i64> {
    index
        .lock()
        .unwrap()
        .conn()
        .query_row(
            "SELECT trusted_at FROM location WHERE id = ?1",
            [location.0],
            |r| r.get(0),
        )
        .unwrap()
}

fn pending(index: &Mutex<Index>) -> i64 {
    index
        .lock()
        .unwrap()
        .conn()
        .query_row("SELECT count(*) FROM sidecar_pending", [], |r| r.get(0))
        .unwrap()
}

/// §31.4's ruling is the user's and nothing re-derives it. It is restored onto the check's row,
/// which a rebuilt index has only once the evaluator writes the project's ten, so that first
/// write applies it: the ruling is never shown unmade, and the projection is the one the same
/// ruling gives an index that was never rebuilt.
#[test]
fn a_users_na_ruling_lands_with_the_first_evaluation_after_a_rebuild() {
    let lib = Library::new();
    let repo = lib.repo("widget");
    let index = lib.open();
    let project = lib.hand_off(&index, &repo).project;
    mark_scorable(&index, project);
    evaluate(&index, project);
    let unruled = projection(&index, project);
    index
        .lock()
        .unwrap()
        .with_tx(|tx| set_check_na(tx, project, CompletionCheck::Remote, Some(true), NOW))
        .unwrap();
    assert_eq!(remote_ruling(&index, project), Some(true));
    let twin = projection(&index, project);
    eprintln!("projection without the ruling {unruled:?}, with it {twin:?}");
    assert_ne!(
        unruled, twin,
        "the ruling must move the projection, or comparing projections proves nothing"
    );

    lib.export_corrupt_and_rebuild(index);
    let rebuilt = lib.open();
    let restored = lib.hand_off(&rebuilt, &repo).project;
    assert_eq!(
        pending(&rebuilt),
        1,
        "the ruling waits for a row to rule on"
    );
    assert_eq!(remote_ruling(&rebuilt, restored), None);

    mark_scorable(&rebuilt, restored);
    evaluate(&rebuilt, restored);
    let waiting = pending(&rebuilt);
    eprintln!("pending after the first evaluation: {waiting}");
    assert_eq!(waiting, 0, "the first evaluation left the ruling unapplied");
    assert_eq!(remote_ruling(&rebuilt, restored), Some(true));
    assert_eq!(projection(&rebuilt, restored), twin);
}

/// §11.1's trust is the user's act on one copy; the rebuilt index trusts the same copy from the
/// same moment.
#[test]
fn a_trusted_copy_stays_trusted() {
    let lib = Library::new();
    let repo = lib.repo("widget");
    let index = lib.open();
    let copy = lib.hand_off(&index, &repo).location;
    assert!(set_trusted(index.lock().unwrap().conn(), copy, TRUSTED_AT).unwrap());

    lib.export_corrupt_and_rebuild(index);
    let rebuilt = lib.open();
    let restored = lib.hand_off(&rebuilt, &repo).location;
    assert_eq!(trusted_at(&rebuilt, restored), Some(TRUSTED_AT));
    assert_eq!(pending(&rebuilt), 0);
}

/// §20's connection survives with the organisation switches the user set, and the sidecar holds
/// the keychain entry's name and never the token stored under it.
#[test]
fn a_connected_account_and_its_org_switches_survive_and_no_secret_is_exported() {
    const TOKEN: &str = "fixture-token-7d1e5b20c4a9";
    const TOKEN_REF: &str = "github:github.com:fixture-user";
    let lib = Library::new();
    let index = lib.open();
    let keychain = FakeTokenStore::available();
    keychain
        .store(TOKEN_REF, &SecretToken::new(TOKEN.to_owned()))
        .unwrap();
    index
        .lock()
        .unwrap()
        .with_tx(|tx| {
            let account = insert_account(
                tx,
                &NewAccount {
                    provider: "github".to_owned(),
                    host: "github.com".to_owned(),
                    login: "fixture-user".to_owned(),
                    display_name: Some("Fixture User".to_owned()),
                    auth_kind: AuthKind::Device,
                    scope_tier: ScopeTier::Private,
                    granted_scopes: vec!["repo".to_owned(), "read:org".to_owned()],
                    token_ref: TOKEN_REF.to_owned(),
                },
                1_700_000_000,
            )
            .unwrap();
            let orgs = [
                OrgListing {
                    login: "org-on".to_owned(),
                    repo_count_seen: Some(3),
                },
                OrgListing {
                    login: "org-off".to_owned(),
                    repo_count_seen: None,
                },
            ];
            upsert_orgs(tx, account, &orgs, 1_700_000_100).unwrap();
            set_org_enabled(tx, account, "org-on", true).unwrap();
            Ok(())
        })
        .unwrap();
    let before = list_accounts(index.lock().unwrap().conn()).unwrap();

    lib.export_corrupt_and_rebuild(index);
    let written = std::fs::read_to_string(Index::sidecar_path(&lib.data())).unwrap();
    let leaks = written.matches(TOKEN).count();
    eprintln!("the token's occurrences in the written sidecar: {leaks}");
    assert_eq!(leaks, 0, "the sidecar carries the token");
    assert!(written.contains(TOKEN_REF), "the account was not exported");

    let rebuilt = lib.open();
    let (after, listed) = {
        let guard = rebuilt.lock().unwrap();
        let accounts = list_accounts(guard.conn()).unwrap();
        let listed = accounts
            .first()
            .map(|account| list_orgs(guard.conn(), account.id).unwrap().unwrap());
        drop(guard);
        (accounts, listed)
    };
    assert_eq!(after.len(), 1, "{after:?}");
    let (was, now) = (&before[0], &after[0]);
    assert_eq!(
        (&now.provider, &now.host, &now.login, &now.display_name),
        (&was.provider, &was.host, &was.login, &was.display_name)
    );
    assert_eq!(
        (now.auth_kind, now.scope_tier, now.connected_at),
        (was.auth_kind, was.scope_tier, was.connected_at)
    );
    assert_eq!(now.granted_scopes, was.granted_scopes);
    let orgs = listed.unwrap();
    let switches: Vec<(&str, bool)> = orgs.iter().map(|o| (o.login.as_str(), o.enabled)).collect();
    assert_eq!(switches, [("org-off", false), ("org-on", true)]);
}

/// §48.8.4: a trust record names a copy, and a copy no scan has found yet is no copy to trust.
/// The record waits for the hand-off that brings it, rather than being spent on its sibling.
#[test]
fn a_trust_record_for_an_undiscovered_copy_stays_pending_and_a_later_scan_applies_it() {
    let lib = Library::new();
    let a = lib.repo("widget");
    let b = lib.copy_of(&a, "widget-b");
    let index = lib.open();
    let first = lib.hand_off(&index, &a);
    let second = lib.hand_off(&index, &b);
    assert_eq!(first.project, second.project, "the copies are one project");
    assert!(set_trusted(index.lock().unwrap().conn(), second.location, TRUSTED_AT).unwrap());

    lib.export_corrupt_and_rebuild(index);
    let rebuilt = lib.open();
    lib.hand_off(&rebuilt, &a);
    let waiting = pending(&rebuilt);
    eprintln!("pending after only the untrusted copy came back: {waiting}");
    assert_eq!(
        waiting, 1,
        "the trust record was spent before its copy came back"
    );
    let trusted: i64 = rebuilt
        .lock()
        .unwrap()
        .conn()
        .query_row(
            "SELECT count(*) FROM location WHERE trusted_at IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(trusted, 0, "a copy the user never trusted is trusted");

    let b_now = lib.hand_off(&rebuilt, &b).location;
    assert_eq!(pending(&rebuilt), 0);
    assert_eq!(trusted_at(&rebuilt, b_now), Some(TRUSTED_AT));
}
