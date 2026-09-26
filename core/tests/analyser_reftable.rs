#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! **AC-P4-45-3 — reftable** (`manual`, R207: its record is the Windows-native gate's). A
//! `--ref-format=reftable` copy is read through git, never through files: a local-only branch
//! and a local annotated tag are `unpushed_commits` and `unpushed_tag`, two stashes are
//! `stash_present` and both are in the snapshot, and below the governed floor the same copy is
//! `refs_unreadable` with **zero** verifying reads. Where the git under test cannot create a
//! reftable repository the test prints why and counts as not run.
//!
//! The stashes are their own copy: a stash is undischargeable for Uninstall, so beside one no
//! verifying read runs (AC-P4-45-16) and the remote-composed blockers cannot appear.

mod support;

use std::path::Path;
use std::process::{Command, Output};

use codotheca_core::cancel::CancelToken;
use codotheca_core::git::{GitVersion, JobClass, JobContext, RepoHandle, StoreKey};
use codotheca_core::mount::StoreClass;
use codotheca_core::testing::{CountingTrash, FixtureRemoteVerifier, RecordingGitBackend};
use support::analyser_world::{Library, Verdict, NOW};

/// `git <args>` from the git under test, which is the one that can make a reftable repository.
fn under_test(lib: &Library, cwd: &Path, args: &[&str]) -> Output {
    let mut cmd = Command::new(support::test_git());
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
        .env("HOME", &lib.home)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", lib.home.join("empty.gitconfig"))
        .env("GIT_AUTHOR_NAME", "Fixture")
        .env("GIT_AUTHOR_EMAIL", "fixture@example.invalid")
        .env("GIT_COMMITTER_NAME", "Fixture")
        .env("GIT_COMMITTER_EMAIL", "fixture@example.invalid")
        .args([
            "-c",
            "commit.gpgsign=false",
            "-c",
            "protocol.file.allow=always",
        ])
        .args(args)
        .output()
        .expect("git under test")
}

fn ok(lib: &Library, cwd: &Path, args: &[&str]) -> String {
    let out = under_test(lib, cwd, args);
    assert!(
        out.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// A reftable copy at `<root>/widget`, pushed to a network origin. `None` when this git cannot
/// make one.
fn reftable_copy(lib: &Library) -> Option<std::path::PathBuf> {
    let copy = lib.root.join("widget");
    let init = under_test(
        lib,
        &lib.base,
        &[
            "init",
            "-q",
            "-b",
            "main",
            "--ref-format=reftable",
            &copy.to_string_lossy(),
        ],
    );
    if !init.status.success() {
        eprintln!(
            "skipped: {} has no reftable ({})",
            support::git_world::test_git_version(),
            String::from_utf8_lossy(&init.stderr).trim()
        );
        return None;
    }
    std::fs::write(copy.join("a.txt"), b"one\n").expect("file");
    ok(lib, &copy, &["add", "a.txt"]);
    ok(lib, &copy, &["commit", "-q", "-m", "one"]);
    let origin = lib.net.join("widget.git");
    ok(
        lib,
        &lib.base,
        &[
            "init",
            "-q",
            "--bare",
            "-b",
            "main",
            &origin.to_string_lossy(),
        ],
    );
    ok(
        lib,
        &copy,
        &["remote", "add", "origin", &origin.to_string_lossy()],
    );
    ok(lib, &copy, &["push", "-q", "origin", "main"]);
    ok(lib, &copy, &["fetch", "-q", "origin"]);
    assert!(copy.join(".git/reftable").is_dir(), "the copy is reftable");
    Some(copy)
}

/// AC-P4-45-3's shape on the git under test. Untagged: the criterion is graded from the
/// Windows-native record alone (R207), so this run is not its coverage.
#[test]
fn reftable_refs_and_stashes_are_read_through_git() {
    // A local-only branch and a local annotated tag.
    let lib = Library::new();
    let Some(copy) = reftable_copy(&lib) else {
        return;
    };
    ok(&lib, &copy, &["checkout", "-q", "-b", "feature"]);
    std::fs::write(copy.join("f.txt"), b"local only\n").expect("file");
    ok(&lib, &copy, &["add", "f.txt"]);
    ok(&lib, &copy, &["commit", "-q", "-m", "local only"]);
    ok(&lib, &copy, &["checkout", "-q", "main"]);
    ok(&lib, &copy, &["tag", "-a", "local", "-m", "only here"]);
    let id = lib.register(&copy);
    let roots = lib.preflight(id, &lib.verifier());

    // Below the governed floor: nothing but `refs_unreadable`, and no verifying read.
    let below = {
        let recording = RecordingGitBackend::with_version(
            lib.read_git.clone(),
            GitVersion {
                major: 2,
                minor: 28,
                patch: 0,
                raw: "git version 2.28.0".to_owned(),
            },
        );
        let remotes = FixtureRemoteVerifier::new();
        let trash = CountingTrash::new();
        let seams = codotheca_core::analyser::AnalyserSeams {
            git: &recording,
            remotes: &remotes,
            trash: &trash,
            before_act: None,
        };
        let value = codotheca_core::uninstall::handle_preflight_off_lock(
            &lib.index,
            &seams,
            serde_json::json!({ "locationId": id }),
            NOW,
        )
        .expect("the pre-flight answers");
        (Verdict(value), remotes.calls())
    };

    // Two stashes, on their own reftable copy.
    let stashed = Library::new();
    let Some(stash_copy) = reftable_copy(&stashed) else {
        return;
    };
    for n in 1..=2 {
        std::fs::write(stash_copy.join("a.txt"), format!("stash {n}\n")).expect("edit");
        ok(
            &stashed,
            &stash_copy,
            &["stash", "push", "-q", "-m", &format!("s{n}")],
        );
    }
    let stash_id = stashed.register(&stash_copy);
    let stash_verdict = stashed.preflight(stash_id, &stashed.verifier());
    let cancel = CancelToken::new();
    let handle =
        RepoHandle::resolve(&stash_copy, StoreKey::new("store"), StoreClass::Local).expect("repo");
    let snapshot = codotheca_core::analyser::snapshot::snapshot(
        &handle,
        &stashed.read_git,
        &JobContext::new(JobClass::Interactive, &cancel, None),
    )
    .expect("snapshot");
    let stash_entries = snapshot.repos[""].stash.len();

    eprintln!(
        "reftable on {}: a local branch and tag {roots}; below the floor {} with {} verifier \
         call(s); two stashes {stash_verdict}, {stash_entries} in the snapshot",
        support::git_world::test_git_version(),
        below.0,
        below.1.len()
    );
    assert!(roots.has("unpushed_commits"), "{roots}");
    assert!(roots.has("unpushed_tag"), "{roots}");
    assert_ne!(roots.disposition(), "safe");
    assert_eq!(below.0.blockers(), vec!["refs_unreadable".to_owned()]);
    assert!(below.1.is_empty(), "{:?}", below.1);
    assert!(stash_verdict.has("stash_present"), "{stash_verdict}");
    assert_ne!(stash_verdict.disposition(), "safe");
    assert_eq!(stash_entries, 2);
}
