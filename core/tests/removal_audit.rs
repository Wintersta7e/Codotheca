#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

//! §24.6's audit: every filesystem removal in `core/src` is either inside the single warranted
//! primitive or is a **reasoned, per-line** entry in the phase-1 baseline.
//!
//! **The baseline is 21 sites outside `core/src/removal/`, not the 17 both plans state.**
//! p2-24's Task 11 and p2-24b's Task 1 were written before this lane's own Task 4 landed the
//! credential helper, which removes its one-shot socket, its nonce file and its private
//! directory on teardown — four sites in `core/src/gitw/credential.rs`. The 17 phase-1 sites are
//! otherwise exactly p2-24b's per-file breakdown: `lifecycle.rs` 3, `art/store.rs` 4,
//! `art/job.rs` 1, `art/commands.rs` 1, `corpus/mod.rs` 5, `index/backup.rs` 2,
//! `surfaces/startup_failure.rs` 1. **p2-24b asserts this list is unchanged and must assert 21.**
//!
//! Every one of the 21 removes **app-owned** bytes: a lock file, a temporary raster, a fixture
//! corpus, a database backup, a credential socket. None can reach a user's working copy, which
//! is what `remove_warranted` exists to be the only route to.

use std::path::{Path, PathBuf};

use codotheca_core::install::staging::{staging_path_for, staging_warrant_for, STAGING_DIR_NAME};
use codotheca_core::removal::{
    remove_warranted, HardDelete, RemovalOutcome, RemovalRefusal, SessionNonce, Warrant,
};

/// One permitted removal outside `core/src/removal/`, with the reason it is permitted.
///
/// The reason lives here rather than in `acceptance/callsites.json`'s `allowWhy`, because
/// `scripts/check-call-sites.mjs` never reads that field — it is documentation for a human,
/// while this table is enforced.
#[derive(Debug)]
struct RemovalAllowance {
    file: &'static str,
    line: u32,
    reason: &'static str,
}

const BASELINE: &[RemovalAllowance] = &[
    RemovalAllowance {
        file: "lifecycle.rs",
        line: 138,
        reason: "the single-instance lock file this process itself created",
    },
    RemovalAllowance {
        file: "lifecycle.rs",
        line: 260,
        reason: "a crashed run's own state directory under the app data root",
    },
    RemovalAllowance {
        file: "lifecycle.rs",
        line: 281,
        reason: "the same state directory on the ordinary shutdown path",
    },
    RemovalAllowance {
        file: "art/commands.rs",
        line: 291,
        reason: "one rendered card raster the art store owns and can redraw",
    },
    RemovalAllowance {
        file: "art/job.rs",
        line: 269,
        reason: "the whole raster cache, which is derived output and never user data",
    },
    RemovalAllowance {
        file: "art/store.rs",
        line: 41,
        reason: "the temporary file of a raster write that failed before its rename",
    },
    RemovalAllowance {
        file: "art/store.rs",
        line: 45,
        reason: "the same temporary file on the other failure arm",
    },
    RemovalAllowance {
        file: "art/store.rs",
        line: 92,
        reason: "one superseded raster, replaced by a fresh render of the same scene",
    },
    RemovalAllowance {
        file: "art/store.rs",
        line: 409,
        reason: "a raster the cache sweep found with no scene row to justify it",
    },
    RemovalAllowance {
        file: "corpus/mod.rs",
        line: 139,
        reason: "the testkit fixture corpus, built by this crate under a temporary root",
    },
    RemovalAllowance {
        file: "corpus/mod.rs",
        line: 297,
        reason: "the corpus builder's own lock file",
    },
    RemovalAllowance {
        file: "corpus/mod.rs",
        line: 321,
        reason: "the same lock file on the release path",
    },
    RemovalAllowance {
        file: "corpus/mod.rs",
        line: 353,
        reason: "a half-built corpus root this builder abandoned",
    },
    RemovalAllowance {
        file: "corpus/mod.rs",
        line: 364,
        reason: "the corpus root on the rebuild path",
    },
    RemovalAllowance {
        file: "gitw/credential.rs",
        line: 446,
        reason: "[p2-24 Task 4] the one-shot credential socket this process bound",
    },
    RemovalAllowance {
        file: "gitw/credential.rs",
        line: 447,
        reason: "[p2-24 Task 4] the nonce file beside it, written by this process",
    },
    RemovalAllowance {
        file: "gitw/credential.rs",
        line: 448,
        reason: "[p2-24 Task 4] the private directory holding both, created by this process",
    },
    RemovalAllowance {
        file: "gitw/credential.rs",
        line: 467,
        reason: "[p2-24 Task 4] the nonce file on the single-use consumption path",
    },
    RemovalAllowance {
        file: "index/backup.rs",
        line: 20,
        reason: "a stale database backup this crate wrote",
    },
    RemovalAllowance {
        file: "index/backup.rs",
        line: 50,
        reason: "a sibling WAL or shm file of that backup",
    },
    RemovalAllowance {
        file: "surfaces/startup_failure.rs",
        line: 148,
        reason: "the startup-failure breadcrumb this process wrote on its last run",
    },
];

fn core_src() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// Every `(relative path, line number, line text)` that removes something from the filesystem.
fn removal_sites() -> (usize, Vec<(String, u32, String)>) {
    const NEEDLES: [&str; 3] = ["remove_dir_all", "remove_file", "remove_dir("];
    let root = core_src();
    let mut files = Vec::new();
    walk(&root, &mut files);
    files.sort();

    let mut sites = Vec::new();
    let mut scanned = 0_usize;
    for file in &files {
        let Ok(text) = std::fs::read_to_string(file) else {
            continue;
        };
        scanned += 1;
        let relative = file
            .strip_prefix(&root)
            .unwrap_or(file)
            .to_string_lossy()
            .replace('\\', "/");
        for (index, line) in text.lines().enumerate() {
            // A doc comment naming a function is prose about it, not a call to it. Grepping a
            // declaration matches prose about the declaration — recorded twice in this project.
            let trimmed = line.trim_start();
            if trimmed.starts_with("//") {
                continue;
            }
            if NEEDLES.iter().any(|needle| line.contains(needle)) {
                let line_no = u32::try_from(index + 1).expect("a source file is not that long");
                sites.push((relative.clone(), line_no, line.trim().to_owned()));
            }
        }
    }
    (scanned, sites)
}

/// **A gate whose passing run scans zero files is a failing gate**, so both counts are printed
/// and both have a floor.
#[test]
fn every_removal_is_warranted_or_a_reasoned_baseline_entry() {
    let (scanned, sites) = removal_sites();
    // stderr, never stdout: `print_stdout` is denied crate-wide (core/Cargo.toml:165) because
    // stdout carries protocol frames and nothing else.
    eprintln!(
        "removal audit: {scanned} source file(s), {} call site(s)",
        sites.len()
    );
    assert!(
        scanned > 100,
        "the audit scanned {scanned} files — it is reading the wrong directory"
    );

    let (inside, outside): (Vec<_>, Vec<_>) = sites
        .iter()
        .partition(|(file, _, _)| file.starts_with("removal/"));
    assert!(
        !inside.is_empty(),
        "the warranted primitive removes nothing — the audit is asserting over an empty set"
    );

    let mut unreasoned = Vec::new();
    for (file, line, text) in &outside {
        let allowed = BASELINE
            .iter()
            .any(|entry| entry.file == file && entry.line == *line);
        if !allowed {
            unreasoned.push(format!("{file}:{line}  {text}"));
        }
    }
    assert!(
        unreasoned.is_empty(),
        "these removals are neither warranted nor reasoned:\n  {}\n\
         Add the reason to BASELINE in this file and the per-line entry to \
         acceptance/callsites.json, or route the call through remove_warranted.",
        unreasoned.join("\n  ")
    );

    assert_eq!(
        outside.len(),
        BASELINE.len(),
        "the baseline is a lock: {} sites outside core/src/removal/, {} reasoned entries. \
         A removed call site must leave BASELINE in the same change.",
        outside.len(),
        BASELINE.len()
    );
}

/// The measured number, stated once so a future reader does not re-derive it from the plans,
/// which say 17 and were written before this lane's Task 4.
#[test]
fn the_baseline_is_twenty_one_sites_not_the_plans_seventeen() {
    assert_eq!(BASELINE.len(), 21);
    let credential = BASELINE
        .iter()
        .filter(|e| e.file == "gitw/credential.rs")
        .count();
    assert_eq!(
        credential, 4,
        "the four beyond the plans' 17 are the credential helper's own teardown"
    );
}

/// The reason is **read**, which is the whole difference between this table and
/// `acceptance/callsites.json`'s `allowWhy` — a field `scripts/check-call-sites.mjs` never opens.
/// An allowance with an empty reason is an allowance nobody justified.
#[test]
fn every_allowance_carries_a_reason_and_names_one_site() {
    let mut seen = std::collections::BTreeSet::new();
    for entry in BASELINE {
        assert!(
            entry.reason.len() > 20,
            "{}:{} has no real reason: {:?}",
            entry.file,
            entry.line,
            entry.reason
        );
        assert!(
            seen.insert((entry.file, entry.line)),
            "{}:{} is listed twice — one line, one reason",
            entry.file,
            entry.line
        );
    }
    assert_eq!(seen.len(), BASELINE.len());
}

/// `remove_dir_all` is the only shape that can take a working copy, so it is allowed in far
/// fewer places than the baseline as a whole.
#[test]
fn recursive_removal_is_confined_to_four_files() {
    const ALLOWED_DIRECTORIES: [&str; 4] =
        ["removal/", "lifecycle.rs", "art/job.rs", "corpus/mod.rs"];

    let (_, sites) = removal_sites();
    let recursive: Vec<_> = sites
        .iter()
        .filter(|(_, _, text)| text.contains("remove_dir_all"))
        .collect();
    eprintln!("removal audit: {} recursive call site(s)", recursive.len());
    assert!(
        !recursive.is_empty(),
        "a run finding none is not a passing run"
    );

    for (file, line, text) in recursive {
        assert!(
            ALLOWED_DIRECTORIES
                .iter()
                .any(|a| file.starts_with(a) || file == *a),
            "recursive removal at {file}:{line} ({text}) is outside the four permitted files"
        );
    }
}

#[test]
fn the_warrant_variant_list_is_two_now_that_uninstall_has_landed() {
    assert_eq!(
        Warrant::ALL.len(),
        2,
        "[p2-24b] raised from 1 with WarrantKind::Uninstall — the tripwire working, not a mirror"
    );
}

/// §24.7E, and the reason the two warrants are discriminated rather than merged.
///
/// A working copy whose root commit is not the one the verdict was computed over is **refused**.
/// The root commit and not `head_oid`: a tip moves with every commit, so a guard over it would
/// refuse a copy the user had merely committed to, and admit one rewound onto the same tip.
#[test]
fn an_uninstall_warrant_whose_identity_changed_is_refused() {
    use codotheca_core::git::RootCommit;
    use codotheca_core::protocol::{LocationId, UninstallDisposition};
    use codotheca_core::uninstall::VerdictSeal;

    let dir = tempfile::tempdir().expect("tmp");
    let copy = dir.path().join("widget");
    std::fs::create_dir_all(copy.join("src")).expect("mkdir");

    let expected = RootCommit {
        oid: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_owned(),
        committed_at: 0,
        tz_offset_min: 0,
    };
    let warrant = Warrant::for_uninstall_in_test(
        LocationId(1),
        copy.clone(),
        expected.clone(),
        VerdictSeal::of(&[], UninstallDisposition::Safe),
    );

    // A different repository at the same path.
    let elsewhere = RootCommit {
        oid: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_owned(),
        ..expected
    };
    assert_eq!(
        remove_warranted(&warrant, &HardDelete, Some(&elsewhere)),
        Err(RemovalRefusal::IdentityChanged)
    );
    assert!(copy.exists(), "a refused removal removes nothing");

    // No identity at all is a refusal, never a match: a directory that could not be read is not
    // a directory that was verified.
    match remove_warranted(&warrant, &HardDelete, None) {
        Err(RemovalRefusal::WarrantFailed(clause)) => {
            assert!(clause.contains("re-derived"), "{clause}");
        }
        other => panic!("expected a warrant refusal, got {other:?}"),
    }
    assert!(copy.exists());

    // And the matching identity removes it.
    assert_eq!(
        remove_warranted(&warrant, &HardDelete, Some(&expected)),
        Ok(RemovalOutcome::HardDeleted)
    );
    assert!(!copy.exists());
}

/// An uninstall warrant is refused on a symlink, without following it — the same guard the
/// staging warrant gets, because a link at a working copy's path aims somewhere else entirely.
#[test]
fn an_uninstall_warrant_on_a_symlink_is_refused_without_following_it() {
    use codotheca_core::git::RootCommit;
    use codotheca_core::protocol::{LocationId, UninstallDisposition};
    use codotheca_core::uninstall::VerdictSeal;

    let dir = tempfile::tempdir().expect("tmp");
    let real = dir.path().join("precious");
    std::fs::create_dir_all(&real).expect("mkdir");
    std::fs::write(real.join("keep"), b"x").expect("write");
    let link = dir.path().join("widget");

    #[cfg(unix)]
    std::os::unix::fs::symlink(&real, &link).expect("symlink");
    #[cfg(windows)]
    if std::os::windows::fs::symlink_dir(&real, &link).is_err() {
        eprintln!("removal audit: SKIPPED the uninstall symlink refusal — no symlink privilege");
        return;
    }

    let identity = RootCommit {
        oid: "cccccccccccccccccccccccccccccccccccccccc".to_owned(),
        committed_at: 0,
        tz_offset_min: 0,
    };
    let warrant = Warrant::for_uninstall_in_test(
        LocationId(1),
        link,
        identity.clone(),
        VerdictSeal::of(&[], UninstallDisposition::Safe),
    );
    assert_eq!(
        remove_warranted(&warrant, &HardDelete, Some(&identity)),
        Err(RemovalRefusal::SymlinkedPath)
    );
    assert!(
        real.join("keep").exists(),
        "following the link would have removed what it aims at"
    );
}

/// R63, proved at the type level rather than by a runtime check.
///
/// There is no constructor on `Warrant` that accepts a path, and `remove_warranted` takes none,
/// so *"remove `<root>/<seed_basename>` under the staging warrant"* — the real working copy, one
/// directory above the staging root — **cannot be written**. This test records that the guarantee
/// is structural; a compile-fail test is the only thing that could assert it directly, and the
/// absence of the parameter is what makes one unnecessary.
#[test]
fn the_authorised_path_is_not_a_parameter() {
    let dir = tempfile::tempdir().expect("tmp");
    let staging_root = dir.path().join(STAGING_DIR_NAME);
    std::fs::create_dir_all(staging_root.join("widget")).expect("mkdir");

    let warrant = warrant_for(&staging_root, "widget");
    assert_eq!(warrant.path(), staging_root.join("widget"));
    // The only path this value can ever name is the one it was built with. The destination one
    // level up is not reachable through any method on it.
    assert_ne!(warrant.path(), dir.path().join("widget"));
}

/// Build a staging warrant for a directory this process just made, in this session.
fn warrant_for(staging_root: &Path, seed: &str) -> Warrant {
    // Not a public constructor: the test reaches the same builder the sweep does, through the
    // durable row, in `staging_warrant_for`'s own test below. This one exercises the shape.
    Warrant::for_staging_in_test(staging_root.to_path_buf(), seed, SessionNonce::current())
}

#[test]
fn a_path_outside_the_warranted_root_is_refused() {
    let dir = tempfile::tempdir().expect("tmp");
    let staging_root = dir.path().join(STAGING_DIR_NAME);
    std::fs::create_dir_all(&staging_root).expect("mkdir");
    let real = dir.path().join("widget");
    std::fs::create_dir_all(&real).expect("mkdir");

    // A forged root: the escape a lexical parent comparison would have allowed.
    let forged = Warrant::for_staging_in_test(staging_root, "..", SessionNonce::current());
    assert_eq!(
        remove_warranted(&forged, &HardDelete, None),
        Err(RemovalRefusal::OutsideWarrantedRoot)
    );
    assert!(real.exists(), "the working copy must still be there");
}

#[test]
fn a_root_that_is_not_a_staging_directory_is_refused() {
    let dir = tempfile::tempdir().expect("tmp");
    let not_staging = dir.path().join("Projects");
    std::fs::create_dir_all(not_staging.join("widget")).expect("mkdir");

    let warrant =
        Warrant::for_staging_in_test(not_staging.clone(), "widget", SessionNonce::current());
    match remove_warranted(&warrant, &HardDelete, None) {
        Err(RemovalRefusal::WarrantFailed(clause)) => {
            assert!(clause.contains("staging"), "{clause}");
        }
        other => panic!("expected a warrant refusal, got {other:?}"),
    }
    assert!(not_staging.join("widget").exists());
}

#[test]
fn a_warrant_from_another_session_is_refused() {
    let dir = tempfile::tempdir().expect("tmp");
    let staging_root = dir.path().join(STAGING_DIR_NAME);
    std::fs::create_dir_all(staging_root.join("widget")).expect("mkdir");

    let stale = Warrant::for_staging_in_test(
        staging_root.clone(),
        "widget",
        SessionNonce::mint_for_test(),
    );
    match remove_warranted(&stale, &HardDelete, None) {
        Err(RemovalRefusal::WarrantFailed(clause)) => {
            assert!(clause.contains("run of the core"), "{clause}");
        }
        other => panic!("expected a session refusal, got {other:?}"),
    }
    assert!(
        staging_root.join("widget").exists(),
        "a previous run's staging directory is the sweep's to report, not this one's to delete"
    );
}

#[test]
fn a_symlinked_target_is_refused_without_being_followed() {
    let dir = tempfile::tempdir().expect("tmp");
    let staging_root = dir.path().join(STAGING_DIR_NAME);
    std::fs::create_dir_all(&staging_root).expect("mkdir");
    let real = dir.path().join("precious");
    std::fs::create_dir_all(&real).expect("mkdir");
    std::fs::write(real.join("keep"), b"x").expect("write");

    let link = staging_root.join("widget");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&real, &link).expect("symlink");
    #[cfg(windows)]
    if std::os::windows::fs::symlink_dir(&real, &link).is_err() {
        // Creating a symlink on Windows needs either developer mode or the privilege. Skipping
        // is stated rather than passed over: a skip that reads as a pass is how a gate stops
        // meaning anything.
        eprintln!("removal audit: SKIPPED the symlink refusal — no symlink privilege");
        return;
    }

    let warrant = Warrant::for_staging_in_test(staging_root, "widget", SessionNonce::current());
    assert_eq!(
        remove_warranted(&warrant, &HardDelete, None),
        Err(RemovalRefusal::SymlinkedPath)
    );
    assert!(
        real.join("keep").exists(),
        "following the link would have removed what it aims at"
    );
}

#[test]
fn a_warranted_staging_directory_is_hard_deleted() {
    let dir = tempfile::tempdir().expect("tmp");
    let staging_root = dir.path().join(STAGING_DIR_NAME);
    std::fs::create_dir_all(staging_root.join("widget").join("objects")).expect("mkdir");

    let warrant = warrant_for(&staging_root, "widget");
    assert_eq!(
        remove_warranted(&warrant, &HardDelete, None),
        Ok(RemovalOutcome::HardDeleted),
        "a partial clone is never reported as recoverable"
    );
    assert!(!staging_root.join("widget").exists());
    assert!(
        staging_root.exists(),
        "the staging root itself is not the target"
    );
}

#[test]
fn a_run_with_no_durable_row_warrants_nothing() {
    let index = codotheca_core::testing::TempIndex::new();
    let _tx_guard = codotheca_core::proto::txguard::TxGuard::enter();
    let binding = index.index();
    let tx = binding
        .conn()
        .unchecked_transaction()
        .expect("read transaction");
    let absent = staging_warrant_for(&tx, codotheca_core::protocol::InstallRunId(4242))
        .expect("the read itself must succeed");
    assert!(
        absent.is_none(),
        "with no install_run row there is no evidence, so there is no warrant"
    );
}

/// §24.3b: refused **before the clone spawns**, which is the only moment at which it is free.
#[test]
fn a_cross_device_staging_root_is_refused_before_anything_is_spawned() {
    let dir = tempfile::tempdir().expect("tmp");
    let composed = staging_path_for(dir.path(), "widget").expect("same device composes");
    assert!(composed.starts_with(dir.path()));

    // A staging root that is a symlink to another tree is the reachable form of "a different
    // device" on a test machine with one filesystem: `file_id` reports the target's identity,
    // so this asserts the check reads identity rather than spelling.
    #[cfg(unix)]
    {
        let elsewhere = tempfile::tempdir().expect("tmp2");
        let staging_root = dir.path().join(STAGING_DIR_NAME);
        std::os::unix::fs::symlink(elsewhere.path(), &staging_root).expect("symlink");
        let same_volume = {
            use codotheca_core::scan::links::file_id;
            file_id(dir.path()).ok().map(|a| a.volume)
                == file_id(&staging_root).ok().map(|a| a.volume)
        };
        if same_volume {
            eprintln!(
                "removal audit: SKIPPED the cross-device refusal — \
                 both temporary roots are on one volume"
            );
        } else {
            assert!(
                staging_path_for(dir.path(), "widget").is_err(),
                "a staging root on another device must be refused before the clone spawns"
            );
        }
    }
}
