#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! §3.2: argv only, never a shell, and every option that neutralises the user's config.

mod support;

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

    let trusted = repo.with_trust(true);
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

/// §47.3: **no scrub variable reaches a read child**, whatever the parent's environment holds.
///
/// The hostile variables are set on a re-executed child of this test binary, never on this
/// process: `set_var` here would race every other test. The child runs the **production**
/// `GitExec` against the recording stand-in, which writes the environment it actually received;
/// this half reads that file. The config file `GIT_CONFIG_GLOBAL` names is kept (§47.3), so it
/// must arrive — a scrub that removed everything would pass the first half alone.
#[cfg(feature = "testkit")]
#[test]
fn no_scrub_variable_reaches_a_read_child_under_a_hostile_parent() {
    use codotheca_core::cancel::CancelToken;
    use codotheca_core::git::{GitExec, RunLimits};
    use support::git_world::{
        child_dir, hostile_parent_env, is_child, read_recording, recording_git, run_in_child,
        scrub_report,
    };

    if is_child() {
        let dir = child_dir();
        let work = dir.join("repo");
        let repo = handle(&work);
        let hooks = ensure_empty_hooks_dir(&dir).unwrap();
        GitExec::new(recording_git(), hooks)
            .run(
                &repo,
                &[std::ffi::OsStr::new("status")],
                RunLimits::none(),
                &CancelToken::new(),
            )
            .expect("the recording stand-in exits 0");
        return;
    }

    let tmp = tempfile::tempdir().unwrap();
    let hostile = hostile_parent_env(tmp.path());
    run_in_child(
        "no_scrub_variable_reaches_a_read_child_under_a_hostile_parent",
        tmp.path(),
        &hostile.all(),
    );
    let recorded = read_recording(&tmp.path().join("repo"));
    let report = scrub_report(&hostile, &recorded.env);
    eprintln!(
        "read-path scrub: {} planted, {} scrubbed variables reached the child {:?}, {} kept \
         variables lost {:?}",
        report.planted,
        report.leaked.len(),
        report.leaked,
        report.lost.len(),
        report.lost
    );
    assert!(
        report.planted > 0,
        "nothing was planted, so nothing was proved"
    );
    assert!(
        report.leaked.is_empty(),
        "scrubbed variables reached the read child: {:?}",
        report.leaked
    );
    assert!(
        report.lost.is_empty(),
        "the user's own config route must be kept: {:?}",
        report.lost
    );
}

/// R237: **one hooks directory name.** Both production backends are built in `main.rs`, where a
/// second spelling (`empty-hooks` against `EMPTY_HOOKS_DIR_NAME`'s `git-hooks-empty`) once pointed
/// the app at a directory nothing created. Neither backend exposes its directory after
/// construction, so the rule is read off the source: no hooks literal, and the constant used.
#[test]
fn the_read_and_write_hooks_directories_are_one() {
    let main = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("main.rs"),
    )
    .unwrap();
    assert!(!main.is_empty(), "core/src/main.rs read as empty");
    let literals: Vec<&str> = main
        .split('"')
        .skip(1)
        .step_by(2)
        .filter(|literal| literal.contains("hooks"))
        .collect();
    let uses = main.matches("EMPTY_HOOKS_DIR_NAME").count();
    eprintln!(
        "main.rs: {} bytes read, {} hooks literal(s) {literals:?}, {uses} use(s) of \
         EMPTY_HOOKS_DIR_NAME",
        main.len(),
        literals.len()
    );
    assert!(
        literals.is_empty(),
        "main.rs spells a hooks directory as a literal: {literals:?}"
    );
    assert!(
        uses >= 1,
        "main.rs must build the hooks directory from EMPTY_HOOKS_DIR_NAME"
    );
}
