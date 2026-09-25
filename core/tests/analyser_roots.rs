#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! §45.2 rows 1–4 and §45.6 steps 3 and 7, through the handlers: every root namespace, every
//! broken shape, grafts and replace refs, and one walk whatever the ref count.
//!
//! Each fixture is a copy pushed in full to a network origin the production verifier reads over
//! the fixture transport, **plus one thing only this copy holds** — so the copy is `safe` without
//! it, and whatever the verdict says is that one thing's.

mod support;

use std::fmt::Write as _;
use std::path::Path;

use codotheca_core::analyser::remote::GitRemoteVerifier;
use codotheca_core::testing::{CountingTrash, RecordingGitBackend};
use support::analyser_world::{Library, Verdict};

/// A commit only this repository holds: `HEAD`'s tree under a new message, parented on `HEAD`,
/// named by no ref.
fn local_commit(lib: &Library, repo: &Path, message: &str) -> String {
    let tree = lib.git(repo, &["log", "-1", "--format=%T", "HEAD"]);
    lib.git(
        repo,
        &["commit-tree", tree.trim(), "-p", "HEAD", "-m", message],
    )
    .trim()
    .to_owned()
}

/// A pushed copy, registered, its pre-flight `safe` — the control every case below spoils once.
fn pushed(lib: &Library) -> (std::path::PathBuf, codotheca_core::protocol::LocationId) {
    let copy = lib.pushed_repo("widget");
    let id = lib.register(&copy);
    let control = lib.preflight(id, &lib.verifier());
    assert_eq!(control.disposition(), "safe", "the control: {control}");
    (copy, id)
}

/// **AC-P4-45-4 — every root namespace.** Each of these, local-only, blocks alone: a detached
/// `HEAD` commit, `refs/stash`, a `refs/replace/*` target, a `refs/original/*` ref, a custom
/// `refs/x/*` ref and a note.
#[test]
fn every_root_namespace_blocks_alone() {
    type Spoil = fn(&Library, &Path);
    let cases: [(&str, Spoil, &str); 6] = [
        (
            "a detached HEAD commit",
            |lib, repo| {
                let local = local_commit(lib, repo, "detached");
                lib.git(repo, &["checkout", "-q", "--detach", &local]);
            },
            "unpushed_commits",
        ),
        (
            "refs/stash",
            |lib, repo| {
                std::fs::write(repo.join("a.txt"), b"stashed\n").expect("edit");
                lib.git(repo, &["stash", "push", "-q", "-m", "work"]);
            },
            "stash_present",
        ),
        (
            "a refs/replace/* target",
            |lib, repo| {
                let head = lib.git(repo, &["rev-parse", "HEAD"]);
                let local = local_commit(lib, repo, "replacement");
                lib.git(repo, &["replace", head.trim(), &local]);
            },
            "unpushed_commits",
        ),
        (
            "a refs/original/* ref",
            |lib, repo| {
                let local = local_commit(lib, repo, "original");
                lib.git(
                    repo,
                    &["update-ref", "refs/original/refs/heads/main", &local],
                );
            },
            "unpushed_commits",
        ),
        (
            "a custom refs/x/* ref",
            |lib, repo| {
                let local = local_commit(lib, repo, "custom");
                lib.git(repo, &["update-ref", "refs/x/custom", &local]);
            },
            "unpushed_commits",
        ),
        (
            "a note",
            |lib, repo| {
                lib.git(repo, &["notes", "add", "-m", "a note only here"]);
            },
            "unpushed_commits",
        ),
    ];
    let mut blocked = 0;
    for (name, spoil, expected) in cases {
        let lib = Library::new();
        let (copy, id) = pushed(&lib);
        spoil(&lib, &copy);
        lib.rescan_lineage(id, &copy);
        let verdict = lib.preflight(id, &lib.verifier());
        eprintln!("root namespace, {name}: {verdict}");
        assert_ne!(verdict.disposition(), "safe", "{name}: {verdict}");
        assert!(verdict.has(expected), "{name}: {verdict}");
        blocked += 1;
    }
    eprintln!("every root namespace: {blocked} of 6 blocked alone");
    assert_eq!(blocked, 6);
}

/// A fixture's one spoiling change.
type Break = fn(&Library, &Path);

/// The broken shapes only a permission bit can make: unix only, because an ACL denial needs a
/// second principal a test cannot rely on.
fn unreadable_shapes() -> Vec<(&'static str, Break, &'static str)> {
    #[allow(unused_mut)]
    let mut cases: Vec<(&'static str, Break, &'static str)> = Vec::new();
    #[cfg(unix)]
    {
        cases.push((
            "an unreadable loose ref file",
            |lib: &Library, repo: &Path| {
                use std::os::unix::fs::PermissionsExt;
                let head = lib.git(repo, &["rev-parse", "HEAD"]);
                let file = repo.join(".git/refs/heads/locked");
                std::fs::write(&file, head).expect("ref");
                std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o000))
                    .expect("chmod");
            },
            "refs_unreadable",
        ));
        cases.push((
            "an unreadable refs/heads/ directory",
            |lib: &Library, repo: &Path| {
                use std::os::unix::fs::PermissionsExt;
                let head = lib.git(repo, &["rev-parse", "HEAD"]);
                let dir = repo.join(".git/refs/heads/sealed");
                std::fs::create_dir_all(&dir).expect("dir");
                std::fs::write(dir.join("inside"), head).expect("ref");
                std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o000))
                    .expect("chmod");
            },
            "refs_unreadable",
        ));
        cases.push((
            "a permission-denied stash reflog with refs/stash present",
            |lib: &Library, repo: &Path| {
                use std::os::unix::fs::PermissionsExt;
                std::fs::write(repo.join("a.txt"), b"stashed\n").expect("edit");
                lib.git(repo, &["stash", "push", "-q", "-m", "work"]);
                std::fs::set_permissions(
                    repo.join(".git/logs/refs/stash"),
                    std::fs::Permissions::from_mode(0o000),
                )
                .expect("chmod");
            },
            "stash_unreadable",
        ));
    }
    cases
}

/// **AC-P4-45-5 — missing, broken, unreadable.** Each yields `refs_unreadable` and never `safe`;
/// a permission-denied stash reflog with `refs/stash` present yields `stash_unreadable`. The count
/// of broken shapes is printed, and zero fails.
#[test]
fn every_broken_shape_is_refs_unreadable_and_never_safe() {
    let mut cases: Vec<(&str, Break, &str)> = vec![
        (
            "a ref naming a missing object",
            |_, repo| {
                std::fs::write(
                    repo.join(".git/refs/heads/ghost"),
                    format!("{}\n", "0123456789abcdef".repeat(3).get(..40).unwrap()),
                )
                .expect("ghost ref");
            },
            "refs_unreadable",
        ),
        (
            "a missing ancestor",
            |lib, repo| {
                let first = local_commit(lib, repo, "first local");
                lib.git(repo, &["update-ref", "refs/heads/side", &first]);
                lib.git(repo, &["checkout", "-q", "side"]);
                lib.commit(repo, "side.txt", "second local\n");
                lib.git(repo, &["checkout", "-q", "main"]);
                let object = repo
                    .join(".git/objects")
                    .join(first.get(..2).unwrap())
                    .join(first.get(2..).unwrap());
                std::fs::remove_file(object).expect("the ancestor was loose");
            },
            "refs_unreadable",
        ),
        (
            "a garbage loose ref",
            |_, repo| {
                std::fs::write(repo.join(".git/refs/heads/junk"), b"garbage\n").expect("junk");
            },
            "refs_unreadable",
        ),
        (
            "a dangling symref",
            |_, repo| {
                std::fs::write(
                    repo.join(".git/refs/heads/dangling"),
                    b"ref: refs/heads/nowhere\n",
                )
                .expect("symref");
            },
            "refs_unreadable",
        ),
    ];
    cases.extend(unreadable_shapes());
    let mut broken = 0;
    for (name, spoil, expected) in cases {
        let lib = Library::new();
        let (copy, id) = pushed(&lib);
        spoil(&lib, &copy);
        let verdict = lib.preflight(id, &lib.verifier());
        eprintln!("broken shape, {name}: {verdict}");
        assert_ne!(verdict.disposition(), "safe", "{name}: {verdict}");
        assert!(verdict.has(expected), "{name}: {verdict}");
        broken += 1;
        restore_modes(&copy);
    }
    eprintln!("broken shapes: {broken}, each unknown and never safe");
    assert!(broken > 0, "no broken shape ran");
}

/// Give every directory and file back its owner's read bit, so the tempdir can be removed.
fn restore_modes(dir: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o755));
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    restore_modes(&path);
                } else {
                    let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644));
                }
            }
        }
    }
    #[cfg(not(unix))]
    let _ = dir;
}

/// **AC-P4-45-14 — grafts and replace refs.** A graft file, and separately a replace ref, that
/// makes a local-only commit reachable from an advertised tip still yields `unpushed_commits`:
/// the walk runs with replace objects **and** grafts off.
#[test]
fn a_graft_and_a_replace_ref_do_not_cover_a_local_commit() {
    let mut covered_wrongly = Vec::new();
    for shape in ["a graft file", "a replace ref"] {
        let lib = Library::new();
        let (copy, id) = pushed(&lib);
        let tip = lib.git(&copy, &["rev-parse", "HEAD"]).trim().to_owned();
        let local = local_commit(&lib, &copy, "local only");
        lib.git(&copy, &["update-ref", "refs/heads/local", &local]);
        if shape == "a graft file" {
            // The advertised tip, grafted onto the local commit as its parent.
            std::fs::write(copy.join(".git/info/grafts"), format!("{tip} {local}\n"))
                .expect("grafts");
        } else {
            lib.git(&copy, &["replace", "--graft", &tip, &local]);
        }
        // A scan after the change: both shapes change what the root set reads.
        lib.rescan_lineage(id, &copy);
        let verdict = lib.preflight(id, &lib.verifier());
        eprintln!("{shape}: {verdict}");
        if !verdict.has("unpushed_commits") {
            covered_wrongly.push(format!("{shape}: {verdict}"));
        }
    }
    assert!(
        covered_wrongly.is_empty(),
        "a local-only commit read as covered: {covered_wrongly:?}"
    );
}

/// **AC-P4-45-15 — one walk.** Over 2,000 refs on 2,000 distinct commits, the `rev-list`-backed
/// reads per analysis are independent of the ref count — counted through the backend and printed.
#[test]
fn the_walk_count_is_independent_of_ref_count() {
    let lib = Library::new();
    let copy = lib.pushed_repo("widget");
    // 2,000 commits on one chain, a branch at each, through fast-import: one process, not 2,000.
    let head = lib.git(&copy, &["rev-parse", "HEAD"]).trim().to_owned();
    let mut stream = String::new();
    for n in 0..2_000 {
        let parent = if n == 0 {
            head.clone()
        } else {
            format!(":{n}")
        };
        let _ = write!(
            stream,
            "commit refs/heads/b{n}\nmark :{}\ncommitter Fixture <fixture@example.invalid> {} +0000\n\
             data 2\nc\nfrom {parent}\n\n",
            n + 1,
            1_700_000_000 + n
        );
    }
    let mut child = std::process::Command::new("git")
        .current_dir(&copy)
        .args(["fast-import", "--quiet"])
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("HOME", &lib.home)
        .stdin(std::process::Stdio::piped())
        .spawn()
        .expect("fast-import");
    std::io::Write::write_all(child.stdin.as_mut().expect("stdin"), stream.as_bytes())
        .expect("stream");
    assert!(child.wait().expect("fast-import").success());
    lib.git(&copy, &["push", "-q", "origin", "--all"]);
    lib.git(&copy, &["fetch", "-q", "origin"]);
    let refs = lib
        .git(&copy, &["for-each-ref", "refs/heads"])
        .lines()
        .count();
    let id = lib.register(&copy);

    let recording = RecordingGitBackend::new(lib.read_git.clone());
    let remotes = GitRemoteVerifier::with_transport_fixture(
        &lib.write_git,
        &recording,
        codotheca_core::gitw::TransportFixture::new(&lib.net),
    );
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
        support::analyser_world::NOW,
    )
    .expect("the pre-flight answers");
    let verdict = Verdict(value);
    let walks = recording
        .calls()
        .iter()
        .filter(|call| call.op == "any_uncovered")
        .count();
    eprintln!("one walk: {refs} branches, {walks} rev-list walk(s); {verdict}");
    assert!(refs >= 2_000, "the fixture holds {refs} branches");
    assert_eq!(verdict.disposition(), "safe", "{verdict}");
    assert_eq!(walks, 1, "the walk count moved with the ref count");
}

/// **AC-P4-45-7's Lane-0 clauses — tags.** A local annotated tag on a pushed commit yields
/// `unpushed_tag`; a lightweight tag on a pushed commit yields nothing.
#[test]
fn a_local_annotated_tag_blocks_and_a_lightweight_one_does_not() {
    let lib = Library::new();
    let (copy, id) = pushed(&lib);
    lib.git(&copy, &["tag", "light"]);
    let lightweight = lib.preflight(id, &lib.verifier());
    lib.git(&copy, &["tag", "-a", "annotated", "-m", "only here"]);
    let annotated = lib.preflight(id, &lib.verifier());
    eprintln!("a lightweight tag: {lightweight}; an annotated tag: {annotated}");
    assert_eq!(lightweight.disposition(), "safe", "{lightweight}");
    assert!(annotated.has("unpushed_tag"), "{annotated}");
    assert!(!annotated.has("unpushed_commits"), "{annotated}");
}
