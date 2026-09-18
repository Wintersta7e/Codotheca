//! §28.1 — what an item is keyed on, and what is merely an attribute of it.
//!
//! **An item's identity is `(subject_key, source, fingerprint)`**, and `subject_key` is
//! `ProjectSubject::to_key()` and never `project_id`: §1.7 records that v1 keyed the ledger on
//! `project_id` and it broke on merges. `project_id` is an attribute, repointed by the merge
//! recompute, never key material.
//!
//! **The path, the line and the column are attributes and never key material.** A rename closes
//! nothing, a line move closes nothing, and two identical marker texts are two items.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use codotheca_core::debt::identity::{normalise_salient, DebtKey, SALIENT_CAP_BYTES};
use codotheca_core::protocol::DebtSource;

const SUBJECT: &str = "lineage:abc123|remote:github.com/o/r";

fn sha_of(raw: &str) -> String {
    use sha2::Digest as _;
    let mut h = sha2::Sha256::new();
    h.update(normalise_salient(raw.as_bytes()).as_bytes());
    format!("{:x}", h.finalize())
}

// ---------------------------------------------------------------------------------------------
// The three fingerprint shapes
// ---------------------------------------------------------------------------------------------

/// Two identical marker texts are **two items**, distinguished by the per-project ordinal and by
/// nothing else. The path, the line and the column do not enter the key.
#[test]
fn two_identical_markers_differ_only_in_the_ordinal() {
    let sha = sha_of("TODO: the same words");
    let first = DebtKey::content(SUBJECT, &sha, 0);
    let second = DebtKey::content(SUBJECT, &sha, 1);

    assert_ne!(first, second, "one identity for two occurrences");
    assert_eq!(first.subject_key, second.subject_key);
    assert_eq!(first.source, second.source);
    assert_eq!(first.source, DebtSource::TodoMarker);
    assert_eq!(first.fingerprint, format!("{sha}:0"));
    assert_eq!(second.fingerprint, format!("{sha}:1"));
}

/// A rename, a line move and a column move all produce the **same** key, because none of the
/// three is an input to it. This is what makes `refresh` a refresh and not a close-and-reopen.
#[test]
fn a_rename_a_line_move_and_a_column_move_close_nothing() {
    let sha = sha_of("TODO: unchanged text");
    let before = DebtKey::content(SUBJECT, &sha, 0);
    // There is nothing to vary: the constructor takes no path, no line and no column, which is
    // the assertion. Re-deriving from the same salient hash and ordinal is the whole test.
    let after = DebtKey::content(SUBJECT, &sha, 0);
    assert_eq!(before, after);

    // And a *reworded* marker is a different item; the app is allowed to think so.
    let reworded = DebtKey::content(SUBJECT, &sha_of("TODO: different words"), 0);
    assert_ne!(before, reworded);
}

/// **A5.** The external fingerprint excludes the version and uses the advisory id. A bump from
/// one vulnerable version to another must not close and reopen.
#[test]
fn a_version_bump_changes_no_external_fingerprint() {
    let a = DebtKey::external(SUBJECT, "npm", "left-pad", "GHSA-aaaa-bbbb-cccc");
    let b = DebtKey::external(SUBJECT, "npm", "left-pad", "GHSA-aaaa-bbbb-cccc");
    assert_eq!(a, b);
    assert_eq!(a.fingerprint, "npm:left-pad:GHSA-aaaa-bbbb-cccc");
    assert_eq!(a.source, DebtSource::DependencyAdvisory);

    // A different advisory on the same package is a different item.
    let other = DebtKey::external(SUBJECT, "npm", "left-pad", "GHSA-dddd-eeee-ffff");
    assert_ne!(a, other);
    // So is the same advisory in another ecosystem.
    assert_ne!(
        a,
        DebtKey::external(SUBJECT, "pypi", "left-pad", "GHSA-aaaa-bbbb-cccc")
    );
}

/// A singleton's fingerprint is `''` and never `NULL`. The column is NOT NULL for the reason
/// `0002_locations_and_roots.sql:7-9` records against `location.distro`, and the constructor is
/// the only place that decides it.
#[test]
fn a_singleton_fingerprint_is_the_empty_string() {
    for source in [
        DebtSource::MissingReadme,
        DebtSource::MissingLicense,
        DebtSource::MissingTests,
        DebtSource::NoRelease,
        DebtSource::UnpushedCommits,
        DebtSource::CiRed,
        DebtSource::AbandonedWithDebt,
    ] {
        let key = DebtKey::singleton(SUBJECT, source);
        assert_eq!(
            key.fingerprint, "",
            "{source:?} must key on the empty string"
        );
        assert_eq!(key.source, source);
    }

    // One singleton per source per subject, which is what the UNIQUE index enforces in the DDL
    // and what `Eq + Hash` must agree with here.
    let set: HashSet<DebtKey> = [
        DebtKey::singleton(SUBJECT, DebtSource::MissingReadme),
        DebtKey::singleton(SUBJECT, DebtSource::MissingReadme),
        DebtKey::singleton(SUBJECT, DebtSource::MissingLicense),
    ]
    .into_iter()
    .collect();
    assert_eq!(set.len(), 2);
}

/// `subject_key` is key material and `project_id` is not, so two projects sharing a lineage
/// share an item and two lineages never do.
#[test]
fn the_subject_key_is_key_material() {
    let a = DebtKey::singleton(SUBJECT, DebtSource::MissingReadme);
    let b = DebtKey::singleton("lineage:zzz|remote:", DebtSource::MissingReadme);
    assert_ne!(a, b);
}

// ---------------------------------------------------------------------------------------------
// p3-29's normalisation, read from this file because the cap is part of the identity
// ---------------------------------------------------------------------------------------------

/// A formatter reflowing a wrapped comment must not change an item's identity; a formatter
/// *rewording* it is a different item.
#[test]
fn a_reflow_keeps_the_key_and_a_rewording_does_not() {
    let wrapped = "TODO: this sentence was\n   wrapped by a formatter";
    let reflowed = "TODO: this sentence was wrapped by a formatter";
    assert_eq!(
        normalise_salient(wrapped.as_bytes()),
        normalise_salient(reflowed.as_bytes())
    );
    assert_eq!(
        DebtKey::content(SUBJECT, &sha_of(wrapped), 0),
        DebtKey::content(SUBJECT, &sha_of(reflowed), 0),
    );

    let reworded = "TODO: this sentence was rewrapped by a formatter";
    assert_ne!(
        DebtKey::content(SUBJECT, &sha_of(wrapped), 0),
        DebtKey::content(SUBJECT, &sha_of(reworded), 0),
    );
}

/// The cap is applied at the largest UTF-8 character boundary at or below it — a byte index that
/// splits a code point panics in Rust, and a minified file is a single line of arbitrary length.
#[test]
fn the_cap_lands_on_a_character_boundary() {
    // A three-byte character straddling the cap.
    let raw = "é".repeat(200) + &"x".repeat(400);
    let capped = normalise_salient(raw.as_bytes());
    assert!(capped.len() <= SALIENT_CAP_BYTES);
    assert!(capped.is_char_boundary(capped.len()));
    assert!(
        !capped.is_empty(),
        "the cap must not erase the salient text"
    );
}

// ---------------------------------------------------------------------------------------------
// The one-implementation gate
// ---------------------------------------------------------------------------------------------

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// **R134's enforcement, and it scans the whole core rather than one directory**: the copy this
/// plan was about to create would have been in `core/src/debt/`, and the copy the next author
/// creates will be wherever they are working.
///
/// The assertion is **exactly one**, never *at least one*: a gate asserting *at least one* would
/// pass against the duplicate it exists to catch. It prints the file count scanned and the sites
/// found, and **fails at zero scanned** — a gate whose passing run scans nothing is a failing
/// gate.
#[test]
fn the_cap_and_its_normalisation_have_one_implementation_each() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rust_files(&root, &mut files);
    assert!(
        !files.is_empty(),
        "scanned zero files, so this gate proved nothing"
    );

    let mut cap_sites = Vec::new();
    let mut fn_sites = Vec::new();
    for file in &files {
        let text = std::fs::read_to_string(file).unwrap();
        // `read-scanned.mjs`'s contract has no Rust twin; these files are ours and are not
        // written to by a parallel gate, so a plain read is honest here.
        if text.contains("const SALIENT_CAP_BYTES") {
            cap_sites.push(file.clone());
        }
        if text.contains("fn normalise_salient") {
            fn_sites.push(file.clone());
        }
    }

    eprintln!(
        "one-implementation gate: scanned {} files; SALIENT_CAP_BYTES at {:?}; \
         normalise_salient at {:?}",
        files.len(),
        cap_sites,
        fn_sites,
    );

    let want = root.join("debt").join("identity.rs");
    assert_eq!(
        cap_sites,
        vec![want.clone()],
        "SALIENT_CAP_BYTES must be declared exactly once, at core/src/debt/identity.rs"
    );
    assert_eq!(
        fn_sites,
        vec![want],
        "normalise_salient must be defined exactly once, at core/src/debt/identity.rs"
    );
}
