#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! Lane 0's whole-analyser criteria: every blocker produced through production, a verdict no
//! cached column can move, and one analyser constructing every blocker.

mod support;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use codotheca_core::analyser::remote::DidNotAnswer;
use codotheca_core::analyser::verdict::is_unknown_blocker;
use codotheca_core::protocol::{NestedKind, UninstallBlocker};
use codotheca_core::testing::FixtureRemoteVerifier;
use support::analyser_world::{Library, Verdict};

/// A pushed copy spoiled by `spoil`, registered and pre-flighted by the production verifier.
fn spoiled(spoil: impl FnOnce(&Library, &Path)) -> Verdict {
    let lib = Library::new();
    let copy = lib.pushed_repo("widget");
    spoil(&lib, &copy);
    let id = lib.register(&copy);
    lib.preflight(id, &lib.verifier())
}

/// A commit only this repository holds, on `HEAD`'s branch.
fn local_commit(lib: &Library, copy: &Path) {
    lib.commit(copy, "local.txt", "only here\n");
}

/// A pushed copy whose only remote does not answer.
fn silent_remote() -> Verdict {
    let lib = Library::new();
    let copy = lib.pushed_repo("widget");
    let id = lib.register(&copy);
    let remotes = FixtureRemoteVerifier::new();
    remotes.silent("origin", DidNotAnswer::Failed);
    lib.preflight(id, &remotes)
}

/// A copy with one local commit whose only remote is a mirror on this machine.
fn local_mirror() -> Verdict {
    let lib = Library::new();
    let copy = lib.repo("widget");
    let mirror = lib.base.join("mirror.git");
    lib.git(
        &lib.base,
        &[
            "init",
            "-q",
            "--bare",
            "-b",
            "main",
            &mirror.to_string_lossy(),
        ],
    );
    lib.git(
        &copy,
        &["remote", "add", "mirror", &mirror.to_string_lossy()],
    );
    let id = lib.register(&copy);
    lib.preflight(id, &lib.verifier())
}

/// A copy registered outside every scan root.
fn outside_every_root() -> Verdict {
    let lib = Library::new();
    let outside = lib.base.join("outside");
    lib.pushed_repo_at(&outside, "outside");
    let id = lib.register(&outside);
    lib.preflight(id, &lib.verifier())
}

/// A copy whose row says the app has never looked at it.
fn never_observed() -> Verdict {
    let lib = Library::new();
    let copy = lib.pushed_repo("widget");
    let id = lib.register(&copy);
    lib.index
        .lock()
        .expect("index")
        .conn()
        .execute(
            "UPDATE location SET refstate_observed_at = NULL WHERE id = ?1",
            [id.0],
        )
        .expect("never observed");
    lib.preflight(id, &lib.verifier())
}

/// A copy with a live launch session.
fn live_session() -> Verdict {
    let lib = Library::new();
    let copy = lib.pushed_repo("widget");
    let id = lib.register(&copy);
    lib.index
        .lock()
        .expect("index")
        .conn()
        .execute(
            "INSERT INTO session (project_id, location_id, started_at)
             SELECT project_id, id, 1 FROM location WHERE id = ?1",
            [id.0],
        )
        .expect("a live session");
    lib.preflight(id, &lib.verifier())
}

/// A copy another location borrows objects from.
fn borrowed() -> Verdict {
    let lib = Library::new();
    let lender = lib.pushed_repo("widget");
    let borrower = lib.root.join("borrower");
    lib.git(
        &lib.base,
        &[
            "clone",
            "-q",
            "--shared",
            &lender.to_string_lossy(),
            &borrower.to_string_lossy(),
        ],
    );
    lib.register(&borrower);
    let id = lib.register(&lender);
    lib.preflight(id, &lib.verifier())
}

/// A submodule holding a local-only commit.
fn submodule_with_a_local_commit() -> Verdict {
    let lib = Library::new();
    let copy = lib.with_submodule();
    lib.commit(&copy.join("sub"), "s.txt", "local in the submodule\n");
    let id = lib.register(&copy);
    lib.preflight(id, &lib.verifier())
}

/// A de-initialised submodule whose module git dir holds a local-only branch.
fn deinitialised_module() -> Verdict {
    let lib = Library::new();
    let copy = lib.with_submodule();
    let sub = copy.join("sub");
    lib.git(&sub, &["checkout", "-q", "-b", "local"]);
    lib.commit(&sub, "s.txt", "local branch\n");
    lib.git(&sub, &["checkout", "-q", "--detach", "HEAD~1"]);
    lib.git(&copy, &["submodule", "deinit", "-q", "-f", "sub"]);
    let id = lib.register(&copy);
    lib.preflight(id, &lib.verifier())
}

/// Repositories nested four deep.
fn four_deep() -> Verdict {
    spoiled(|lib, copy| {
        let mut dir = copy.to_path_buf();
        for level in ["n1", "n2", "n3", "n4"] {
            dir = dir.join(level);
            lib.git(
                &lib.base,
                &["init", "-q", "-b", "main", &dir.to_string_lossy()],
            );
        }
    })
}

/// The fixture built to produce `blocker`. The match is exhaustive over the generated enum, so a
/// variant with no fixture fails to compile rather than going uncovered.
// One arm per blocker variant: splitting the match would scatter the one list the exhaustiveness
// check reads.
#[allow(clippy::too_many_lines)]
fn fixture_for(blocker: UninstallBlocker) -> Verdict {
    match blocker {
        UninstallBlocker::UnpushedCommits => spoiled(local_commit),
        UninstallBlocker::UncommittedChanges => spoiled(|_, copy| {
            std::fs::write(copy.join("a.txt"), b"edited\n").expect("edit");
        }),
        UninstallBlocker::StashPresent => spoiled(|lib, copy| {
            std::fs::write(copy.join("a.txt"), b"stashed\n").expect("edit");
            lib.git(copy, &["stash", "push", "-q", "-m", "work"]);
        }),
        UninstallBlocker::UntrackedPrecious => spoiled(|_, copy| {
            std::fs::write(copy.join("notes.txt"), b"unsaved\n").expect("untracked");
        }),
        UninstallBlocker::IgnoredPrecious => spoiled(|_, copy| {
            std::fs::write(copy.join(".git/info/exclude"), b".env\n").expect("exclude");
            std::fs::write(copy.join(".env"), b"TOKEN=local\n").expect(".env");
        }),
        UninstallBlocker::SubmoduleUnsafe => spoiled(|lib, copy| {
            let inner = copy.join("vendor").join("x");
            lib.git(
                &lib.base,
                &["init", "-q", "-b", "main", &inner.to_string_lossy()],
            );
            lib.commit(&inner, "x.txt", "only here\n");
        }),
        UninstallBlocker::LinkedWorktree => spoiled(|lib, copy| {
            lib.git(
                copy,
                &[
                    "worktree",
                    "add",
                    "-q",
                    &lib.base.join("elsewhere").to_string_lossy(),
                ],
            );
        }),
        UninstallBlocker::ShallowClone => {
            let lib = Library::new();
            let copy = lib.shallow_repo("widget");
            let id = lib.register(&copy);
            lib.preflight(id, &lib.verifier())
        }
        UninstallBlocker::RemoteUnreachable => silent_remote(),
        UninstallBlocker::RemoteIsLocalMirror => local_mirror(),
        UninstallBlocker::StashUnreadable => spoiled(|lib, copy| {
            std::fs::write(copy.join("a.txt"), b"stashed\n").expect("edit");
            lib.git(copy, &["stash", "push", "-q", "-m", "work"]);
            // A directory where the reflog should be: unreadable as a file on every platform,
            // without a permission bit this test's user may override.
            let reflog = copy.join(".git/logs/refs/stash");
            std::fs::remove_file(&reflog).expect("reflog");
            std::fs::create_dir_all(&reflog).expect("reflog directory");
        }),
        UninstallBlocker::LiveSession => live_session(),
        UninstallBlocker::RefusedPath => outside_every_root(),
        UninstallBlocker::NeverObserved => never_observed(),
        UninstallBlocker::NoRemote => {
            let lib = Library::new();
            let copy = lib.repo("widget");
            let id = lib.register(&copy);
            lib.preflight(id, &lib.verifier())
        }
        UninstallBlocker::UnpushedTag => spoiled(|lib, copy| {
            lib.git(copy, &["tag", "-a", "local", "-m", "only here"]);
        }),
        UninstallBlocker::InterruptedOperation => spoiled(|lib, copy| {
            lib.git(copy, &["checkout", "-q", "-b", "side"]);
            let pick = lib.commit(copy, "a.txt", "side\n");
            lib.git(copy, &["checkout", "-q", "main"]);
            lib.commit(copy, "a.txt", "main\n");
            let _ = lib.git_output(copy, &["cherry-pick", &pick]);
        }),
        UninstallBlocker::BorrowedByAnotherRepository => borrowed(),
        UninstallBlocker::RefsUnreadable => spoiled(|_, copy| {
            std::fs::write(copy.join(".git/refs/heads/junk"), b"garbage\n").expect("junk");
        }),
        UninstallBlocker::HiddenFromStatus => spoiled(|lib, copy| {
            lib.git(copy, &["update-index", "--skip-worktree", "a.txt"]);
        }),
        UninstallBlocker::LfsUnverified => spoiled(|_, copy| {
            let objects = copy.join(".git/lfs/objects/ab/cd");
            std::fs::create_dir_all(&objects).expect("lfs");
            std::fs::write(objects.join("abcd"), b"a large file\n").expect("object");
        }),
        UninstallBlocker::NestingTooDeep => four_deep(),
    }
}

/// **AC-P4-45-1 — every blocker, through production.** One fixture per variant of the
/// generated `UninstallBlocker::ALL`, built to produce it and driven through the pre-flight
/// handler; every `NestedKind` appears in `nested`. Prints covered / total and each variant's
/// class, derives the partition, and fails on a variant with no fixture and at zero.
#[test]
fn every_blocker_is_produced_through_the_production_path() {
    let mut covered = 0;
    let mut missing = Vec::new();
    let mut classes: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    let mut kinds_seen: Vec<String> = Vec::new();
    for blocker in UninstallBlocker::ALL {
        let name = serde_json::to_value(blocker)
            .expect("name")
            .as_str()
            .expect("a string")
            .to_owned();
        let verdict = fixture_for(blocker);
        let class = if is_unknown_blocker(blocker) {
            "unknown"
        } else {
            "known-bad"
        };
        eprintln!("blocker {name} ({class}): {verdict}");
        if verdict.has(&name) {
            covered += 1;
        } else {
            missing.push(name.clone());
        }
        classes.entry(class).or_default().push(name);
        for entry in verdict.0["nested"].as_array().into_iter().flatten() {
            let kind = entry["kind"].as_str().expect("kind").to_owned();
            if !kinds_seen.contains(&kind) {
                kinds_seen.push(kind);
            }
        }
    }
    // The de-initialised module git dir is the one kind no blocker fixture above needs.
    for entry in deinitialised_module().0["nested"]
        .as_array()
        .into_iter()
        .flatten()
    {
        let kind = entry["kind"].as_str().expect("kind").to_owned();
        if !kinds_seen.contains(&kind) {
            kinds_seen.push(kind);
        }
    }
    for entry in submodule_with_a_local_commit().0["nested"]
        .as_array()
        .into_iter()
        .flatten()
    {
        let kind = entry["kind"].as_str().expect("kind").to_owned();
        if !kinds_seen.contains(&kind) {
            kinds_seen.push(kind);
        }
    }
    let total = UninstallBlocker::ALL.len();
    eprintln!(
        "every blocker: {covered} / {total} produced through production; partition {:?}; \
         nested kinds {kinds_seen:?}",
        classes
            .iter()
            .map(|(class, names)| (class, names.len()))
            .collect::<Vec<_>>()
    );
    assert!(covered > 0, "no blocker was produced");
    assert!(missing.is_empty(), "no fixture produced: {missing:?}");
    for kind in NestedKind::ALL {
        let name = serde_json::to_value(kind).expect("kind");
        assert!(
            kinds_seen.iter().any(|seen| name == *seen),
            "no fixture listed a {name} nested repository"
        );
    }
}

/// §45.7's columns the analyser may not read.
const FORBIDDEN: [&str; 10] = [
    "is_dirty",
    "untracked_count",
    "stash_count",
    "ahead",
    "behind",
    "branch",
    "head_oid",
    "refstate_basis",
    "p.is_shallow",
    "fetch_head_at",
];

/// D-4's two plumbing columns, allowed and printed.
const PLUMBING: [&str; 2] = ["store_key", "trusted_at"];

/// **AC-P4-45-18 — never from cache.** Over a shallow, dirty, stashed copy, the verdict is
/// identical whether the cached columns tell the truth or lie. And the analyser names none of
/// §45.7's columns but the two D-4 plumbing ones, and imports nothing from §25.3's producer.
#[test]
fn the_verdict_is_identical_under_lying_cached_columns() {
    let lib = Library::new();
    let copy = lib.shallow_repo("widget");
    std::fs::write(copy.join("a.txt"), b"stashed\n").expect("edit");
    lib.git(&copy, &["stash", "push", "-q", "-m", "work"]);
    std::fs::write(copy.join("a.txt"), b"dirty\n").expect("dirty");
    let id = lib.register(&copy);
    let set = |ahead: i64, stash: i64, dirty: i64, shallow: i64| {
        let guard = lib.index.lock().expect("index");
        guard
            .conn()
            .execute(
                "UPDATE location SET ahead = ?1, stash_count = ?2, is_dirty = ?3 WHERE id = ?4",
                rusqlite::params![ahead, stash, dirty, id.0],
            )
            .expect("columns");
        guard
            .conn()
            .execute("UPDATE project SET is_shallow = ?1", [shallow])
            .expect("shallow");
    };
    set(3, 1, 1, 1);
    let truthful = lib.preflight(id, &lib.verifier());
    set(0, 0, 0, 0);
    let lying = lib.preflight(id, &lib.verifier());
    eprintln!("truthful columns: {truthful}; lying columns: {lying}");
    assert_eq!(truthful.0, lying.0, "a cached column moved the verdict");
    assert!(
        lying.has("shallow_clone") && lying.has("uncommitted_changes"),
        "{lying}"
    );

    // The call-site rule over `core/src/analyser/`.
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("analyser");
    let mut scanned = 0;
    let mut offenders = Vec::new();
    let mut plumbing_seen = Vec::new();
    for entry in std::fs::read_dir(&dir).expect("analyser").flatten() {
        let path: PathBuf = entry.path();
        let text = std::fs::read_to_string(&path).expect("source");
        scanned += 1;
        let sql: Vec<&str> = text
            .split('"')
            .skip(1)
            .step_by(2)
            .filter(|literal| literal.contains("SELECT") || literal.contains("FROM"))
            .collect();
        for statement in &sql {
            for column in FORBIDDEN {
                if statement
                    .split(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '.')
                    .any(|word| word == column || word.ends_with(&format!(".{column}")))
                {
                    offenders.push(format!("{}: {column}", path.display()));
                }
            }
            for column in PLUMBING {
                if statement.contains(column) && !plumbing_seen.contains(&column) {
                    plumbing_seen.push(column);
                }
            }
        }
        for line in text.lines().filter(|l| !l.trim_start().starts_with("//")) {
            if line.contains("remote::backup") || line.contains("backup_state") {
                offenders.push(format!("{}: imports §25.3's producer", path.display()));
            }
        }
    }
    eprintln!(
        "never from cache: {scanned} analyser file(s) scanned; D-4's plumbing columns read: \
         {plumbing_seen:?}; offenders {offenders:?}"
    );
    assert!(scanned > 0);
    assert!(offenders.is_empty(), "{offenders:?}");
}

/// **AC-P4-45-22 — one analyser, one vocabulary** (Lane 0's list). The production callers of
/// `analyse` are the two uninstall handlers, and every file in `core/src` constructing an
/// `UninstallBlocker` variant — the generated protocol aside — is under `core/src/analyser/`.
#[test]
fn one_analyser_constructs_every_blocker() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files: Vec<PathBuf> = Vec::new();
    let mut stack = vec![src.clone()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("src").flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                files.push(path);
            }
        }
    }
    let mut constructors = Vec::new();
    let mut callers = Vec::new();
    for path in &files {
        let rel = path
            .strip_prefix(&src)
            .expect("under src")
            .to_string_lossy()
            .replace('\\', "/");
        let text = std::fs::read_to_string(path).expect("source");
        let code: Vec<&str> = text
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect();
        if rel != "protocol.rs" && code.iter().any(|l| l.contains("UninstallBlocker::")) {
            constructors.push(rel.clone());
        }
        if code
            .iter()
            .any(|l| l.contains("analyse(&") && !l.contains("fn analyse"))
            && !rel.starts_with("analyser/")
        {
            callers.push(rel);
        }
    }
    eprintln!(
        "one analyser: {} source file(s) scanned; callers of analyse {callers:?}; files naming \
         a blocker variant {constructors:?}",
        files.len()
    );
    assert!(!constructors.is_empty(), "no file constructs a blocker");
    assert_eq!(callers, vec!["uninstall/handle.rs".to_owned()]);
    let outside: Vec<&String> = constructors
        .iter()
        .filter(|file| !file.starts_with("analyser/"))
        .collect();
    assert!(
        outside.is_empty(),
        "a second site constructs blockers: {outside:?}"
    );
}
