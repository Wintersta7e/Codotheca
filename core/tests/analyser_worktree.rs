#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! §45.6 steps 4 and 5 through the handlers: ignored and untracked files, junk, the rows `status`
//! cannot see, and every nested repository — each fixture a pushed copy under the hostile read
//! profile, spoiled once.

mod support;

use std::path::Path;

use codotheca_core::cancel::CancelToken;
use codotheca_core::git::{GitBackend as _, JobClass, JobContext, RepoHandle, StoreKey};
use codotheca_core::mount::StoreClass;
use codotheca_core::testing::FixtureRemoteVerifier;
use support::analyser_world::{Library, Verdict};
use support::git_world::{read_recordings, recording_git};

/// A pushed copy spoiled by `spoil`, registered and pre-flighted.
fn spoiled(spoil: impl FnOnce(&Library, &Path)) -> Verdict {
    let lib = Library::new();
    let copy = lib.pushed_repo("widget");
    spoil(&lib, &copy);
    let id = lib.register(&copy);
    lib.preflight(id, &lib.verifier())
}

/// Ignore `pattern` through the repository's own exclude file — no commit, so nothing unpushed.
fn ignore(copy: &Path, pattern: &str) {
    let exclude = copy.join(".git/info/exclude");
    let mut text = std::fs::read_to_string(&exclude).unwrap_or_default();
    text.push_str(pattern);
    text.push('\n');
    std::fs::write(exclude, text).expect("exclude");
}

/// A package tree a build would recreate.
fn node_modules(copy: &Path, name: &str) {
    let dir = copy.join(name).join("pkg");
    std::fs::create_dir_all(&dir).expect("junk");
    std::fs::write(dir.join("index.js"), b"module.exports = 1;\n").expect("junk file");
}

/// The precious entries' `pathDisplay`s.
fn precious(verdict: &Verdict) -> Vec<String> {
    verdict.0["precious"]["entries"]
        .as_array()
        .map(|entries| {
            entries
                .iter()
                .map(|e| e["pathDisplay"].as_str().expect("path").to_owned())
                .collect()
        })
        .unwrap_or_default()
}

/// The `status` argv the analyser's worktree read renders, recorded by the stand-in.
fn recorded_status_argv() -> Vec<String> {
    let dir = tempfile::tempdir().expect("tempdir");
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(repo.join(".git")).expect("repo");
    let hooks =
        codotheca_core::git::ensure_empty_hooks_dir(&dir.path().join("app")).expect("hooks");
    let git = codotheca_core::git::SystemGit::new(
        std::sync::Arc::new(codotheca_core::git::GitExec::new(recording_git(), hooks)),
        std::sync::Arc::new(codotheca_core::git::GitSlots::for_machine()),
        std::sync::Arc::new(codotheca_core::clock::SystemClock::new()),
    );
    let cancel = CancelToken::new();
    let handle =
        RepoHandle::resolve(&repo, StoreKey::new("s"), StoreClass::Local).expect("resolves");
    let _ = git.worktree_scan(
        &handle,
        &JobContext::new(JobClass::Interactive, &cancel, None),
    );
    read_recordings(&repo)
        .into_iter()
        .map(|call| call.argv)
        .find(|argv| argv.iter().any(|a| a == "status"))
        .expect("a status call was recorded")
}

/// **AC-P4-45-9 — ignored, untracked, junk.**
#[test]
fn ac_p4_45_9() {
    let env_file = spoiled(|lib, copy| {
        let _ = lib;
        ignore(copy, ".env");
        std::fs::write(copy.join(".env"), b"TOKEN=local\n").expect(".env");
    });
    let ignored_junk = spoiled(|_, copy| {
        ignore(copy, "node_modules/");
        node_modules(copy, "node_modules");
    });
    let unignored_junk = spoiled(|_, copy| node_modules(copy, "node_modules"));
    let nested_in_junk = spoiled(|lib, copy| {
        let inner = copy.join("node_modules").join("x");
        lib.git(
            &lib.base,
            &["init", "-q", "-b", "main", &inner.to_string_lossy()],
        );
        lib.commit(&inner, "local.txt", "only here\n");
    });
    let build_file = spoiled(|_, copy| {
        std::fs::write(copy.join("build"), b"a file, not a directory\n").expect("build");
    });
    let near_miss = spoiled(|_, copy| node_modules(copy, "Node_modules"));
    let argv = recorded_status_argv();

    eprintln!(
        "an ignored .env: {env_file}, precious {:?}",
        precious(&env_file)
    );
    eprintln!("an ignored node_modules/: {ignored_junk}; unignored: {unignored_junk}");
    eprintln!(
        "node_modules/x/.git with a local commit: {nested_in_junk}; nested {}",
        nested_in_junk.0["nested"]
    );
    eprintln!("a file named build: {build_file}; Node_modules/: {near_miss}");
    eprintln!("the status argv: {argv:?}");

    assert!(env_file.has("ignored_precious"), "{env_file}");
    assert!(precious(&env_file).contains(&".env".to_owned()));
    assert_eq!(ignored_junk.disposition(), "safe", "{ignored_junk}");
    assert_eq!(unignored_junk.disposition(), "safe", "{unignored_junk}");
    assert!(nested_in_junk.has("submodule_unsafe"), "{nested_in_junk}");
    let nested = nested_in_junk.0["nested"].as_array().expect("nested");
    assert!(
        nested.iter().any(|n| n["pathDisplay"] == "node_modules/x"
            && n["kind"] == "independent"
            && n["disposition"] == "blocked"),
        "{nested:?}"
    );
    assert!(build_file.has("untracked_precious"), "{build_file}");
    assert!(near_miss.has("untracked_precious"), "{near_miss}");
    assert!(argv.iter().any(|a| a == "-unormal"), "{argv:?}");
    assert!(argv.iter().any(|a| a == "--ignored=matching"), "{argv:?}");
    assert!(!argv.iter().any(|a| a == "-uall"), "{argv:?}");
}

/// A parent pushed to its origin, with a submodule `sub` whose own origin is under `net/`.
fn with_submodule(lib: &Library) -> std::path::PathBuf {
    let sub_seed = lib.base.join("sub-seed");
    lib.pushed_repo_at(&sub_seed, "sub");
    let copy = lib.pushed_repo("widget");
    lib.git(
        &copy,
        &[
            "submodule",
            "add",
            "-q",
            &lib.net.join("sub.git").to_string_lossy(),
            "sub",
        ],
    );
    lib.git(&copy, &["commit", "-q", "-m", "add sub"]);
    lib.git(&copy, &["push", "-q", "origin", "main"]);
    lib.git(&copy, &["fetch", "-q", "origin"]);
    copy
}

/// **AC-P4-45-10 — nesting.** A checked-out submodule with a local-only commit is `submodule`
/// `blocked`; a de-initialised `.git/modules/<n>` with a local-only branch is `module_gitdir`
/// `blocked`; a submodule whose only remote is unreachable leaves the parent `unknown`; depth 4
/// is `nesting_too_deep`.
#[test]
fn ac_p4_45_10() {
    let checked_out = {
        let lib = Library::new();
        let copy = with_submodule(&lib);
        lib.commit(&copy.join("sub"), "s.txt", "local in the submodule\n");
        let id = lib.register(&copy);
        lib.preflight(id, &lib.verifier())
    };
    let deinitialised = {
        let lib = Library::new();
        let copy = with_submodule(&lib);
        let sub = copy.join("sub");
        lib.git(&sub, &["checkout", "-q", "-b", "local"]);
        lib.commit(&sub, "s.txt", "local branch\n");
        lib.git(&sub, &["checkout", "-q", "--detach", "HEAD~1"]);
        lib.git(&copy, &["submodule", "deinit", "-q", "-f", "sub"]);
        let id = lib.register(&copy);
        lib.preflight(id, &lib.verifier())
    };
    let offline = {
        let lib = Library::new();
        let copy = with_submodule(&lib);
        lib.git(
            &copy.join("sub"),
            &["remote", "rename", "origin", "upstream"],
        );
        let id = lib.register(&copy);
        let head = lib.git(&copy, &["rev-parse", "HEAD"]).trim().to_owned();
        let remotes = FixtureRemoteVerifier::new();
        remotes.answering("origin", &[head]);
        remotes.silent(
            "upstream",
            codotheca_core::analyser::remote::DidNotAnswer::Failed,
        );
        lib.preflight(id, &remotes)
    };
    let too_deep = {
        let lib = Library::new();
        let copy = lib.pushed_repo("widget");
        let mut dir = copy.clone();
        for level in ["n1", "n2", "n3", "n4"] {
            dir = dir.join(level);
            lib.git(
                &lib.base,
                &["init", "-q", "-b", "main", &dir.to_string_lossy()],
            );
        }
        let id = lib.register(&copy);
        lib.preflight(id, &lib.verifier())
    };
    let kinds = |verdict: &Verdict| verdict.0["nested"].clone();
    eprintln!(
        "a checked-out submodule: {checked_out}; {}",
        kinds(&checked_out)
    );
    eprintln!(
        "a de-initialised module: {deinitialised}; {}",
        kinds(&deinitialised)
    );
    eprintln!(
        "an unreachable submodule remote: {offline}; {}",
        kinds(&offline)
    );
    eprintln!("depth 4: {too_deep}; {}", kinds(&too_deep));

    let has_nested = |verdict: &Verdict, kind: &str, disposition: &str| {
        verdict.0["nested"].as_array().is_some_and(|n| {
            n.iter()
                .any(|e| e["kind"] == kind && e["disposition"] == disposition)
        })
    };
    assert!(
        has_nested(&checked_out, "submodule", "blocked"),
        "{checked_out}"
    );
    assert!(checked_out.has("submodule_unsafe"), "{checked_out}");
    assert!(
        has_nested(&deinitialised, "module_gitdir", "blocked"),
        "{deinitialised}"
    );
    assert!(deinitialised.has("submodule_unsafe"), "{deinitialised}");
    assert_eq!(offline.disposition(), "unknown", "{offline}");
    assert!(offline.has("remote_unreachable"), "{offline}");
    assert!(too_deep.has("nesting_too_deep"), "{too_deep}");
    assert_eq!(too_deep.disposition(), "unknown", "{too_deep}");
}

/// **AC-P4-45-13 — the hidden rows.** An assume-unchanged file edited on disk and a
/// skip-worktree file present on disk are `hidden_from_status`; a staged-then-modified file is
/// `uncommitted_changes`; a non-sample hook is `untracked_precious`; a non-empty
/// `.git/lfs/objects` is `lfs_unverified`, which is undischargeable for Uninstall.
#[test]
fn ac_p4_45_13() {
    type Spoil = fn(&Library, &Path);
    let cases: [(&str, Spoil, &str); 5] = [
        (
            "an assume-unchanged file edited on disk",
            |lib, copy| {
                lib.git(copy, &["update-index", "--assume-unchanged", "a.txt"]);
                std::fs::write(copy.join("a.txt"), b"edited where status cannot see\n")
                    .expect("edit");
            },
            "hidden_from_status",
        ),
        (
            "a skip-worktree file present on disk",
            |lib, copy| {
                lib.git(copy, &["update-index", "--skip-worktree", "a.txt"]);
            },
            "hidden_from_status",
        ),
        (
            "a staged-then-modified file",
            |lib, copy| {
                std::fs::write(copy.join("a.txt"), b"staged\n").expect("stage");
                lib.git(copy, &["add", "a.txt"]);
                std::fs::write(copy.join("a.txt"), b"modified again\n").expect("modify");
            },
            "uncommitted_changes",
        ),
        (
            "a non-sample hook",
            |_, copy| {
                std::fs::write(copy.join(".git/hooks/pre-commit"), b"#!/bin/sh\nexit 0\n")
                    .expect("hook");
            },
            "untracked_precious",
        ),
        (
            "a non-empty .git/lfs/objects",
            |_, copy| {
                let objects = copy.join(".git/lfs/objects/ab/cd");
                std::fs::create_dir_all(&objects).expect("lfs");
                std::fs::write(objects.join("abcd"), b"a large file\n").expect("object");
            },
            "lfs_unverified",
        ),
    ];
    let mut produced = 0;
    for (name, spoil, expected) in cases {
        let verdict = spoiled(spoil);
        eprintln!("hidden row, {name}: {verdict}");
        assert!(verdict.has(expected), "{name}: {verdict}");
        assert_ne!(verdict.disposition(), "safe", "{name}: {verdict}");
        produced += 1;
    }
    assert_eq!(produced, 5);
}
