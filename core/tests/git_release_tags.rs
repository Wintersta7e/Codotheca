#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! §38.8's tag facts and the archive reachability observation, read through git over real
//! repositories.
//!
//! Both reads fail closed: a tag list or a reachability answer git could not produce in full is
//! an error, never a shorter list and never `true`. *Absent* is not *unreadable*.

mod support;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use codotheca_core::cancel::CancelToken;
use codotheca_core::clock::SystemClock;
use codotheca_core::git::{
    ensure_empty_hooks_dir, GitBackend, GitExec, GitResult, GitSlots, JobClass, JobContext,
    RepoHandle, StoreKey, SystemGit, TagFact, TagKind,
};
use codotheca_core::mount::StoreClass;

const AUTHOR: &str = "author@example.invalid";
/// Non-ASCII, so an output encoding other than UTF-8 would change the bytes a read sees.
const COMMITTER: &str = "pé@example.invalid";
const TAGGER: &str = "tagger@example.invalid";

/// A repository in a temp dir with its own `HOME`, built by the `PATH` git with fixed identities.
struct Repo {
    _dir: tempfile::TempDir,
    root: PathBuf,
    work: PathBuf,
}

impl Repo {
    /// A temp dir with a `HOME` and no repository yet.
    fn empty() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = support::canonical(dir.path());
        std::fs::create_dir_all(root.join("home")).unwrap();
        Self {
            _dir: dir,
            work: root.join("repo"),
            root,
        }
    }

    /// `git init` of the repository with `flags`, as git answered it.
    fn init_with(&self, flags: &[&str]) -> std::process::Output {
        let mut args = vec!["init", "-q", "-b", "main"];
        args.extend_from_slice(flags);
        args.push("repo");
        self.command(&self.root, &[], &args).output().unwrap()
    }

    fn init() -> Self {
        let repo = Self::empty();
        assert!(repo.init_with(&[]).status.success(), "git init");
        repo
    }

    fn command(&self, cwd: &Path, env: &[(&str, &str)], args: &[&str]) -> Command {
        let home = self.root.join("home");
        let mut cmd = Command::new("git");
        cmd.current_dir(cwd)
            .env("HOME", &home)
            .env("XDG_CONFIG_HOME", &home)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("LC_ALL", "C")
            .env("GIT_AUTHOR_NAME", "Author")
            .env("GIT_AUTHOR_EMAIL", AUTHOR)
            .env("GIT_COMMITTER_NAME", "Author")
            .env("GIT_COMMITTER_EMAIL", AUTHOR)
            .envs(env.iter().copied())
            .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
            .args(["-c", "gc.auto=0"])
            .args(args);
        cmd
    }

    fn git_with(&self, env: &[(&str, &str)], args: &[&str]) -> String {
        let out = self.command(&self.work, env, args).output().unwrap();
        assert!(
            out.status.success(),
            "fixture git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    }

    fn git(&self, args: &[&str]) -> String {
        self.git_with(&[], args)
    }

    fn commit(&self, env: &[(&str, &str)], message: &str) -> String {
        self.git_with(env, &["commit", "-q", "--allow-empty", "-m", message]);
        self.rev("HEAD")
    }

    fn rev(&self, name: &str) -> String {
        self.git(&["rev-parse", name]).trim().to_owned()
    }

    /// A bare remote beside the repository, as `origin`, holding `main`.
    fn push_to_remote(&self) {
        let out = self
            .command(&self.root, &[], &["init", "-q", "--bare", "remote.git"])
            .output()
            .unwrap();
        assert!(out.status.success(), "bare remote");
        let remote = self.root.join("remote.git");
        self.git(&["remote", "add", "origin", remote.to_str().unwrap()]);
        self.git(&["push", "-q", "origin", "main"]);
    }

    /// Delete a loose object, as a prune or a damaged disk would.
    fn delete_object(&self, oid: &str) {
        let (dir, file) = oid.split_at(2);
        let path = self.work.join(".git/objects").join(dir).join(file);
        assert!(path.exists(), "the fixture's object must be loose");
        let mut perms = std::fs::metadata(&path).unwrap().permissions();
        // Windows refuses to delete a read-only file; git writes objects read-only.
        #[allow(clippy::permissions_set_readonly_false)]
        perms.set_readonly(false);
        std::fs::set_permissions(&path, perms).unwrap();
        std::fs::remove_file(&path).unwrap();
    }

    fn backend(&self) -> SystemGit {
        let hooks = ensure_empty_hooks_dir(&self.root.join("app-data")).unwrap();
        SystemGit::new(
            Arc::new(GitExec::new(support::test_git(), hooks)),
            Arc::new(GitSlots::for_machine()),
            Arc::new(SystemClock::new()),
        )
    }

    fn handle(&self) -> RepoHandle {
        RepoHandle::resolve(&self.work, StoreKey::new("s"), StoreClass::Local).unwrap()
    }

    fn tags(&self) -> GitResult<Vec<TagFact>> {
        let cancel = CancelToken::new();
        let ctx = JobContext::new(JobClass::Background, &cancel, None);
        self.backend().release_tags(&self.handle(), &ctx)
    }

    fn reach(&self, detached_head: Option<&str>, extra_excluded: &[String]) -> GitResult<bool> {
        let cancel = CancelToken::new();
        let ctx = JobContext::new(JobClass::Background, &cancel, None);
        self.backend()
            .archive_reach(&self.handle(), detached_head, extra_excluded, &ctx)
    }
}

/// Two tags on one date, a lightweight tag on a commit authored at `-0800` and committed at
/// `+0000` by a non-ASCII committer, an annotated tag by a `+0530` tagger, and a tag on a tree.
struct Tagged {
    repo: Repo,
    first: String,
    second: String,
    annotated: String,
}

fn tagged() -> Tagged {
    let repo = Repo::init();
    let first = repo.commit(
        &[
            ("GIT_AUTHOR_DATE", "2024-01-02T03:04:05+00:00"),
            ("GIT_COMMITTER_DATE", "2024-01-02T03:04:05+00:00"),
        ],
        "first",
    );
    repo.git(&["tag", "same-a"]);
    repo.git(&["tag", "same-b"]);
    let second = repo.commit(
        &[
            ("GIT_AUTHOR_DATE", "2024-02-03T23:30:00-08:00"),
            ("GIT_COMMITTER_DATE", "2024-02-05T10:00:00+00:00"),
            ("GIT_COMMITTER_EMAIL", COMMITTER),
        ],
        "second",
    );
    repo.git(&["tag", "light"]);
    repo.git_with(
        &[
            ("GIT_COMMITTER_EMAIL", TAGGER),
            ("GIT_COMMITTER_DATE", "2024-03-04T05:06:07+05:30"),
        ],
        &["tag", "-a", "annotated", "-m", "a release"],
    );
    let tree = repo.rev("HEAD:");
    repo.git(&["tag", "on-a-tree", &tree]);
    let annotated = repo.rev("refs/tags/annotated");
    Tagged {
        repo,
        first,
        second,
        annotated,
    }
}

fn lightweight(name: &str, commit: &str, email: &str, at: i64, tz: i32) -> TagFact {
    TagFact {
        name: name.to_owned(),
        object_oid: commit.to_owned(),
        commit_oid: commit.to_owned(),
        kind: TagKind::Lightweight {
            committer_email: email.to_owned(),
            authored_at: at,
            tz_offset_min: tz,
        },
    }
}

/// In ref-name byte order; the tag on a tree peels to no commit and is absent.
fn expected(t: &Tagged) -> Vec<TagFact> {
    vec![
        TagFact {
            name: "annotated".to_owned(),
            object_oid: t.annotated.clone(),
            commit_oid: t.second.clone(),
            kind: TagKind::Annotated {
                tagger_email: TAGGER.to_owned(),
                // 2024-03-03T23:36:07Z, the tagger's own +0530.
                tagged_at: 1_709_508_967,
                tz_offset_min: 330,
            },
        },
        // 2024-02-04T07:30:00Z: author time and the author's -0800, not the committer's +0000.
        lightweight("light", &t.second, COMMITTER, 1_707_031_800, -480),
        lightweight("same-a", &t.first, AUTHOR, 1_704_164_645, 0),
        lightweight("same-b", &t.first, AUTHOR, 1_704_164_645, 0),
    ]
}

#[test]
fn tag_facts_carry_the_tagger_or_the_author_date_and_offset() {
    let t = tagged();
    assert_eq!(t.repo.tags().expect("tags read"), expected(&t));
}

/// The repository's own config set to lie to a naive read: signatures shown, colour forced,
/// abbreviated ids, a sort order, and a log output encoding that re-codes the non-ASCII email.
#[test]
fn a_hostile_config_yields_the_same_facts() {
    let t = tagged();
    for (key, value) in [
        ("log.showSignature", "true"),
        ("color.ui", "always"),
        ("core.abbrev", "5"),
        ("tag.sort", "-v:refname"),
        ("i18n.logOutputEncoding", "ISO-8859-1"),
    ] {
        t.repo.git(&["config", key, value]);
    }
    assert_eq!(t.repo.tags().expect("tags read"), expected(&t));
}

/// A tag whose commit is gone is unreadable, not a tag that peels to no commit.
#[test]
fn a_tag_whose_commit_is_missing_is_an_error_never_a_skip() {
    let repo = Repo::init();
    repo.commit(&[], "first");
    let tree = repo.rev("HEAD:");
    let orphan = repo.git(&["commit-tree", &tree, "-p", "HEAD", "-m", "gone"]);
    let orphan = orphan.trim();
    repo.git(&["tag", "-a", "dangling", orphan, "-m", "a release"]);
    repo.delete_object(orphan);

    let read = repo.tags();
    assert!(
        read.is_err(),
        "a tag over a missing commit read as {read:?}"
    );
}

/// A tag object whose tagger line does not parse names nobody the identity set could match, as a
/// tag with no tagger line does: it is left out, and the well-formed tag beside it is still read.
#[test]
fn a_tagger_line_that_does_not_parse_is_skipped_never_an_error() {
    let repo = Repo::init();
    let commit = repo.commit(&[], "first");
    repo.git_with(
        &[
            ("GIT_COMMITTER_EMAIL", TAGGER),
            ("GIT_COMMITTER_DATE", "2024-03-04T05:06:07+05:30"),
        ],
        &["tag", "-a", "good", "-m", "a release"],
    );
    // Written past git's own checks; `broken` sorts before `good`, so a skip that lost its place
    // would date `good` by the wrong object.
    let file = repo.root.join("broken-tag");
    std::fs::write(
        &file,
        format!(
            "object {commit}\ntype commit\ntag broken\ntagger Nobody <{TAGGER}> yesterday +0000\n\na release\n"
        ),
    )
    .unwrap();
    let broken = repo.git(&[
        "hash-object",
        "-t",
        "tag",
        "--literally",
        "-w",
        file.to_str().unwrap(),
    ]);
    repo.git(&["update-ref", "refs/tags/broken", broken.trim()]);

    let good = TagFact {
        name: "good".to_owned(),
        object_oid: repo.rev("refs/tags/good"),
        commit_oid: commit,
        kind: TagKind::Annotated {
            tagger_email: TAGGER.to_owned(),
            tagged_at: 1_709_508_967,
            tz_offset_min: 330,
        },
    };
    assert_eq!(repo.tags().expect("tags read"), [good]);
}

/// A repository with no tags has none — an empty list that was read, not a failure.
#[test]
fn no_tags_is_an_empty_list() {
    let repo = Repo::init();
    repo.commit(&[], "first");
    assert_eq!(repo.tags().expect("tags read"), []);
}

/// Where the git under test cannot create a reftable repository, the test prints why and counts
/// as not run.
#[test]
fn a_reftable_repository_reads_its_tags() {
    let repo = Repo::empty();
    let init = repo.init_with(&["--ref-format=reftable"]);
    if !init.status.success() {
        eprintln!(
            "skipped: this git has no reftable ({})",
            String::from_utf8_lossy(&init.stderr)
                .lines()
                .next()
                .unwrap_or_default()
        );
        return;
    }
    let commit = repo.commit(&[], "first");
    repo.git(&["tag", "v1"]);
    let tags = repo.tags().expect("tags read");
    assert_eq!(
        tags.iter()
            .map(|t| (t.name.as_str(), t.commit_oid.as_str()))
            .collect::<Vec<_>>(),
        [("v1", commit.as_str())]
    );
}

/// A repository whose `main` and a tag on it are both on the remote.
fn archived() -> Repo {
    let repo = Repo::init();
    repo.git(&["commit", "-q", "--allow-empty", "-m", "first"]);
    repo.git(&["tag", "v1"]);
    repo.push_to_remote();
    repo
}

#[test]
fn every_local_tip_on_the_remote_is_archived() {
    assert!(archived().reach(None, &[]).expect("reach read"));
}

#[test]
fn a_local_only_branch_is_not_archived() {
    let repo = archived();
    repo.git(&["checkout", "-q", "-b", "feature"]);
    repo.commit(&[], "local only");
    assert!(!repo.reach(None, &[]).expect("reach read"));
}

/// A detached `HEAD` is a local tip only when the caller says it is detached.
#[test]
fn a_detached_head_on_a_local_only_commit_is_not_archived() {
    let repo = archived();
    repo.git(&["checkout", "-q", "--detach"]);
    let detached = repo.commit(&[], "detached");
    assert!(!repo.reach(Some(&detached), &[]).expect("reach read"));
    assert!(
        repo.reach(None, &[]).expect("reach read"),
        "without the detached head every branch and tag tip is on the remote"
    );
}

#[test]
fn a_stash_alone_is_not_a_local_tip() {
    let repo = Repo::init();
    std::fs::write(repo.work.join("a.txt"), b"one\n").unwrap();
    repo.git(&["add", "a.txt"]);
    repo.commit(&[], "first");
    repo.push_to_remote();
    std::fs::write(repo.work.join("a.txt"), b"two\n").unwrap();
    repo.git(&["stash", "-q"]);
    assert!(
        !repo.rev("refs/stash").is_empty(),
        "the fixture holds a stash"
    );
    assert!(repo.reach(None, &[]).expect("reach read"));
}

#[test]
fn no_remote_is_not_archived() {
    let repo = Repo::init();
    repo.commit(&[], "first");
    assert!(!repo.reach(None, &[]).expect("reach read"));
}

#[test]
fn extra_excluded_tips_cover_the_locals() {
    let repo = Repo::init();
    let tip = repo.commit(&[], "first");
    assert!(repo.reach(None, &[tip]).expect("reach read"));
}

#[test]
fn no_local_tip_is_archived() {
    assert!(Repo::init().reach(None, &[]).expect("reach read"));
}

/// A walk that meets a missing object cannot say what is reachable: `Err`, never `true`.
#[test]
fn a_pruned_object_is_an_error_never_archived() {
    let repo = archived();
    let pruned = repo.commit(&[], "second");
    repo.commit(&[], "third");
    repo.delete_object(&pruned);

    let read = repo.reach(None, &[]);
    assert!(
        read.is_err(),
        "a walk over a pruned object read as {read:?}"
    );
}
