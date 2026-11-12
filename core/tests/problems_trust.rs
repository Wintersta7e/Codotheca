#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! §11.1's `TRUST THIS REPOSITORY` through the real route: a scan under a foreign owner records
//! one `untrusted_repo` row that keeps its folder, `problems.list` offers it by `problemId`, and
//! `problems.trust` indexes it with `-c safe.directory` on argv — `trusted_at` set, the row gone,
//! a second call refused, and no byte of the repository's or the user's git config written.
//!
//! Every test takes [`GIT_ENV`]: the different-owner half sets a process-wide variable that
//! would turn every other repository in this binary into someone else's.

#![cfg(feature = "testkit")]

mod support;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use codotheca_core::clock::SystemClock;
use codotheca_core::git::{GitSlots, SystemGit};
use codotheca_core::index::Index;
use codotheca_core::proto::dispatch::{CommandFailure, CommandHandler as _};
use codotheca_core::protocol::ErrorCode;
use codotheca_core::scan::ScanLauncher;
use codotheca_core::testing::{core_handler_with, CoreHandlerParts, HeldScanLauncher};
use serde_json::{json, Value};
use support::scan_rig::{mounts_for, scan_in};
use support::TestRepo;

const NOW: i64 = 1_700_000_000;
const DIFFERENT_OWNER: &str = "GIT_TEST_ASSUME_DIFFERENT_OWNER";

static GIT_ENV: Mutex<()> = Mutex::new(());

/// Sets [`DIFFERENT_OWNER`] and clears it on drop, so a failed assertion cannot leave it set.
struct ForeignOwner;

impl ForeignOwner {
    fn set() -> Self {
        std::env::set_var(DIFFERENT_OWNER, "1");
        Self
    }
}

impl Drop for ForeignOwner {
    fn drop(&mut self) {
        std::env::remove_var(DIFFERENT_OWNER);
    }
}

struct Rig {
    _dir: tempfile::TempDir,
    index: Arc<Mutex<Index>>,
    parts: CoreHandlerParts,
}

/// A handler over a fresh index whose command arms run `repo`'s real git, with `root` mounted.
fn rig(repo: &TestRepo, root: &Path) -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let index = Arc::new(Mutex::new(Index::open_at(dir.path(), NOW).unwrap()));
    let launcher: Arc<dyn ScanLauncher> = Arc::new(HeldScanLauncher::new(Arc::clone(&index)));
    let git = Arc::new(SystemGit::new(
        Arc::new(repo.exec()),
        Arc::new(GitSlots::new(4)),
        Arc::new(SystemClock::new()),
    ));
    let parts = core_handler_with(
        dir.path(),
        &index,
        NOW,
        launcher,
        git,
        Arc::new(mounts_for(root)),
    );
    Rig {
        _dir: dir,
        index,
        parts,
    }
}

impl Rig {
    fn trust(&mut self, problem_id: &Value) -> Result<Value, CommandFailure> {
        self.parts
            .handler
            .handle("problems.trust", json!({ "problemId": problem_id }))
    }

    fn list(&mut self) -> Value {
        self.parts
            .handler
            .handle("problems.list", json!({}))
            .expect("problems.list is answerable")
    }

    fn query<T: rusqlite::types::FromSql>(&self, sql: &str) -> T {
        self.index
            .lock()
            .unwrap()
            .conn()
            .query_row(sql, [], |r| r.get(0))
            .unwrap()
    }

    fn exec(&self, sql: &str, params: impl rusqlite::Params) -> i64 {
        let guard = self.index.lock().unwrap();
        guard.conn().execute(sql, params).unwrap();
        guard.conn().last_insert_rowid()
    }
}

/// Every item of `list`'s group of `kind`, empty when the group is absent.
fn items(list: &Value, kind: &str) -> Vec<Value> {
    list["groups"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|g| g["kind"] == kind)
        .flat_map(|g| g["items"].as_array().unwrap().clone())
        .collect()
}

/// The files git reads config from that a write could reach: the repository's own, and the
/// global ones of the user this process runs as.
fn config_files(repo: &Path) -> Vec<(PathBuf, Option<Vec<u8>>)> {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from);
    let xdg = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| home.as_ref().map(|h| h.join(".config")));
    let mut files = vec![repo.join(".git").join("config")];
    files.extend(home.map(|h| h.join(".gitconfig")));
    files.extend(xdg.map(|x| x.join("git").join("config")));
    files
        .into_iter()
        .map(|f| {
            let bytes = std::fs::read(&f).ok();
            (f, bytes)
        })
        .collect()
}

fn root_bytes(path: &Path) -> Vec<u8> {
    path.to_string_lossy().as_bytes().to_vec()
}

#[test]
fn an_untrusted_repository_is_indexed_from_its_problem_row_and_its_config_is_untouched() {
    let _env = GIT_ENV
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let repo = TestRepo::init();
    repo.write("a.txt", b"a\n");
    repo.commit("first");
    let root = support::canonical(repo.scratch());
    let folder = root.join("repo");
    // A damaged repository beside it: its row must keep no folder and offer no TRUST.
    std::fs::create_dir_all(root.join("broken").join(".git")).unwrap();
    std::fs::write(root.join("broken").join(".git").join("junk"), b"x").unwrap();

    let mut rig = rig(&repo, &root);
    let foreign = ForeignOwner::set();
    assert!(
        !repo.try_git(&["status"]),
        "git must honour {DIFFERENT_OWNER} for this test to mean anything"
    );
    let scanned = scan_in(&repo, &root, &rig.index);
    assert_eq!(scanned.indexed, 0, "{:?}", scanned.locations);
    assert!(scanned.locations.is_empty(), "{:?}", scanned.locations);
    let kinds: Vec<&str> = scanned.problems.iter().map(|p| p.0.as_str()).collect();
    assert_eq!(
        kinds.iter().filter(|k| **k == "untrusted_repo").count(),
        1,
        "{:?}",
        scanned.problems
    );
    assert!(kinds.contains(&"unreadable_repo"), "{:?}", scanned.problems);

    // The writer keeps the untrusted folder's raw bytes and its root, and nothing for any
    // other kind.
    let root_id: i64 = rig.query("SELECT id FROM scan_root");
    let stored: (Option<Vec<u8>>, Option<i64>) = rig
        .index
        .lock()
        .unwrap()
        .conn()
        .query_row(
            "SELECT path_bytes, root_id FROM scan_problem WHERE kind = 'untrusted_repo'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(stored, (Some(root_bytes(&folder)), Some(root_id)));
    let others_with_origin: i64 = rig.query(
        "SELECT COUNT(*) FROM scan_problem
          WHERE kind <> 'untrusted_repo' AND (path_bytes IS NOT NULL OR root_id IS NOT NULL)",
    );
    assert_eq!(others_with_origin, 0);

    let list = rig.list();
    let untrusted = items(&list, "untrusted_repo");
    assert_eq!(untrusted.len(), 1, "{list}");
    let problem_id = untrusted[0]["problemId"].clone();
    assert!(
        problem_id.is_i64(),
        "an untrusted row carries its id: {list}"
    );
    let unreadable = items(&list, "unreadable_repo");
    assert!(!unreadable.is_empty(), "{list}");
    assert!(
        unreadable.iter().all(|i| i["problemId"].is_null()),
        "no other kind is offered TRUST: {list}"
    );

    let before = config_files(&folder);
    let location = rig
        .trust(&problem_id)
        .expect("trusting the untrusted row indexes it");

    // Still foreign: what indexed it was the argv `safe.directory`, not a changed owner.
    assert!(!repo.try_git(&["status"]));
    let (path_display, trusted_at): (String, Option<i64>) = rig
        .index
        .lock()
        .unwrap()
        .conn()
        .query_row(
            "SELECT path_display, trusted_at FROM location WHERE id = ?1",
            [location.as_i64().unwrap()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(PathBuf::from(path_display), folder);
    assert_eq!(trusted_at, Some(NOW));
    let rows_left: i64 =
        rig.query("SELECT COUNT(*) FROM scan_problem WHERE kind = 'untrusted_repo'");
    assert_eq!(rows_left, 0, "the answered row leaves the summary");
    let listed = items(&rig.list(), "untrusted_repo");
    assert!(listed.is_empty(), "{listed:?}");

    let again = rig.trust(&problem_id).unwrap_err();
    assert_eq!(again.code, ErrorCode::PathGone, "{}", again.message);
    let locations: i64 = rig.query("SELECT COUNT(*) FROM location");
    assert_eq!(locations, 1, "a second call indexes nothing");
    drop(foreign);

    let after = config_files(&folder);
    for ((file, a), (_, b)) in before.iter().zip(&after) {
        assert!(a == b, "{} changed", file.display());
    }
    assert!(
        before[0].1.is_some(),
        "the repository's own config was read, not skipped"
    );
}

#[test]
fn each_refusal_takes_no_effect() {
    let _env = GIT_ENV
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let repo = TestRepo::init();
    let root = support::canonical(repo.scratch()).join("root");
    std::fs::create_dir_all(root.join("plain")).unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    let elsewhere = support::canonical(elsewhere.path());
    let vanished = root.join("vanished");

    let mut rig = rig(&repo, &root);
    scan_in(&repo, &root, &rig.index);
    let run: i64 = rig.query("SELECT id FROM scan_run");
    let root_id: i64 = rig.query("SELECT id FROM scan_root");
    let gone_root = rig.exec(
        "INSERT INTO scan_root (kind, distro, path_bytes, path_key, path_display,
                                enabled, added_by, descend_into_repos, added_at)
         VALUES ('linux', '', x'2f676f6e65', x'2f676f6e65', '/gone', 1, 'user', 0, 1)",
        [],
    );
    let plain = root.join("plain");
    let untrusted = "untrusted_repo";
    let cases = [
        (
            "another kind",
            "unreadable_repo",
            Some(&plain),
            Some(root_id),
            ErrorCode::Protocol,
        ),
        (
            "no stored folder",
            untrusted,
            None,
            Some(root_id),
            ErrorCode::PathGone,
        ),
        (
            "no stored root",
            untrusted,
            Some(&plain),
            None,
            ErrorCode::PathGone,
        ),
        (
            "a removed root",
            untrusted,
            Some(&plain),
            Some(gone_root),
            ErrorCode::PathGone,
        ),
        (
            "a folder no longer there",
            untrusted,
            Some(&vanished),
            Some(root_id),
            ErrorCode::PathGone,
        ),
        (
            "an unmounted store",
            untrusted,
            Some(&elsewhere),
            Some(root_id),
            ErrorCode::StoreOffline,
        ),
        (
            "not a repository",
            untrusted,
            Some(&plain),
            Some(root_id),
            ErrorCode::RepoUnreadable,
        ),
    ];
    let seeded: Vec<(&str, Value, ErrorCode)> = cases
        .into_iter()
        .map(|(case, kind, folder, under, code)| {
            let id = rig.exec(
                "INSERT INTO scan_problem (scan_run_id, kind, path_display, path_bytes, root_id)
                 VALUES (?1, ?2, 'shown', ?3, ?4)",
                rusqlite::params![run, kind, folder.map(|f| root_bytes(f)), under],
            );
            (case, json!(id), code)
        })
        .chain([("no such row", json!(999_999), ErrorCode::PathGone)])
        .collect();
    rig.exec("DELETE FROM scan_root WHERE id = ?1", [gone_root]);
    let rows_before: i64 = rig.query("SELECT COUNT(*) FROM scan_problem");

    for (case, id, code) in seeded {
        let refused = rig.trust(&id).unwrap_err();
        assert_eq!(refused.code, code, "{case}: {}", refused.message);
        assert!(
            refused.outcome.is_none(),
            "{case}: a refusal took no effect"
        );
    }
    let rows_after: i64 = rig.query("SELECT COUNT(*) FROM scan_problem");
    assert_eq!(rows_after, rows_before, "no refusal removed a row");
    let locations: i64 = rig.query("SELECT COUNT(*) FROM location");
    assert_eq!(locations, 0);
}

#[test]
fn arguments_that_do_not_parse_are_a_protocol_refusal() {
    let _env = GIT_ENV
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let repo = TestRepo::init();
    let mut rig = rig(&repo, repo.scratch());
    let malformed = rig
        .parts
        .handler
        .handle("problems.trust", json!({}))
        .unwrap_err();
    assert_eq!(malformed.code, ErrorCode::Protocol, "{}", malformed.message);
}
