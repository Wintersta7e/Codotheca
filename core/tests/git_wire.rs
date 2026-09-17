//! §13 — every git result the worker produces has to survive the hop out of the distro.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

// `core::git`'s submodules are private and every transported type is re-exported from the
// module root, so an integration test names them there. Inside the crate the module paths the
// plan writes still resolve.
use codotheca_core::git::{
    Authorship, CommitSubject, CommitterTally, Divergence, GitVersion, InterruptedOp,
    RefFingerprint, RefState, RepoFacts, RootCommit, TrackedInventory, UpstreamRef, WorktreeStatus,
};
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

/// Serialise, parse, serialise again. Equal JSON on both sides proves nothing was dropped,
/// without requiring `PartialEq` on types that do not derive it.
fn round_trip<T: Serialize + DeserializeOwned>(value: &T) -> String {
    let first = serde_json::to_string(value).expect("serialises");
    let back: T = serde_json::from_str(&first).expect("parses");
    let second = serde_json::to_string(&back).expect("re-serialises");
    assert_eq!(first, second, "the value changed shape crossing the wire");
    first
}

const DIGEST: &str = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";

#[test]
fn a_fingerprint_travels_as_hex_and_a_non_digest_is_refused_on_the_way_in() {
    // R5/R28: the basis is 64 lowercase hex — `refstate_basis` is a TEXT column holding a
    // SHA-256 — and it crosses the wire as that bare string, not as a wrapper object.
    let fp = RefFingerprint::from_hex(DIGEST).expect("parses");
    let json = round_trip(&fp);
    assert_eq!(json, format!("\"{DIGEST}\""));

    assert_eq!(RefFingerprint::from_hex("zz"), None);
    // The one validator is `from_hex`, and deserialising has to go through it: a string that is
    // not a digest must never become a `RefFingerprint` and reach the column.
    assert!(serde_json::from_str::<RefFingerprint>("\"zz\"").is_err());
    assert!(
        serde_json::from_str::<RefFingerprint>(&format!("\"{}\"", DIGEST.to_uppercase())).is_err()
    );
}

#[test]
fn ref_state_survives_the_hop() {
    let state = RefState {
        head_oid: Some("0123456789abcdef0123456789abcdef01234567".to_owned()),
        branch: Some("main".to_owned()),
        upstream: Some(UpstreamRef {
            remote: "origin".to_owned(),
            ref_name: "refs/remotes/origin/main".to_owned(),
            oid: None,
        }),
        ahead: Some(2),
        behind: Some(0),
        tag_count: 3,
        stash_count: Some(0),
        is_shallow: true,
        is_bare: false,
        interrupted_op: Some(InterruptedOp::Rebase),
        fetch_head_at: Some(1_700_000_000),
        reflog_tail_at: Some(1_699_999_999),
        basis: RefFingerprint::from_hex(DIGEST).expect("parses"),
        observed_at: 1_700_000_001,
    };
    let json = round_trip(&state);
    // The interrupted operation travels as the slug `as_str` already returns, not as `Rebase`.
    assert!(json.contains("\"interrupted_op\":\"rebase\""), "{json}");
}

#[test]
fn every_other_transported_result_survives_the_hop() {
    round_trip(&GitVersion {
        major: 2,
        minor: 43,
        patch: 0,
        raw: "git version 2.43.0".to_owned(),
    });
    round_trip(&RepoFacts {
        is_bare: false,
        is_shallow: false,
        git_dir: PathBuf::from("/home/me/widget/.git"),
        common_dir: PathBuf::from("/home/me/widget/.git"),
    });
    round_trip(&Divergence {
        ahead: 2,
        behind: 0,
    });
    round_trip(&WorktreeStatus {
        is_dirty: true,
        tracked_changes: 4,
        untracked_count: None,
        branch: Some("main".to_owned()),
        head_oid: None,
        ahead: Some(1),
        behind: None,
        observed_at: 1_700_000_002,
    });
    let mut extensions = BTreeMap::new();
    extensions.insert("rs".to_owned(), 4096_u64);
    round_trip(&TrackedInventory {
        tracked_files: 12,
        size_tracked_bytes: 4096,
        extension_bytes: extensions,
        // Raw path bytes, because a Linux path is not a `String`. They have to survive too:
        // §1.2's archetype is decided by basenames the extension histogram cannot express.
        paths: vec![b"src/main.rs".to_vec(), vec![0x66, 0xFF, 0x2E, 0x72, 0x73]],
        observed_at: 1_700_000_003,
    });
    round_trip(&RootCommit {
        oid: "abc".to_owned(),
        committed_at: 1_600_000_000,
        tz_offset_min: -120,
    });
    let mut days = BTreeSet::new();
    days.insert(19_000_i64);
    round_trip(&Authorship {
        committers: vec![CommitterTally {
            email: "someone@example.invalid".to_owned(),
            commits: 7,
            days,
            last_commit_at: 1_600_000_500,
        }],
    });
    round_trip(&CommitSubject {
        oid: "def".to_owned(),
        committed_at: 1_600_000_001,
        subject: "a subject".to_owned(),
    });
}

#[test]
fn a_non_utf8_tracked_path_is_not_silently_lost() {
    // A field added to a struct and omitted from a fixture is a field nothing proves survives.
    // These bytes are not valid UTF-8 and must come back exactly.
    let raw = vec![0x2F, 0x74, 0xC3, 0x28, 0x2E, 0x74, 0x78, 0x74];
    let inventory = TrackedInventory {
        tracked_files: 1,
        size_tracked_bytes: 3,
        extension_bytes: BTreeMap::new(),
        paths: vec![raw.clone()],
        observed_at: 1,
    };
    let text = serde_json::to_string(&inventory).expect("serialises");
    let back: TrackedInventory = serde_json::from_str(&text).expect("parses");
    assert_eq!(back.paths, vec![raw]);
}

/// **[p2-24b] R51: all three stash states survive the hop to the WSL worker.**
///
/// `RefState` is serialised to the worker (`core/src/wsl/proto.rs` carries `state: Box<RefState>`),
/// so **the worker binary moves with this struct**: one built before the field became nullable and
/// a core built after disagree about its shape, and nothing on the wire says so. This is the gate
/// that says so instead.
#[test]
fn every_stash_state_survives_the_hop_to_the_worker() {
    let base = RefState {
        head_oid: Some("0123456789abcdef0123456789abcdef01234567".to_owned()),
        branch: Some("main".to_owned()),
        upstream: None,
        ahead: None,
        behind: None,
        tag_count: 0,
        stash_count: Some(0),
        is_shallow: false,
        is_bare: false,
        interrupted_op: None,
        fetch_head_at: None,
        reflog_tail_at: None,
        basis: RefFingerprint::from_hex(DIGEST).expect("parses"),
        observed_at: 1,
    };

    // `Some(0)` is *no stash* — a real observation — and must not arrive as `null`.
    let none_present = round_trip(&RefState {
        stash_count: Some(0),
        ..base.clone()
    });
    assert!(none_present.contains("\"stash_count\":0"), "{none_present}");

    let counted = round_trip(&RefState {
        stash_count: Some(2),
        ..base.clone()
    });
    assert!(counted.contains("\"stash_count\":2"), "{counted}");

    // `None` is *unreadable* and must arrive as `null`, never as 0. A worker that collapsed the
    // two would hand the core a false all-clear across the process boundary.
    let unreadable = round_trip(&RefState {
        stash_count: None,
        ..base
    });
    assert!(unreadable.contains("\"stash_count\":null"), "{unreadable}");
}
