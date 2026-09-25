#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! §45.8's snapshot and §45.6 step 9: the content an act was decided over is bound, and any
//! change to it between the verdict and the removal refuses the act.

mod support;

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::Path;

use codotheca_core::analyser::snapshot::{snapshot, Snapshot, SNAPSHOT_FORMAT_VERSION};
use codotheca_core::cancel::CancelToken;
use codotheca_core::git::{JobClass, JobContext, RepoHandle, StoreKey};
use codotheca_core::mount::StoreClass;
use codotheca_core::testing::CountingTrash;
use support::analyser_world::Library;

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

/// One perturbation of a copy.
type Perturb = fn(&Library, &Path);

/// The snapshot of the repository at `path`, through the production read backend.
fn take(lib: &Library, path: &Path) -> Snapshot {
    let cancel = CancelToken::new();
    let repo = RepoHandle::resolve(path, StoreKey::new("store"), StoreClass::Local).expect("repo");
    snapshot(
        &repo,
        &lib.read_git,
        &JobContext::new(JobClass::Interactive, &cancel, None),
    )
    .expect("the snapshot reads")
}

/// **AC-P4-45-19 — the snapshot binds content.** Not through a handler: the snapshot function
/// itself, because the criterion is about its sensitivity. Each perturbation changes the digest;
/// an edit inside junk does not.
#[test]
fn ac_p4_45_19() {
    let lib = Library::new();
    let copy = lib.pushed_repo("widget");
    lib.commit(&copy, "b.txt", "two\n");
    lib.git(&copy, &["tag", "-a", "v1", "-m", "one"]);
    lib.git(&copy, &["checkout", "-q", "--detach"]);
    std::fs::write(copy.join(".git/info/exclude"), b".env\n").expect("exclude");
    std::fs::write(copy.join(".env"), b"TOKEN=one\n").expect(".env");
    std::fs::write(copy.join(".git/hooks/pre-commit"), b"#!/bin/sh\nexit 0\n").expect("hook");
    let nested = copy.join("nested");
    lib.git(
        &lib.base,
        &["init", "-q", "-b", "main", &nested.to_string_lossy()],
    );
    lib.commit(&nested, "n.txt", "one\n");
    let junk = copy.join("node_modules").join("pkg");
    std::fs::create_dir_all(&junk).expect("junk");
    std::fs::write(junk.join("index.js"), b"one\n").expect("junk file");

    let perturbations: [(&str, Perturb); 7] = [
        ("an unstaged edit to a tracked file", |_, repo| {
            std::fs::write(repo.join("a.txt"), b"edited\n").expect("edit");
        }),
        ("an edit to an ignored .env", |_, repo| {
            std::fs::write(repo.join(".env"), b"TOKEN=two\n").expect("edit");
        }),
        ("a new stash entry", |fixture, repo| {
            std::fs::write(repo.join("b.txt"), b"stashed\n").expect("edit");
            fixture.git(repo, &["stash", "push", "-q", "-m", "work", "--", "b.txt"]);
        }),
        ("a detached-HEAD move", |fixture, repo| {
            fixture.git(repo, &["checkout", "-q", "--detach", "HEAD~1"]);
        }),
        ("a replaced tag object", |fixture, repo| {
            fixture.git(repo, &["tag", "-d", "v1"]);
            fixture.git(repo, &["tag", "-a", "v1", "-m", "two"]);
        }),
        ("a nested repository's commit", |fixture, repo| {
            fixture.commit(&repo.join("nested"), "n.txt", "two\n");
        }),
        ("a hook edit", |_, repo| {
            std::fs::write(repo.join(".git/hooks/pre-commit"), b"#!/bin/sh\nexit 1\n")
                .expect("hook");
        }),
    ];
    let mut changed = 0;
    let mut before = take(&lib, &copy);
    for (name, perturb) in perturbations {
        perturb(&lib, &copy);
        let after = take(&lib, &copy);
        eprintln!(
            "snapshot, {name}: {} → {}",
            before.digest().get(..12).unwrap(),
            after.digest().get(..12).unwrap()
        );
        assert_ne!(
            before.digest(),
            after.digest(),
            "{name} left the digest unchanged"
        );
        changed += 1;
        before = after;
    }
    assert_ne!(
        before.digest(),
        before.digest_at(SNAPSHOT_FORMAT_VERSION + 1),
        "a format-version change must change the digest"
    );
    changed += 1;

    std::fs::write(junk.join("index.js"), b"two\n").expect("junk edit");
    let junk_edit = take(&lib, &copy);
    eprintln!(
        "snapshot: {changed} perturbations changed the digest; an edit inside junk left it {}",
        if junk_edit.digest() == before.digest() {
            "unchanged"
        } else {
            "CHANGED"
        }
    );
    assert_eq!(
        junk_edit.digest(),
        before.digest(),
        "junk is not in the snapshot"
    );
    assert!(
        junk_edit.repos.contains_key("nested"),
        "the nested repository is in the snapshot"
    );
    assert_eq!(changed, 8);
}

/// **AC-P4-45-17's Uninstall half — re-read before the act.** A hook between step 8 and step 9
/// edits an ignored precious file, adds a stash entry, moves a detached `HEAD`, swaps the
/// directory — in turn, each on a copy that was `safe`. The act refuses each time, the trash is
/// sent nothing, and the copy is byte-identical to what the perturbation left.
#[test]
fn ac_p4_45_17() {
    let perturbations: [(&str, Perturb); 4] = [
        ("edits an ignored precious file", |_, repo| {
            std::fs::write(repo.join(".env"), b"TOKEN=local\n").expect(".env");
        }),
        ("adds a stash entry", |fixture, repo| {
            std::fs::write(repo.join("a.txt"), b"stashed\n").expect("edit");
            fixture.git(repo, &["stash", "push", "-q", "-m", "work"]);
        }),
        ("moves a detached HEAD", |fixture, repo| {
            fixture.git(repo, &["checkout", "-q", "--detach", "HEAD~1"]);
        }),
        ("swaps the directory", |fixture, repo| {
            std::fs::rename(repo, fixture.base.join("swapped-out")).expect("swap out");
            fixture.pushed_repo_at(repo, "replacement");
        }),
    ];
    let mut refused = 0;
    for (name, perturb) in perturbations {
        let lib = Library::new();
        let copy = lib.pushed_repo("widget");
        lib.commit(&copy, "b.txt", "two\n");
        lib.git(&copy, &["push", "-q", "origin", "main"]);
        lib.git(&copy, &["checkout", "-q", "--detach"]);
        std::fs::write(copy.join(".git/info/exclude"), b".env\n").expect("exclude");
        let id = lib.register(&copy);
        let verifier = lib.verifier();
        assert_eq!(
            lib.preflight(id, &verifier).disposition(),
            "safe",
            "{name}: the fixture must be removable"
        );

        let left: RefCell<BTreeMap<String, Vec<u8>>> = RefCell::new(BTreeMap::new());
        let hook = || {
            perturb(&lib, &copy);
            *left.borrow_mut() = tree_bytes(&copy);
        };
        let trash = CountingTrash::new();
        let outcome = lib.uninstall_with_hook(id, &verifier, &trash, &hook);
        eprintln!(
            "re-read before the act, {name}: {outcome:?}; {} send(s)",
            trash.sends()
        );
        assert!(outcome.is_err(), "{name}: the act went through");
        assert_eq!(trash.sends(), 0, "{name}: the trash was sent the copy");
        assert_eq!(
            tree_bytes(&copy),
            *left.borrow(),
            "{name}: the copy changed under a refused act"
        );
        refused += 1;
    }
    eprintln!("re-read before the act: {refused} of 4 perturbations refused");
    assert_eq!(refused, 4);
}
