#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

//! §24.7A's stash truth, and the analyser's other phase-2 reads, on the readers §45.2 owns now.
//!
//! [p4] The file-reading stash reader is gone: row 3 reads **through git**, `log -g` and never
//! `git stash`, with an unreadable files-backend reflog answered as `Unreadable` (§45.2). Each
//! case below is still a repository that genuinely occurs — not a synthetic corruption.

mod support;

use codotheca_core::cancel::CancelToken;
use codotheca_core::git::{GitBackend as _, JobClass, JobContext, StashEntries};
use support::git_world::system_git;
use support::TestRepo;

/// §45.2 row 3, through the production read backend.
fn stash(repo: &TestRepo) -> StashEntries {
    let cancel = CancelToken::new();
    system_git(repo)
        .stash_entries(
            &repo.handle(),
            &JobContext::new(JobClass::Interactive, &cancel, None),
        )
        .expect("the stash read answers")
}

/// The number of stash entries, or `None` when the stash cannot be read.
fn entries(truth: &StashEntries) -> Option<usize> {
    match truth {
        StashEntries::Entries(entries) => Some(entries.len()),
        StashEntries::Unreadable => None,
    }
}

/// A clean repository has no stash, and every input was readable — which is what makes this
/// `None` rather than `Unreadable`.
#[test]
fn a_clean_repository_has_no_stash() {
    let repo = TestRepo::init();
    repo.write("a.txt", b"one\n");
    repo.commit("first");
    assert_eq!(entries(&stash(&repo)), Some(0));
}

/// The ordinary case: the reflog is there and says how many.
#[test]
fn the_reflog_gives_an_exact_count() {
    let repo = TestRepo::init();
    repo.write("a.txt", b"one\n");
    repo.commit("first");
    repo.write("a.txt", b"two\n");
    repo.git(&["stash", "push", "-q", "-m", "one"]);
    repo.write("a.txt", b"three\n");
    repo.git(&["stash", "push", "-q", "-m", "two"]);

    assert_eq!(entries(&stash(&repo)), Some(2));
}

/// **The case `location.stash_count` and the reflog both miss.** A stash reachable only through a
/// packed `refs/stash`, with no reflog at all — which is what `core.logAllRefUpdates=false` plus a
/// `git gc` leaves behind. A gate that read only the reflog would call this repository clean and
/// delete stashed work.
#[test]
fn a_stash_reachable_only_through_packed_refs_is_still_a_stash() {
    let repo = TestRepo::init();
    repo.write("a.txt", b"one\n");
    repo.commit("first");
    repo.write("a.txt", b"two\n");
    repo.git(&["stash", "push", "-q", "-m", "one"]);

    let git_dir = repo.path().join(".git");
    let stash_oid = std::fs::read_to_string(git_dir.join("refs").join("stash"))
        .or_else(|_| -> std::io::Result<String> {
            // Already packed by this git version; read it back out of packed-refs.
            let packed = std::fs::read_to_string(git_dir.join("packed-refs"))?;
            for line in packed.lines() {
                if let Some((oid, name)) = line.split_once(' ') {
                    if name.trim() == "refs/stash" {
                        return Ok(oid.to_owned());
                    }
                }
            }
            Ok(String::new())
        })
        .expect("a stash ref exists somewhere");
    let stash_oid = stash_oid.trim().to_owned();
    assert!(
        !stash_oid.is_empty(),
        "the fixture must have produced a stash"
    );

    // Pack the ref by hand and remove both the loose ref and the reflog — the exact shape a
    // repository with reflogs disabled has after a gc.
    std::fs::write(
        git_dir.join("packed-refs"),
        format!("# pack-refs with: peeled fully-peeled sorted \n{stash_oid} refs/stash\n"),
    )
    .expect("write packed-refs");
    let _ = std::fs::remove_file(git_dir.join("refs").join("stash"));
    let _ = std::fs::remove_file(git_dir.join("logs").join("refs").join("stash"));

    assert_eq!(
        entries(&stash(&repo)),
        Some(1),
        "a packed stash ref proves a stash exists; the count is a floor, not a guess"
    );
}

/// A loose `refs/stash` with no reflog — `core.logAllRefUpdates=false` before any gc.
#[test]
fn a_loose_stash_ref_with_no_reflog_is_still_a_stash() {
    let repo = TestRepo::init();
    repo.write("a.txt", b"one\n");
    repo.commit("first");
    repo.write("a.txt", b"two\n");
    repo.git(&["stash", "push", "-q", "-m", "one"]);

    let git_dir = repo.path().join(".git");
    if !git_dir.join("refs").join("stash").exists() {
        eprintln!("uninstall_preflight: SKIPPED — this git packed the stash ref immediately");
        return;
    }
    let _ = std::fs::remove_file(git_dir.join("logs").join("refs").join("stash"));

    assert_eq!(entries(&stash(&repo)), Some(1));
}

/// **Unreadable is unsafe.** An input that could not be read is not an absent stash, and it must
/// not be outvoted by the two that were readable.
#[test]
fn an_unreadable_input_is_unreadable_and_never_none() {
    let repo = TestRepo::init();
    repo.write("a.txt", b"one\n");
    repo.commit("first");
    let git_dir = repo.path().join(".git");

    // A directory where the reflog should be: unreadable as a file on every platform, without
    // depending on a permission bit this test's user may override.
    let reflog = git_dir.join("logs").join("refs").join("stash");
    std::fs::create_dir_all(reflog.parent().unwrap()).unwrap();
    let _ = std::fs::remove_file(&reflog);
    std::fs::create_dir_all(&reflog).unwrap();

    assert_eq!(
        entries(&stash(&repo)),
        None,
        "an unreadable input is never reported as no stash"
    );
}

/// **AC-P2-24-15's second half, as a counting check rather than a grep.**
///
/// The pre-flight must not read `location.stash_count`: it is a cached badge value keyed on the
/// freshness basis, and a deletion gate reads live. A grep whose passing run proves nothing is
/// exactly the defect this project keeps recording, so the scan prints what it read and fails at
/// zero.
#[test]
fn the_uninstall_module_never_reads_the_cached_stash_column() {
    // [p4] Widened to `core/src/analyser/`, where every blocker is computed now (§45.7).
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut scanned = 0_usize;
    let mut offenders = Vec::new();
    let files: Vec<std::path::PathBuf> = ["uninstall", "analyser"]
        .iter()
        .flat_map(|dir| {
            std::fs::read_dir(src.join(dir))
                .expect("the module must exist")
                .flatten()
                .map(|entry| entry.path())
        })
        .collect();
    for path in files {
        if path.extension().is_none_or(|e| e != "rs") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        scanned += 1;
        for (index, line) in text.lines().enumerate() {
            let trimmed = line.trim_start();
            // Prose about the column is not a read of it — grepping a declaration matches prose
            // about the declaration, recorded twice in this project.
            if trimmed.starts_with("//") || trimmed.starts_with("///") {
                continue;
            }
            if !line.contains("stash_count") {
                continue;
            }
            // **Clearing the column is the opposite of trusting it.** §24.6b nulls `stash_count`
            // on removal precisely because it describes a directory that no longer exists. What
            // this gate bans is a **read**, so it matches the read shapes — a `SELECT` of the
            // column, or a row accessor on it — rather than the name appearing at all. Loosening
            // it to "not on a line mentioning NULL" would have let a real read through on any
            // line that happened to say NULL too.
            let reads_it = line.contains("SELECT") || line.contains(".get(");
            if reads_it {
                offenders.push(format!("{}:{}", path.display(), index + 1));
            }
        }
    }
    eprintln!("uninstall_preflight: the cached-column scan read {scanned} file(s)");
    assert!(
        scanned >= 10,
        "the scan read {scanned} file(s); a gate that scans nothing is a failing gate"
    );
    assert!(
        offenders.is_empty(),
        "the pre-flight reads the cached badge column at {offenders:?} — it must read live"
    );
}

/// **A3's substance, on the reader §45.2 row 3 owns**: the stash is read through `log -g`, and
/// no `git stash` subcommand is an argv token anywhere in the read path or the analyser. A3's
/// file mechanism did not survive — it read reftable as *no stash* — and its point does.
#[test]
fn the_stash_is_read_through_log_and_never_through_git_stash() {
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let analyse = std::fs::read_to_string(src.join("git").join("analyse.rs")).expect("analyse.rs");
    let reader = analyse
        .split("pub(crate) fn stash_entries(")
        .nth(1)
        .and_then(|rest| rest.split("\n}\n").next())
        .expect("the stash reader");
    assert!(
        reader.contains("OsStr::new(\"log\")") && reader.contains("OsStr::new(\"-g\")"),
        "the stash is read through `log -g`"
    );
    let mut scanned = 0_usize;
    for dir in ["git", "analyser"] {
        for entry in std::fs::read_dir(src.join(dir)).expect("module").flatten() {
            let Ok(text) = std::fs::read_to_string(entry.path()) else {
                continue;
            };
            scanned += 1;
            for line in text.lines() {
                if line.trim_start().starts_with("//") {
                    continue;
                }
                assert!(
                    !line.contains("OsStr::new(\"stash\")") && !line.contains("\"stash\","),
                    "a `git stash` argv token in {}: {line}",
                    entry.path().display()
                );
            }
        }
    }
    eprintln!("uninstall_preflight: the stash-subcommand scan read {scanned} file(s)");
    assert!(scanned >= 10, "the scan read {scanned} file(s)");
}

// ---------------------------------------------------------------------------
// §24.7A's uniqueness analyser.
// ---------------------------------------------------------------------------

/// §45.2 row 1's names, through the production read backend the analyser uses.
fn ref_names(repo: &TestRepo) -> Vec<String> {
    let cancel = CancelToken::new();
    system_git(repo)
        .enumerate_refs(
            &repo.handle(),
            &JobContext::new(JobClass::Interactive, &cancel, None),
        )
        .expect("the listing")
        .refs
        .into_iter()
        .map(|r| r.name)
        .collect()
}

/// **The case a pre-flight that checks only `HEAD` gets wrong, which is the shredder.**
///
/// A tag and a note that exist nowhere else must each block a removal, exactly as an unpushed
/// branch does.
#[test]
fn every_local_ref_is_enumerated_and_not_only_the_checked_out_branch() {
    let repo = TestRepo::init();
    repo.write("a.txt", b"one\n");
    repo.commit("first");
    repo.git(&["branch", "feature"]);
    repo.git(&["tag", "v1"]);
    repo.git(&["notes", "add", "-m", "a note"]);

    let names = ref_names(&repo);
    assert!(
        names.iter().any(|n| n == "refs/heads/main"),
        "the checked-out branch: {names:?}"
    );
    assert!(
        names.iter().any(|n| n == "refs/heads/feature"),
        "a branch that is not HEAD: {names:?}"
    );
    assert!(
        names.iter().any(|n| n == "refs/tags/v1"),
        "a tag — a release that exists nowhere else: {names:?}"
    );
    assert!(
        names.iter().any(|n| n.starts_with("refs/notes/")),
        "a note — the most easily lost of the three: {names:?}"
    );
    assert!(
        !names.iter().any(|n| n.starts_with("refs/remotes/")),
        "remotes are what local refs are checked AGAINST, never checked: {names:?}"
    );
}

/// Packed refs count too: a repository after `git gc` has no loose refs at all, and a gate that
/// read only the loose ones would call it empty.
#[test]
fn packed_refs_are_enumerated_as_well_as_loose_ones() {
    let repo = TestRepo::init();
    repo.write("a.txt", b"one\n");
    repo.commit("first");
    repo.git(&["branch", "feature"]);
    repo.git(&["tag", "v1"]);
    repo.git(&["pack-refs", "--all"]);

    let names = ref_names(&repo);
    assert!(names.iter().any(|n| n == "refs/heads/feature"), "{names:?}");
    assert!(names.iter().any(|n| n == "refs/tags/v1"), "{names:?}");
}

/// **The direction of the ignored rule, which is the one that loses work if inverted.**
///
/// An ignored path is precious **unless** it matches known junk. A `.env` matches nothing in the
/// set and is therefore precious; `node_modules/` matches and is not.
#[test]
fn an_ignored_path_is_precious_unless_it_is_known_junk() {
    use codotheca_core::analyser::junk::is_junk;
    use std::path::Path;

    // Precious: nothing in the junk set matches these, and each is real work.
    for precious in [
        ".env",
        "local.db",
        "notes.md",
        "secrets/key.pem",
        "TODO.txt",
    ] {
        assert!(
            !is_junk(Path::new(precious)),
            "{precious} matches no junk pattern and must be treated as precious"
        );
    }

    // Junk: each is a rebuildable cache or output directory.
    for junk in [
        "node_modules/react/index.js",
        "target/debug/thing",
        "dist/bundle.js",
        "__pycache__/x.pyc",
        ".venv/bin/python",
    ] {
        assert!(is_junk(Path::new(junk)), "{junk} is rebuildable");
    }
}

/// The junk set has **one owner**, and the rule reads off it rather than restating it.
#[test]
fn the_junk_set_has_one_owner() {
    use codotheca_core::analyser::junk::{is_junk, JUNK_PATTERNS};
    use std::path::Path;

    assert!(!JUNK_PATTERNS.is_empty());
    for pattern in JUNK_PATTERNS {
        assert!(
            is_junk(Path::new(pattern)),
            "{pattern} is in the set and must be recognised by the predicate that reads it"
        );
    }
}
