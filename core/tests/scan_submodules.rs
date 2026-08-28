//! Compiled only under `testkit`: these link `codotheca_core::testing`, which the feature gates.
#![cfg(feature = "testkit")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

//! §4.2's last bullet and §4.4. The walk stops at a repository root, so a submodule is never
//! reached by descent; it is enumerated from `.gitmodules` and becomes **its own project** with
//! an edge, never a location of the parent.

use codotheca_core::cancel::CancelToken;
use codotheca_core::git::StoreKey;
use codotheca_core::mount::StoreClass;
use codotheca_core::scan::discover::ProbeCtx;
use codotheca_core::scan::submodules::{
    enumerate_submodules, is_safe_submodule_path, parse_gitmodules, GITMODULES_BYTE_CAP,
};
use codotheca_core::scan::{ScanProblemKind, WalkEvent, WalkOptions};
use codotheca_core::testing::{FakeGitBackend, GitReply};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

fn probe<'a>(git: &'a FakeGitBackend, cancel: &'a CancelToken) -> ProbeCtx<'a> {
    ProbeCtx::new(git, StoreKey::new("store-a"), StoreClass::Local, cancel)
}

fn repo_dir(path: &Path) {
    std::fs::create_dir_all(path.join(".git")).unwrap();
}

#[test]
fn the_parser_takes_path_keys_in_file_order() {
    let text = b"[submodule \"a\"]\n\tpath = vendor/a\n\turl = https://example.invalid/a\n\
                 [submodule \"b\"]\n\tpath=lib/b\n\turl = https://example.invalid/b\n";
    assert_eq!(
        parse_gitmodules(text),
        vec![b"vendor/a".to_vec(), b"lib/b".to_vec()]
    );
}

#[test]
fn the_parser_ignores_keys_outside_a_submodule_section() {
    let text = b"[core]\n\tpath = not-a-submodule\n[submodule \"a\"]\n\tpath = a\n";
    assert_eq!(parse_gitmodules(text), vec![b"a".to_vec()]);
}

/// `.gitmodules` is repository content and can say anything. A submodule path is a path *within*
/// the parent, so anything that could leave it is rejected outright rather than normalised.
#[test]
fn escaping_paths_are_rejected() {
    assert!(is_safe_submodule_path(b"vendor/a"));
    assert!(!is_safe_submodule_path(b"../outside"));
    assert!(!is_safe_submodule_path(b"vendor/../../outside"));
    assert!(!is_safe_submodule_path(b"/etc"));
    assert!(!is_safe_submodule_path(b""));
}

#[test]
fn an_initialised_submodule_becomes_its_own_repository_and_one_edge() {
    let base = tempfile::tempdir().unwrap();
    let parent = base.path();
    repo_dir(parent);
    std::fs::write(
        parent.join(".gitmodules"),
        b"[submodule \"a\"]\n\tpath = vendor/a\n",
    )
    .unwrap();
    let child = parent.join("vendor/a");
    repo_dir(&child);

    let oid = "8f1c2b3d4e5f60718293a4b5c6d7e8f901234567".to_owned();
    let mut links = BTreeMap::new();
    links.insert(b"vendor/a".to_vec(), oid.clone());
    let git = FakeGitBackend::new();
    git.always_submodule_gitlinks(GitReply::Ok(links));
    let cancel = CancelToken::new();

    let repos = Mutex::new(Vec::new());
    let edges = enumerate_submodules(
        parent,
        &WalkOptions::default(),
        &probe(&git, &cancel),
        true,
        &|event| {
            if let WalkEvent::Repo(c) = event {
                repos.lock().unwrap().push(c.path);
            }
        },
    );

    assert_eq!(edges.len(), 1);
    let edge = edges.first().unwrap();
    assert_eq!(edge.parent_worktree, parent);
    assert_eq!(edge.child_worktree, child);
    assert_eq!(edge.path_bytes, b"vendor/a".to_vec());
    assert_eq!(edge.gitlink_oid.as_deref(), Some(oid.as_str()));
    assert_eq!(repos.into_inner().unwrap(), vec![child]);
}

/// §1.9: `gitlink_oid` is nullable. An unreadable parent index leaves it absent — **never zero
/// and never a placeholder OID**, which would name a commit that does not exist.
#[test]
fn a_gitlink_that_cannot_be_read_leaves_the_oid_absent_never_zero() {
    let base = tempfile::tempdir().unwrap();
    let parent = base.path();
    repo_dir(parent);
    std::fs::write(
        parent.join(".gitmodules"),
        b"[submodule \"a\"]\n\tpath = vendor/a\n",
    )
    .unwrap();
    repo_dir(&parent.join("vendor/a"));

    // The fake has no reply configured, so `submodule_gitlinks` fails.
    let git = FakeGitBackend::new();
    let cancel = CancelToken::new();
    let edges = enumerate_submodules(
        parent,
        &WalkOptions::default(),
        &probe(&git, &cancel),
        true,
        &|_| {},
    );
    assert_eq!(edges.len(), 1);
    assert_eq!(edges.first().and_then(|e| e.gitlink_oid.clone()), None);
}

#[test]
fn an_uninitialised_submodule_produces_no_edge_and_no_repository() {
    let base = tempfile::tempdir().unwrap();
    let parent = base.path();
    repo_dir(parent);
    std::fs::write(
        parent.join(".gitmodules"),
        b"[submodule \"a\"]\n\tpath = vendor/a\n",
    )
    .unwrap();
    std::fs::create_dir_all(parent.join("vendor/a")).unwrap(); // empty, never cloned
    let git = FakeGitBackend::new();
    let cancel = CancelToken::new();
    let edges = enumerate_submodules(
        parent,
        &WalkOptions::default(),
        &probe(&git, &cancel),
        true,
        &|_| {},
    );
    assert!(edges.is_empty());
}

#[test]
fn a_nested_submodule_is_reached_through_its_parent() {
    let base = tempfile::tempdir().unwrap();
    let parent = base.path();
    repo_dir(parent);
    std::fs::write(
        parent.join(".gitmodules"),
        b"[submodule \"a\"]\n\tpath = a\n",
    )
    .unwrap();
    let mid = parent.join("a");
    repo_dir(&mid);
    std::fs::write(mid.join(".gitmodules"), b"[submodule \"b\"]\n\tpath = b\n").unwrap();
    repo_dir(&mid.join("b"));

    let git = FakeGitBackend::new();
    let cancel = CancelToken::new();
    let edges = enumerate_submodules(
        parent,
        &WalkOptions::default(),
        &probe(&git, &cancel),
        true,
        &|_| {},
    );
    let children: Vec<PathBuf> = edges.iter().map(|e| e.child_worktree.clone()).collect();
    assert_eq!(children, vec![mid.clone(), mid.join("b")]);
}

/// §10.1 promised the user a 256 KB cap on the four root files it names. A larger one is
/// reported, not read.
#[test]
fn an_oversized_gitmodules_is_reported_and_not_read() {
    let base = tempfile::tempdir().unwrap();
    let parent = base.path();
    repo_dir(parent);
    let mut bytes = b"[submodule \"a\"]\n\tpath = a\n".to_vec();
    bytes.resize(usize::try_from(GITMODULES_BYTE_CAP).unwrap() + 1, b'#');
    std::fs::write(parent.join(".gitmodules"), bytes).unwrap();
    repo_dir(&parent.join("a"));

    let git = FakeGitBackend::new();
    let cancel = CancelToken::new();
    let problems = Mutex::new(Vec::new());
    let edges = enumerate_submodules(
        parent,
        &WalkOptions::default(),
        &probe(&git, &cancel),
        true,
        &|event| {
            if let WalkEvent::Problem(p) = event {
                problems.lock().unwrap().push(p);
            }
        },
    );
    assert!(edges.is_empty());
    let problems = problems.into_inner().unwrap();
    assert_eq!(problems.len(), 1);
    assert_eq!(
        problems.first().map(|p| p.kind),
        Some(ScanProblemKind::UnreadableRepo)
    );
}

/// A repository with no `.gitmodules` at all costs nothing and reports nothing.
#[test]
fn a_repository_with_no_gitmodules_enumerates_nothing_quietly() {
    let base = tempfile::tempdir().unwrap();
    let parent = base.path();
    repo_dir(parent);
    let git = FakeGitBackend::new();
    let cancel = CancelToken::new();
    let problems = Mutex::new(Vec::new());
    let edges = enumerate_submodules(
        parent,
        &WalkOptions::default(),
        &probe(&git, &cancel),
        true,
        &|event| {
            if let WalkEvent::Problem(p) = event {
                problems.lock().unwrap().push(p);
            }
        },
    );
    assert!(edges.is_empty());
    assert!(problems.into_inner().unwrap().is_empty());
    assert!(git.calls().is_empty());
}

/// §4.4: a submodule is a repository in its own right, so a bare-shaped probe has no place here
/// and the root's `bare_candidates` setting must not leak into the child.
#[test]
fn a_submodule_path_is_never_probed_as_a_bare_candidate() {
    let base = tempfile::tempdir().unwrap();
    let parent = base.path();
    repo_dir(parent);
    std::fs::write(
        parent.join(".gitmodules"),
        b"[submodule \"a\"]\n\tpath = a\n",
    )
    .unwrap();
    // Bare-shaped, but not a checked-out submodule.
    let child = parent.join("a");
    std::fs::create_dir_all(child.join("objects")).unwrap();
    std::fs::create_dir_all(child.join("refs")).unwrap();
    std::fs::write(child.join("HEAD"), b"ref: refs/heads/main\n").unwrap();

    let git = FakeGitBackend::new();
    let cancel = CancelToken::new();
    let edges = enumerate_submodules(
        parent,
        &WalkOptions {
            bare_candidates: true,
            ..WalkOptions::default()
        },
        &probe(&git, &cancel),
        true,
        &|_| {},
    );
    assert!(edges.is_empty());
    assert!(
        !git.calls().iter().any(|c| c.op == "repo_facts"),
        "no bare probe may run for a declared submodule path"
    );
}
