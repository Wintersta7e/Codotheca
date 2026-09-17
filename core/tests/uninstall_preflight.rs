#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

//! §24.7A's stash truth: the reader a **deletion gate** may trust.
//!
//! Each of the three inputs is absent in a real configuration where a stash exists, so each case
//! below is a repository that genuinely occurs — not a synthetic corruption.

mod support;

use codotheca_core::uninstall::{read_stash_truth, StashTruth};
use support::TestRepo;

/// A clean repository has no stash, and every input was readable — which is what makes this
/// `None` rather than `Unreadable`.
#[test]
fn a_clean_repository_has_no_stash() {
    let repo = TestRepo::init();
    repo.write("a.txt", b"one\n");
    repo.commit("first");
    assert_eq!(
        read_stash_truth(&repo.path().join(".git")),
        StashTruth::None
    );
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

    assert_eq!(
        read_stash_truth(&repo.path().join(".git")),
        StashTruth::Present(2)
    );
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
        read_stash_truth(&git_dir),
        StashTruth::Present(1),
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

    assert_eq!(read_stash_truth(&git_dir), StashTruth::Present(1));
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
        read_stash_truth(&git_dir),
        StashTruth::Unreadable,
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
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("uninstall");
    let mut scanned = 0_usize;
    let mut offenders = Vec::new();
    for entry in std::fs::read_dir(&dir)
        .expect("core/src/uninstall/ must exist")
        .flatten()
    {
        let path = entry.path();
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
            if line.contains("stash_count") {
                offenders.push(format!("{}:{}", path.display(), index + 1));
            }
        }
    }
    eprintln!("uninstall_preflight: the cached-column scan read {scanned} file(s)");
    assert!(
        scanned >= 2,
        "the scan read {scanned} file(s); a gate that scans nothing is a failing gate"
    );
    assert!(
        offenders.is_empty(),
        "the pre-flight reads the cached badge column at {offenders:?} — it must read live"
    );
}

/// And the reader spawns nothing (A3): no `git stash` subcommand exists anywhere in the module.
#[test]
fn the_stash_reader_spawns_no_git() {
    let source = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("uninstall")
            .join("stash.rs"),
    )
    .expect("the stash reader");
    assert!(
        !source.contains("Command::new"),
        "the reader spawns a process"
    );
    for line in source.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("//") {
            continue;
        }
        assert!(
            !line.contains("\"stash\","),
            "an argv token appeared in a reader that reads files: {line}"
        );
    }
}
