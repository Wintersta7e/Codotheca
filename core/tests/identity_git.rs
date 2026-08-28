//! Runs real `git`. Proves the identity ref set (§1.1) finds what it claims to find.
//!
//! Task 5 asserts the argv; this asserts the argv is *right* — that it reaches a root reachable
//! only from a non-`HEAD` branch, which is the entire reason §1.1 states the ref set, and that
//! it does not reach a root that arrived by fetch.
//!
//! **It runs `root_set_argv` itself rather than a copy of it.** The plan wrote the vector out
//! literally, on the reasoning that `codotheca_core` was a binary crate and that a duplicate
//! would catch a later edit to `--all`. The crate has a library target now, and calling the real
//! function is the stronger test of the two: every assertion below is about what git *returns*
//! under that argv, so widening it to `--all` fails
//! `the_stated_ref_set_excludes_remote_tracking_refs` here as well as Task 5's equality — where
//! a literal copy would have gone on agreeing with itself.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::Path;
use std::process::Command;

use codotheca_core::identity::lineage::{parse_root_oids, root_set_argv};

/// Fixture git with its own `HOME`, so the developer's global config — a signing key, a
/// `defaultBranch`, a hooks path — never decides whether this test passes.
fn git_in(dir: &Path, home: &Path, args: &[&str]) -> Vec<u8> {
    let out = Command::new("git")
        .current_dir(dir)
        .args(args)
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", home)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("LC_ALL", "C")
        .env("GIT_AUTHOR_NAME", "Fixture")
        .env("GIT_AUTHOR_EMAIL", "fixture@example.invalid")
        .env("GIT_COMMITTER_NAME", "Fixture")
        .env("GIT_COMMITTER_EMAIL", "fixture@example.invalid")
        .output()
        .expect("git runs");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    out.stdout
}

/// A repository plus the fake home beside it, so neither is inside the other.
struct Fixture {
    dir: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("home")).unwrap();
        std::fs::create_dir_all(dir.path().join("repo")).unwrap();
        Self { dir }
    }

    fn home(&self) -> std::path::PathBuf {
        self.dir.path().join("home")
    }

    fn repo(&self) -> std::path::PathBuf {
        self.dir.path().join("repo")
    }

    fn git(&self, args: &[&str]) -> Vec<u8> {
        git_in(&self.repo(), &self.home(), args)
    }

    fn init(&self) -> &Self {
        self.git(&["init", "-q", "--initial-branch=main", "."]);
        self
    }

    fn commit(&self, file: &str, msg: &str) {
        std::fs::write(self.repo().join(file), msg).unwrap();
        self.git(&["add", file]);
        self.git(&["commit", "-q", "-m", msg]);
    }

    /// The roots the product's own ref set finds, parsed by the product's own parser.
    fn stated_roots(&self) -> Vec<String> {
        parse_root_oids(&self.git(&root_set_argv(true)))
    }
}

#[test]
fn the_stated_ref_set_finds_a_root_that_head_alone_misses() {
    let fx = Fixture::new();
    fx.init();
    fx.commit("a.txt", "first");

    // An orphan branch is a second root, reachable from no ref HEAD can see.
    fx.git(&["checkout", "-q", "--orphan", "side"]);
    // `-f`, not `--cached`: `--orphan` keeps the index *and* the working tree, so clearing only
    // the index leaves `a.txt` on disk as an untracked file and `checkout main` then aborts
    // rather than overwrite it. The plan's version stops here.
    fx.git(&["rm", "-rqf", "."]);
    fx.commit("b.txt", "second-root");
    fx.git(&["checkout", "-q", "main"]);

    let head_only = parse_root_oids(&fx.git(&["rev-list", "--max-parents=0", "HEAD"]));
    let stated = fx.stated_roots();

    assert_eq!(head_only.len(), 1, "HEAD alone sees one root");
    assert_eq!(stated.len(), 2, "the stated ref set sees both roots");
    for oid in &head_only {
        assert!(stated.contains(oid));
    }
}

#[test]
fn the_stated_ref_set_excludes_remote_tracking_refs() {
    // A fetched remote-tracking ref must not enter the root set: it churns on every fetch
    // (§6), and a lineage that moves when the user fetches is not an identity.
    let upstream = Fixture::new();
    upstream.init();
    upstream.commit("u.txt", "upstream-root");

    let clone = Fixture::new();
    clone.init();
    clone.commit("l.txt", "local-root");
    let upstream_path = upstream.repo();
    clone.git(&["remote", "add", "origin", &upstream_path.to_string_lossy()]);
    clone.git(&["fetch", "-q", "origin"]);

    assert_eq!(
        clone.stated_roots().len(),
        1,
        "the fetched root is not part of this repository's identity"
    );

    let with_all = parse_root_oids(&clone.git(&["rev-list", "--max-parents=0", "--all", "--"]));
    assert_eq!(
        with_all.len(),
        2,
        "--all would have swallowed it — which is why it is not used"
    );
}

#[test]
fn a_shallow_clone_reports_its_graft_boundary_as_a_root() {
    // This is the evidence for the ruling that a shallow repository has no lineage: git calls
    // the boundary commit parentless, so --max-parents=0 returns a commit that is not a root.
    let origin = Fixture::new();
    origin.init();
    origin.commit("a.txt", "one");
    origin.commit("b.txt", "two");
    let real_root = origin.stated_roots().first().unwrap().clone();

    let shallow = Fixture::new();
    let target = shallow.dir.path().join("s");
    git_in(
        shallow.dir.path(),
        &shallow.home(),
        &[
            "clone",
            "-q",
            "--depth",
            "1",
            "--no-local",
            &format!("file://{}", origin.repo().to_string_lossy()),
            &target.to_string_lossy(),
        ],
    );
    let is_shallow = git_in(
        &target,
        &shallow.home(),
        &["rev-parse", "--is-shallow-repository"],
    );
    assert_eq!(String::from_utf8_lossy(&is_shallow).trim(), "true");

    let boundary = parse_root_oids(&git_in(&target, &shallow.home(), &root_set_argv(true)));
    assert_eq!(boundary.len(), 1);
    assert_ne!(
        boundary.first().unwrap(),
        &real_root,
        "the boundary is not the root, so it is not a lineage"
    );
}
