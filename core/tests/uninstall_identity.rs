#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! §45.6 step 1 through the handlers: **the directory is compared with its row, never with
//! itself.** Phase 2 built the uninstall warrant's expected identity from the directory it was
//! about to check (§37.8), so a working copy replaced by another repository matched itself and
//! went to the trash.
//!
//! Every copy here is under the hostile read profile, read by the real read backend. The remote
//! is answered by a scripted write seam that advertises exactly what the copy holds, so a copy
//! that passes step 1 is `safe` — which is what makes a refusal here a refusal of identity and
//! nothing else.

mod support;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};

use codotheca_core::cancel::CancelToken;
use codotheca_core::git::{GitError, GitResult};
use codotheca_core::gitw::{Intent, MutatingGit, RunOutput, VerifyStepKind};
use codotheca_core::index::Index;
use codotheca_core::testing::CountingTrash;
use support::git_world::HOSTILE_READ_PROFILE;

const NOW: i64 = 1_750_000_000;

/// Fixture git in `cwd`, isolated from the developer's config.
fn git(cwd: &Path, home: &Path, args: &[&str]) -> String {
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
    let out = cmd
        .current_dir(cwd)
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", home)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", home.join("empty.gitconfig"))
        .env("GIT_AUTHOR_NAME", "Fixture")
        .env("GIT_AUTHOR_EMAIL", "fixture@example.invalid")
        .env("GIT_COMMITTER_NAME", "Fixture")
        .env("GIT_COMMITTER_EMAIL", "fixture@example.invalid")
        .args(["-c", "commit.gpgsign=false", "-c", "core.autocrlf=false"])
        .args(args)
        .output()
        .expect("fixture git");
    assert!(
        out.status.success(),
        "fixture git {args:?} in {}: {}",
        cwd.display(),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// A write seam whose remote holds everything the copy holds: `ls-remote --get-url` names an
/// https URL, and the advertisement is the copy's own branches and tags, all present locally.
#[derive(Debug)]
struct AnsweringRemote {
    home: PathBuf,
}

impl MutatingGit for AnsweringRemote {
    fn run(
        &self,
        intent: &Intent,
        _cancel: &CancelToken,
        _on_stderr: &mut dyn FnMut(&str),
    ) -> GitResult<RunOutput> {
        let Intent::VerifyRead { repo, step, .. } = intent else {
            return Err(GitError::Internal {
                detail: "only the verifying read is scripted".to_owned(),
            });
        };
        let stdout = match step.kind() {
            VerifyStepKind::ResolveUrl => "https://forge.example/acme/widget.git\n".to_owned(),
            VerifyStepKind::Advertise => git(
                repo,
                &self.home,
                &[
                    "for-each-ref",
                    "--format=%(objectname)%09%(refname)",
                    "refs/heads",
                    "refs/tags",
                ],
            ),
            VerifyStepKind::Objects => String::new(),
        };
        Ok(RunOutput {
            stdout: stdout.into_bytes(),
        })
    }
}

/// One scan root holding one working copy, a row for it, and the seams the handlers take.
struct Fixture {
    _dir: tempfile::TempDir,
    base: PathBuf,
    home: PathBuf,
    copy: PathBuf,
    index: Arc<Mutex<Index>>,
    location: i64,
    read_git: codotheca_core::git::SystemGit,
    remote: AnsweringRemote,
}

impl Fixture {
    /// A clean, fully pushed copy of a fresh history whose files say `seed`, at `<root>/widget`,
    /// its row carrying the lineage the scan's own derivation writes.
    fn new(seed: &str) -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path().canonicalize().expect("canonical");
        let home = base.join("home");
        let root = base.join("root");
        std::fs::create_dir_all(&home).expect("home");
        std::fs::create_dir_all(&root).expect("root");
        std::fs::write(home.join("empty.gitconfig"), b"").expect("global");
        let copy = root.join("widget");
        clone_history(&base, &home, seed, &copy, None);

        let hooks =
            codotheca_core::git::ensure_empty_hooks_dir(&base.join("app-data")).expect("hooks");
        let read_git = codotheca_core::git::SystemGit::new(
            Arc::new(codotheca_core::git::GitExec::new(
                support::test_git(),
                hooks,
            )),
            Arc::new(codotheca_core::git::GitSlots::for_machine()),
            Arc::new(codotheca_core::clock::SystemClock::new()),
        );
        let lineage = support::git_world::scan_lineage(&read_git, &copy);
        let index = Arc::new(Mutex::new(
            Index::open_at(&base.join("index"), NOW).expect("index"),
        ));
        let location = {
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
                            worktree_observed_at)
                         VALUES (?1, 'linux', '', ?2, ?2, ?3, 'store', 'present', 'worktree',
                                 ?4, ?4)",
                        rusqlite::params![
                            project,
                            copy.to_string_lossy().as_bytes(),
                            copy.to_string_lossy(),
                            NOW - 60
                        ],
                    )?;
                    Ok(tx.last_insert_rowid())
                })
                .expect("seeded")
        };
        Self {
            _dir: dir,
            remote: AnsweringRemote { home: home.clone() },
            base,
            home,
            copy,
            index,
            location,
            read_git,
        }
    }

    fn preflight(&self) -> serde_json::Value {
        codotheca_core::uninstall::handle_preflight_off_lock(
            &self.index,
            &self.read_git,
            &self.remote,
            serde_json::json!({ "locationId": self.location }),
            NOW,
        )
        .expect("the pre-flight answers")
    }

    fn uninstall(&self, trash: &CountingTrash) -> Result<serde_json::Value, String> {
        codotheca_core::uninstall::handle_uninstall_off_lock(
            &self.index,
            &self.read_git,
            &self.remote,
            trash,
            serde_json::json!({ "locationId": self.location }),
            NOW,
        )
        .map_err(|failure| failure.message)
    }

    /// Another history, cloned to `at`.
    fn other_copy(&self, seed: &str, at: &Path) {
        clone_history(&self.base, &self.home, seed, at, None);
    }
}

/// A seed history of two commits whose files say `seed`, a bare origin of it, and a clone of the
/// origin at `at` (`depth` commits deep, or whole), under the hostile read profile.
fn clone_history(base: &Path, home: &Path, seed: &str, at: &Path, depth: Option<&str>) {
    let src = base.join(format!("seed-{seed}"));
    if !src.exists() {
        git(
            base,
            home,
            &["init", "-q", "-b", "main", &src.to_string_lossy()],
        );
        for n in 1..=2 {
            std::fs::write(src.join("a.txt"), format!("{seed} {n}\n")).expect("seed file");
            git(&src, home, &["add", "a.txt"]);
            git(&src, home, &["commit", "-q", "-m", &format!("{seed} {n}")]);
        }
        let origin = base.join(format!("{seed}.git"));
        git(
            base,
            home,
            &[
                "clone",
                "-q",
                "--bare",
                &src.to_string_lossy(),
                &origin.to_string_lossy(),
            ],
        );
    }
    let origin = base.join(format!("{seed}.git"));
    let url = format!(
        "file://{}{}",
        if cfg!(windows) { "/" } else { "" },
        origin.to_string_lossy().replace('\\', "/")
    );
    let mut args = vec!["clone", "-q"];
    if let Some(depth) = depth {
        args.extend(["--depth", depth]);
    }
    let at_text = at.to_string_lossy().into_owned();
    args.extend([url.as_str(), at_text.as_str()]);
    git(base, home, &args);
    for (key, value) in HOSTILE_READ_PROFILE {
        git(at, home, &["config", key, value]);
    }
    git(at, home, &["update-index", "--untracked-cache"]);
    git(at, home, &["status", "--porcelain"]);
}

/// Every file under `dir`, by relative path, with its bytes.
fn tree_bytes(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(base: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
        for entry in std::fs::read_dir(dir).expect("read_dir").flatten() {
            let path = entry.path();
            let kind = entry.file_type().expect("file type");
            if kind.is_dir() {
                walk(base, &path, out);
            } else if kind.is_file() {
                let rel = path.strip_prefix(base).expect("under base");
                out.insert(
                    rel.to_string_lossy().into_owned(),
                    std::fs::read(&path).expect("read"),
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(dir, dir, &mut out);
    out
}

fn blockers(verdict: &serde_json::Value) -> Vec<String> {
    verdict["blockers"]
        .as_array()
        .expect("blockers")
        .iter()
        .map(|b| b.as_str().expect("a blocker").to_owned())
        .collect()
}

/// **AC-P4-45-2.** The directory is replaced by a repository of a different lineage between the
/// pre-flight and the act. The act refuses at step 1, the trash is sent **nothing**, and the
/// replacement is byte-identical afterwards. Put back, the original goes through — so the
/// refusal was identity's and not a verdict that was never going to be safe.
#[test]
fn a_replaced_directory_is_refused_through_the_handler() {
    let fx = Fixture::new("original");
    let before = fx.preflight();
    eprintln!(
        "the original: {} {:?}",
        before["disposition"],
        blockers(&before)
    );
    assert_eq!(
        before["disposition"], "safe",
        "the fixture must be removable for a refusal to mean anything: {before}"
    );

    let aside = fx.base.join("original-aside");
    std::fs::rename(&fx.copy, &aside).expect("move the original aside");
    fx.other_copy("replacement", &fx.copy);
    let replacement = tree_bytes(&fx.copy);

    let trash = CountingTrash::new();
    let outcome = fx.uninstall(&trash);
    eprintln!(
        "the act on the replacement: {outcome:?}; {} send(s)",
        trash.sends()
    );
    assert_eq!(trash.sends(), 0, "the trash was sent a replaced directory");
    let refused = outcome.expect_err("a replaced directory is refused");
    assert!(refused.contains("RefusedPath"), "{refused}");
    assert_eq!(
        tree_bytes(&fx.copy),
        replacement,
        "the replacement changed under a refused act"
    );

    let verdict = fx.preflight();
    eprintln!(
        "the replacement's pre-flight: {} {:?}",
        verdict["disposition"],
        blockers(&verdict)
    );
    assert_eq!(verdict["disposition"], "blocked");
    assert_eq!(blockers(&verdict), vec!["refused_path".to_owned()]);

    std::fs::rename(&fx.copy, fx.base.join("replacement-aside")).expect("move it aside");
    std::fs::rename(&aside, &fx.copy).expect("put the original back");
    let removed = fx.uninstall(&trash);
    eprintln!(
        "the act on the original: {}; {} send(s)",
        removed
            .as_ref()
            .map_or_else(Clone::clone, |_| "removed".to_owned()),
        trash.sends()
    );
    assert!(removed.is_ok(), "{removed:?}");
    assert_eq!(trash.sends(), 1);
}

/// The directory unchanged and **the row's `lineage_key` changed instead**: the same refusal,
/// because the comparison is with the row.
#[test]
fn a_changed_row_lineage_is_refused_through_the_handler() {
    let fx = Fixture::new("original");
    {
        let guard = fx.index.lock().expect("index");
        guard
            .conn()
            .execute("UPDATE project SET lineage_key = ?1", [&"f".repeat(64)])
            .expect("lineage changed");
    }
    let unchanged = tree_bytes(&fx.copy);
    let verdict = fx.preflight();
    assert_eq!(blockers(&verdict), vec!["refused_path".to_owned()]);
    let trash = CountingTrash::new();
    let refused = fx.uninstall(&trash).expect_err("refused");
    eprintln!(
        "a changed row lineage: {refused}; {} send(s)",
        trash.sends()
    );
    assert_eq!(trash.sends(), 0);
    assert_eq!(tree_bytes(&fx.copy), unchanged);
}

/// **Two NULLs match only when the live root set is empty and not shallow.** An empty
/// repository matches a NULL row and no other; a repository with commits never matches a NULL
/// row.
#[test]
fn an_empty_live_root_set_matches_only_a_null_row() {
    let fx = Fixture::new("original");
    let set_row = |lineage: Option<&str>| {
        let guard = fx.index.lock().expect("index");
        guard
            .conn()
            .execute("UPDATE project SET lineage_key = ?1", [lineage])
            .expect("row lineage");
    };

    // Commits against a NULL row.
    set_row(None);
    let with_commits = blockers(&fx.preflight());
    assert!(
        with_commits.contains(&"refused_path".to_owned()),
        "{with_commits:?}"
    );

    // No commits against a NULL row, then against a lineage.
    std::fs::rename(&fx.copy, fx.base.join("aside")).expect("aside");
    git(
        &fx.base,
        &fx.home,
        &["init", "-q", "-b", "main", &fx.copy.to_string_lossy()],
    );
    let empty_null = blockers(&fx.preflight());
    set_row(Some(&"e".repeat(64)));
    let empty_keyed = blockers(&fx.preflight());
    eprintln!(
        "commits vs NULL {with_commits:?}; empty vs NULL {empty_null:?}; empty vs a lineage \
         {empty_keyed:?}"
    );
    assert!(
        !empty_null.contains(&"refused_path".to_owned()),
        "{empty_null:?}"
    );
    assert!(
        !empty_null.contains(&"refs_unreadable".to_owned()),
        "{empty_null:?}"
    );
    assert_eq!(empty_keyed, vec!["refused_path".to_owned()]);
}

/// **A live shallow copy yields `shallow_clone`, never `refused_path`** (D-2): it has no
/// lineage by construction, so step 1 cannot match it and must not call it another repository.
#[test]
fn a_live_shallow_copy_yields_shallow_clone() {
    let fx = Fixture::new("original");
    std::fs::rename(&fx.copy, fx.base.join("aside")).expect("aside");
    clone_history(&fx.base, &fx.home, "original", &fx.copy, Some("1"));
    let verdict = fx.preflight();
    let found = blockers(&verdict);
    eprintln!("a live shallow copy: {} {found:?}", verdict["disposition"]);
    assert!(found.contains(&"shallow_clone".to_owned()), "{found:?}");
    assert!(!found.contains(&"refused_path".to_owned()), "{found:?}");
    let trash = CountingTrash::new();
    assert!(fx.uninstall(&trash).is_err());
    assert_eq!(trash.sends(), 0);
}
