//! A library for the analyser's handler-level tests: one scan root, working copies under it,
//! "network" origins the `TransportFixture` admits, a real index, the real read backend and the
//! two handlers.
//!
//! **Only the remote verifier may be doubled** (§45.14). Everything else here is production: the
//! read backend, the analyser, the composition, the fold. A test that wants a remote to answer
//! uses [`Library::verifier`] — the production `GitRemoteVerifier` over the production write path
//! with the fixture's one difference — and a test that wants one of the did-not-answer classes
//! uses `FixtureRemoteVerifier`.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::{Arc, Mutex};

use codotheca_core::analyser::remote::{GitRemoteVerifier, RemoteVerifier};
use codotheca_core::analyser::AnalyserSeams;
use codotheca_core::git::SystemGit;
use codotheca_core::gitw::{SystemMutatingGit, TransportFixture};
use codotheca_core::index::Index;
use codotheca_core::protocol::LocationId;
use codotheca_core::removal::Trash;
use codotheca_core::testing::CountingTrash;

use super::git_world::HOSTILE_READ_PROFILE;

/// The clock every handler call here reads.
pub(crate) const NOW: i64 = 1_750_000_000;

/// One scan root, its index and its seams.
pub(crate) struct Library {
    _dir: tempfile::TempDir,
    /// The canonical temporary directory everything lives under.
    pub(crate) base: PathBuf,
    /// Fixture git's `HOME`.
    pub(crate) home: PathBuf,
    /// The scan root the working copies sit under.
    pub(crate) root: PathBuf,
    /// Where "network" origins live: the `TransportFixture` admits exactly this tree.
    pub(crate) net: PathBuf,
    /// The index the handlers read.
    pub(crate) index: Arc<Mutex<Index>>,
    /// The production read backend, on the git under test.
    pub(crate) read_git: SystemGit,
    /// The production write path with the fixture's one difference.
    pub(crate) write_git: SystemMutatingGit,
}

impl Library {
    /// An empty library: one scan root, no locations.
    pub(crate) fn new() -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path().canonicalize().expect("canonical");
        let home = base.join("home");
        let root = base.join("root");
        let net = base.join("net");
        for d in [&home, &root, &net] {
            std::fs::create_dir_all(d).expect("fixture dir");
        }
        std::fs::write(home.join("empty.gitconfig"), b"").expect("global");
        let hooks =
            codotheca_core::git::ensure_empty_hooks_dir(&base.join("app-data")).expect("hooks");
        let read_git = SystemGit::new(
            Arc::new(codotheca_core::git::GitExec::new(
                super::test_git(),
                hooks.clone(),
            )),
            Arc::new(codotheca_core::git::GitSlots::for_machine()),
            Arc::new(codotheca_core::clock::SystemClock::new()),
        );
        let write_git = SystemMutatingGit::with_transport_fixture(
            super::test_git(),
            hooks,
            TransportFixture::new(&net),
        );
        let index = Arc::new(Mutex::new(
            Index::open_at(&base.join("index"), NOW).expect("index"),
        ));
        {
            let mut guard = index.lock().expect("index");
            guard
                .with_tx(|tx| {
                    tx.execute(
                        "INSERT INTO scan_root (kind, distro, path_bytes, path_key, path_display,
                                                added_by, added_at)
                         VALUES ('linux', '', ?1, ?1, ?2, 'user', 0)",
                        rusqlite::params![
                            root.to_string_lossy().as_bytes(),
                            root.to_string_lossy()
                        ],
                    )?;
                    Ok(())
                })
                .expect("scan root");
        }
        Self {
            _dir: dir,
            base,
            home,
            root,
            net,
            index,
            read_git,
            write_git,
        }
    }

    /// Fixture git in `cwd`, isolated from the developer's config and from every inherited
    /// `GIT_*`; returns the output whatever the exit.
    pub(crate) fn git_output(&self, cwd: &Path, args: &[&str]) -> Output {
        let mut cmd = Command::new("git");
        for (key, _) in std::env::vars_os() {
            if key
                .to_string_lossy()
                .to_ascii_uppercase()
                .starts_with("GIT_")
            {
                cmd.env_remove(key);
            }
        }
        cmd.current_dir(cwd)
            .env("HOME", &self.home)
            .env("XDG_CONFIG_HOME", &self.home)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", self.home.join("empty.gitconfig"))
            .env("GIT_AUTHOR_NAME", "Fixture")
            .env("GIT_AUTHOR_EMAIL", "fixture@example.invalid")
            .env("GIT_COMMITTER_NAME", "Fixture")
            .env("GIT_COMMITTER_EMAIL", "fixture@example.invalid")
            .env("GIT_ALLOW_PROTOCOL", OsString::from("file"))
            .args([
                "-c",
                "commit.gpgsign=false",
                "-c",
                "tag.gpgsign=false",
                "-c",
                "core.autocrlf=false",
                "-c",
                "protocol.file.allow=always",
            ])
            .args(args)
            .output()
            .expect("fixture git")
    }

    /// Fixture git that must succeed; returns stdout.
    pub(crate) fn git(&self, cwd: &Path, args: &[&str]) -> String {
        let out = self.git_output(cwd, args);
        assert!(
            out.status.success(),
            "fixture git {args:?} in {}: {}",
            cwd.display(),
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    /// Commit `body` to `file` in `repo`; returns the new commit.
    pub(crate) fn commit(&self, repo: &Path, file: &str, body: &str) -> String {
        if let Some(parent) = repo.join(file).parent() {
            std::fs::create_dir_all(parent).expect("parent");
        }
        std::fs::write(repo.join(file), body).expect("file");
        self.git(repo, &["add", "--", file]);
        self.git(repo, &["commit", "-q", "-m", &format!("{file}: {body}")]);
        self.git(repo, &["rev-parse", "HEAD"]).trim().to_owned()
    }

    /// §45.6's hostile read profile, delivered through the repository's own config.
    pub(crate) fn hostile(&self, repo: &Path) {
        for (key, value) in HOSTILE_READ_PROFILE {
            self.git(repo, &["config", key, value]);
        }
        self.git(repo, &["update-index", "--untracked-cache"]);
        self.git(repo, &["status", "--porcelain"]);
    }

    /// A repository at `<root>/<name>` with one commit and no remote, under the hostile read
    /// profile.
    pub(crate) fn repo(&self, name: &str) -> PathBuf {
        let path = self.root.join(name);
        self.git(
            &self.base,
            &["init", "-q", "-b", "main", &path.to_string_lossy()],
        );
        self.commit(&path, "a.txt", &format!("{name}\n"));
        self.hostile(&path);
        path
    }

    /// A bare origin at `<net>/<name>.git`, which the fixture admits as a network remote.
    pub(crate) fn origin(&self, name: &str) -> PathBuf {
        let origin = self.net.join(format!("{name}.git"));
        self.git(
            &self.base,
            &[
                "init",
                "-q",
                "--bare",
                "-b",
                "main",
                &origin.to_string_lossy(),
            ],
        );
        origin
    }

    /// Add `url` as remote `name` of `repo` and push every branch and tag to it.
    pub(crate) fn push_to(&self, repo: &Path, name: &str, url: &Path) {
        self.git(repo, &["remote", "add", name, &url.to_string_lossy()]);
        self.git(repo, &["push", "-q", name, "--all"]);
        self.git(repo, &["push", "-q", name, "--tags"]);
        self.git(repo, &["fetch", "-q", name]);
    }

    /// A repository at `<root>/<name>` with one commit, pushed in full to a network origin
    /// named `origin`: the shape that is `safe`.
    pub(crate) fn pushed_repo(&self, name: &str) -> PathBuf {
        let path = self.root.join(name);
        self.pushed_repo_at(&path, name);
        path
    }

    /// The same shape at any `path`, its history seeded by `seed` and its origin at
    /// `<net>/<seed>.git` — two seeds are two unrelated histories.
    pub(crate) fn pushed_repo_at(&self, path: &Path, seed: &str) {
        self.git(
            &self.base,
            &["init", "-q", "-b", "main", &path.to_string_lossy()],
        );
        self.commit(path, "a.txt", &format!("{seed}\n"));
        self.hostile(path);
        let origin = self.origin(seed);
        self.push_to(path, "origin", &origin);
    }

    /// A shallow clone at `<root>/<name>`, one commit deep, of a network origin holding two.
    pub(crate) fn shallow_repo(&self, name: &str) -> PathBuf {
        let seed = self.base.join(format!("{name}-seed"));
        self.pushed_repo_at(&seed, name);
        self.commit(&seed, "b.txt", "two\n");
        self.git(&seed, &["push", "-q", "origin", "main"]);
        let url = format!(
            "file://{}{}",
            if cfg!(windows) { "/" } else { "" },
            self.net
                .join(format!("{name}.git"))
                .to_string_lossy()
                .replace('\\', "/")
        );
        let path = self.root.join(name);
        self.git(
            &self.base,
            &["clone", "-q", "--depth", "1", &url, &path.to_string_lossy()],
        );
        path
    }

    /// Register `path` as a location, its row carrying the lineage the scan writes and both
    /// observation clocks set. `common_dir` is recorded as `<path>/.git` when it is one.
    pub(crate) fn register(&self, path: &Path) -> LocationId {
        let lineage = if path.join(".git").exists() {
            super::git_world::scan_lineage(&self.read_git, path)
        } else {
            None
        };
        let common = path.join(".git");
        let common_dir = common
            .is_dir()
            .then(|| common.to_string_lossy().into_owned());
        let id = self
            .index
            .lock()
            .expect("index")
            .with_tx(|tx| {
                tx.execute(
                    "INSERT INTO project (name, seed_basename, lineage_key, created_at,
                                          updated_at)
                     VALUES ('widget', 'widget', ?1, 0, 0)",
                    [&lineage],
                )?;
                let project = tx.last_insert_rowid();
                tx.execute(
                    "INSERT INTO location
                       (project_id, kind, distro, path_bytes, path_key, path_display,
                        store_key, presence, repo_kind, refstate_observed_at,
                        worktree_observed_at, common_dir_bytes)
                     VALUES (?1, 'linux', '', ?2, ?2, ?3, 'store', 'present', 'worktree',
                             ?4, ?4, ?5)",
                    rusqlite::params![
                        project,
                        path.to_string_lossy().as_bytes(),
                        path.to_string_lossy(),
                        NOW - 60,
                        common_dir.map(String::into_bytes)
                    ],
                )?;
                Ok(tx.last_insert_rowid())
            })
            .expect("registered");
        LocationId(id)
    }

    /// Record `common_dir` as the location's `common_dir_bytes`, as a scan of a gitfile would.
    pub(crate) fn set_common_dir(&self, id: LocationId, common_dir: &Path) {
        let guard = self.index.lock().expect("index");
        guard
            .conn()
            .execute(
                "UPDATE location SET common_dir_bytes = ?1 WHERE id = ?2",
                rusqlite::params![common_dir.to_string_lossy().as_bytes(), id.0],
            )
            .expect("common dir");
    }

    /// Re-derive the row's `lineage_key` from `path` as a scan would, after a fixture changed
    /// what the root set reads — a replace ref or a graft changes it, and a row that predates
    /// the change is another repository to §45.6 step 1.
    pub(crate) fn rescan_lineage(&self, id: LocationId, path: &Path) {
        let lineage = super::git_world::scan_lineage(&self.read_git, path);
        let guard = self.index.lock().expect("index");
        guard
            .conn()
            .execute(
                "UPDATE project SET lineage_key = ?1
                  WHERE id = (SELECT project_id FROM location WHERE id = ?2)",
                rusqlite::params![lineage, id.0],
            )
            .expect("rescanned");
    }

    /// The production verifier with the fixture's one difference: an origin under `net/` is a
    /// network remote, read through the real write path.
    pub(crate) fn verifier(&self) -> GitRemoteVerifier<'_> {
        GitRemoteVerifier::with_transport_fixture(
            &self.write_git,
            &self.read_git,
            TransportFixture::new(&self.net),
        )
    }

    /// The seams over `remotes` and `trash`.
    pub(crate) fn seams<'a>(
        &'a self,
        remotes: &'a dyn RemoteVerifier,
        trash: &'a dyn Trash,
    ) -> AnalyserSeams<'a> {
        AnalyserSeams {
            git: &self.read_git,
            remotes,
            trash,
        }
    }

    /// `locations.uninstallPreflight` over `remotes`.
    pub(crate) fn preflight(&self, id: LocationId, remotes: &dyn RemoteVerifier) -> Verdict {
        let trash = CountingTrash::new();
        let value = codotheca_core::uninstall::handle_preflight_off_lock(
            &self.index,
            &self.seams(remotes, &trash),
            serde_json::json!({ "locationId": id }),
            NOW,
        )
        .expect("the pre-flight answers");
        assert_eq!(trash.sends(), 0, "a pre-flight sent something to the trash");
        Verdict(value)
    }

    /// `locations.uninstall` over `remotes` and `trash`; the failure's message on a refusal.
    pub(crate) fn uninstall(
        &self,
        id: LocationId,
        remotes: &dyn RemoteVerifier,
        trash: &dyn Trash,
    ) -> Result<serde_json::Value, String> {
        codotheca_core::uninstall::handle_uninstall_off_lock(
            &self.index,
            &self.seams(remotes, trash),
            serde_json::json!({ "locationId": id }),
            NOW,
        )
        .map_err(|failure| failure.message)
    }
}

/// A verdict as the wire carries it.
#[derive(Debug, Clone)]
pub(crate) struct Verdict(pub(crate) serde_json::Value);

impl Verdict {
    /// `disposition`.
    pub(crate) fn disposition(&self) -> String {
        self.0["disposition"]
            .as_str()
            .expect("a disposition")
            .to_owned()
    }

    /// `blockers`, in the order the verdict lists them.
    pub(crate) fn blockers(&self) -> Vec<String> {
        self.0["blockers"]
            .as_array()
            .expect("blockers")
            .iter()
            .map(|b| b.as_str().expect("a blocker").to_owned())
            .collect()
    }

    /// Does the verdict name `blocker`?
    pub(crate) fn has(&self, blocker: &str) -> bool {
        self.blockers().iter().any(|b| b == blocker)
    }
}

impl std::fmt::Display for Verdict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {:?}", self.disposition(), self.blockers())
    }
}
