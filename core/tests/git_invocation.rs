#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! §3.2: argv only, never a shell, and every option that neutralises the user's config.

use std::ffi::OsString;
use std::path::Path;
use std::process::Command;

use codotheca_core::git::{
    base_args, ensure_empty_hooks_dir, neutralise_env, RepoHandle, StoreKey,
};
use codotheca_core::mount::StoreClass;

fn handle(dir: &Path) -> RepoHandle {
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    RepoHandle::resolve(dir, StoreKey::new("s0"), StoreClass::Local).unwrap()
}

fn strings(v: &[OsString]) -> Vec<String> {
    v.iter().map(|s| s.to_string_lossy().into_owned()).collect()
}

#[test]
fn the_argv_carries_every_neutralising_option() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = handle(tmp.path());
    let hooks = tmp.path().join("hooks-empty");
    let args = strings(&base_args(&repo, &hooks));

    assert_eq!(args[0], "-C");
    assert_eq!(args[1], repo.work_dir.to_string_lossy());
    for expected in [
        "--no-optional-locks",
        "core.fsmonitor=false",
        "protocol.ext.allow=never",
        "diff.external=",
        "core.askPass=",
        "credential.helper=",
    ] {
        assert!(
            args.iter().any(|a| a == expected),
            "missing {expected} in {args:?}"
        );
    }
    assert!(args
        .iter()
        .any(|a| a == &format!("core.hooksPath={}", hooks.to_string_lossy())));
    assert_eq!(args.iter().filter(|a| *a == "-c").count(), 6);
}

#[test]
fn safe_directory_appears_only_for_a_trusted_location() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = handle(tmp.path());
    let hooks = tmp.path().join("hooks-empty");

    let untrusted = strings(&base_args(&repo, &hooks));
    assert!(!untrusted.iter().any(|a| a.starts_with("safe.directory=")));

    let trusted = repo.clone().with_trust(true);
    let args = strings(&base_args(&trusted, &hooks));
    assert!(args
        .iter()
        .any(|a| a == &format!("safe.directory={}", trusted.work_dir.to_string_lossy())));
}

#[test]
fn the_environment_is_neutralised() {
    let mut cmd = Command::new("git");
    neutralise_env(&mut cmd);
    let set: Vec<(String, Option<String>)> = cmd
        .get_envs()
        .map(|(k, v)| {
            (
                k.to_string_lossy().into_owned(),
                v.map(|v| v.to_string_lossy().into_owned()),
            )
        })
        .collect();

    let get = |k: &str| set.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
    assert_eq!(get("GIT_TERMINAL_PROMPT"), Some(Some("0".into())));
    assert_eq!(get("GIT_CONFIG_NOSYSTEM"), Some(Some("1".into())));
    assert_eq!(get("LC_ALL"), Some(Some("C".into())));
    // Redirect variables are removed, not overwritten: `None` is `env_remove`.
    for k in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    ] {
        assert_eq!(get(k), Some(None), "{k} should be removed");
    }
}

#[test]
fn a_dot_git_file_resolves_to_its_real_git_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let real = tmp.path().join("real-gitdir");
    std::fs::create_dir_all(&real).unwrap();
    let work = tmp.path().join("work");
    std::fs::create_dir_all(&work).unwrap();
    std::fs::write(
        work.join(".git"),
        format!("gitdir: {}\n", real.to_string_lossy()),
    )
    .unwrap();

    let repo = RepoHandle::resolve(&work, StoreKey::new("s0"), StoreClass::Local).unwrap();
    assert_eq!(repo.git_dir, real);
    assert_eq!(
        repo.common_dir, real,
        "no commondir file means the git dir is the common dir"
    );
    assert!(!repo.is_linked_worktree());
}

#[test]
fn a_linked_worktree_keeps_its_own_git_dir_and_shares_the_common_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let common = tmp.path().join("parent").join(".git");
    let linked = common.join("worktrees").join("wt1");
    std::fs::create_dir_all(&linked).unwrap();
    std::fs::write(linked.join("commondir"), "../..\n").unwrap();
    let work = tmp.path().join("wt1");
    std::fs::create_dir_all(&work).unwrap();
    std::fs::write(
        work.join(".git"),
        format!("gitdir: {}\n", linked.to_string_lossy()),
    )
    .unwrap();

    let repo = RepoHandle::resolve(&work, StoreKey::new("s0"), StoreClass::Local).unwrap();
    assert_eq!(repo.git_dir, linked);
    assert_eq!(
        repo.common_dir.canonicalize().unwrap(),
        common.canonicalize().unwrap()
    );
    assert!(repo.is_linked_worktree());
}

#[test]
fn a_bare_repository_uses_the_git_dir_as_its_work_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = RepoHandle::bare(tmp.path(), StoreKey::new("s0"), StoreClass::Network);
    assert_eq!(repo.work_dir, repo.git_dir);
    assert_eq!(repo.common_dir, repo.git_dir);
    assert_eq!(repo.store_class, StoreClass::Network);
}

#[test]
fn the_hooks_directory_is_created_and_empty() {
    let tmp = tempfile::tempdir().unwrap();
    let hooks = ensure_empty_hooks_dir(tmp.path()).unwrap();
    assert!(hooks.is_dir());
    assert_eq!(std::fs::read_dir(&hooks).unwrap().count(), 0);
    // Idempotent, and it does not delete a foreign file it did not create.
    let again = ensure_empty_hooks_dir(tmp.path()).unwrap();
    assert_eq!(again, hooks);
}
