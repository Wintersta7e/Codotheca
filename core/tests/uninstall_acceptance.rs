#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

//! §24's acceptance criteria, stated as the criteria state them.
//!
//! **This file is not a second copy of the unit suites.** `uninstall_preflight.rs`,
//! `uninstall_gates.rs`, `uninstall_verdict.rs` and `uninstall_command.rs` each prove one seam
//! behaves; a criterion is a sentence about the *product*, and R46's finding was that the
//! structure making something checkable gets built while the check itself is assumed to be
//! somebody's next step. Each test below is named for its criterion and asserts the whole
//! sentence, including the half a unit test has no reason to state — "**none of the four ever
//! yields `safe`**" is the half that matters and the half no per-gate test says.

use std::path::{Path, PathBuf};

use codotheca_core::git::RootCommit;
use codotheca_core::protocol::{
    BackupState, LocationId, UninstallBlocker, UninstallDisposition, UninstallVerdict,
};
use codotheca_core::removal::{Warrant, WarrantVariant};
use codotheca_core::uninstall::gates::RemoteOutcome;
use codotheca_core::uninstall::verdict::VerdictSeal;
use codotheca_core::uninstall::{
    compute_verdict, is_unknown_blocker, read_stash_truth, uninstall_location, LocationSnapshot,
    StashTruth, VerdictInputs,
};

fn identity() -> RootCommit {
    RootCommit {
        oid: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_owned(),
        committed_at: 0,
        tz_offset_min: 0,
    }
}

/// A copy that clears every gate. Each test below spoils exactly one thing, so a failure names
/// the gate that moved rather than the fixture.
fn clean_inputs(root: &Path, copy: &Path) -> VerdictInputs {
    VerdictInputs {
        snapshot: LocationSnapshot {
            id: LocationId(1),
            path: copy.to_path_buf(),
            refstate_observed_at: Some(10),
            worktree_observed_at: Some(11),
            is_shallow: false,
            removed_at: None,
        },
        roots: vec![root.to_path_buf()],
        remote: RemoteOutcome::Reached,
        unique: Vec::new(),
        live_session: false,
        now: 1_700_000_000,
    }
}

fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let dir = tempfile::tempdir().expect("tmp");
    let root = dir.path().join("library");
    let copy = root.join("widget");
    std::fs::create_dir_all(copy.join(".git")).expect("mkdir");
    std::fs::write(copy.join("a.txt"), b"one").expect("write");
    (dir, root, copy)
}

fn verdict_for(inputs: &VerdictInputs) -> UninstallVerdict {
    compute_verdict(inputs).expect("computed").0
}

// ---------------------------------------------------------------------------
// AC-P2-24-13 — recompute inside, refuse if changed, no verdict token crosses
// ---------------------------------------------------------------------------

/// The criterion's **second** sentence, which the mutation test in `uninstall_command.rs` has no
/// reason to state: *"A `LocationId` is the only argument; no verdict token crosses."*
///
/// A token that crossed the wire would make the refusal above decorative — the caller would hand
/// back the answer it was given, and the recomputation would have nothing to disagree with. The
/// schema is where that is decidable, so the schema is what this reads.
#[test]
fn ac_p2_24_13_no_verdict_token_crosses_the_wire() {
    let schema = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("protocol")
            .join("schema")
            .join("protocol.json"),
    )
    .expect("the schema is tracked and readable");
    let parsed: serde_json::Value = serde_json::from_str(&schema).expect("the schema parses");

    let command = parsed["commands"]
        .as_array()
        .expect("`commands` is a list")
        .iter()
        .find(|c| c["name"] == "locations.uninstall")
        .expect("`locations.uninstall` is declared");
    let args = command["args"].as_object().expect("it takes arguments");

    let names: Vec<&String> = args.keys().collect();
    assert_eq!(
        names,
        vec!["locationId"],
        "a LocationId is the only argument; anything else is a token the core would have to trust"
    );

    // `VerdictSeal` is the in-core identity of a computed answer. It is not a wire type, and the
    // way it becomes one is somebody adding it to a struct that is.
    assert!(
        !schema.contains("VerdictSeal"),
        "the seal is an in-process check, never a token a caller can present"
    );
}

// ---------------------------------------------------------------------------
// AC-P2-24-15 — the unreadable stash, end to end
// ---------------------------------------------------------------------------

/// **The criterion's own limb**: `logs/refs/stash` *unreadable (permission denied)* — not absent,
/// not empty — reaches disposition `unknown`.
///
/// `uninstall_preflight.rs` proves `read_stash_truth` answers `Unreadable`; this proves the
/// answer survives the fold. The two are different failures: a reader that is right and a fold
/// that collapses it to `safe` ships a shredder, and only this test would say so.
///
/// **Unix only, and it is counted.** `chmod 000` is the only way to make a file exist and not be
/// readable, and Windows has no equivalent a test can rely on — an ACL denial needs a second
/// principal. The `read_stash_truth` half runs on both platforms in `uninstall_preflight.rs`;
/// this end-to-end half is one of the platform-conditional tests the report names by identity.
#[cfg(unix)]
#[test]
fn ac_p2_24_15_an_unreadable_stash_reflog_is_unknown_and_never_safe() {
    use std::os::unix::fs::PermissionsExt;

    let (_dir, root, copy) = fixture();
    let common = copy.join(".git");
    let logs = common.join("logs").join("refs");
    std::fs::create_dir_all(&logs).expect("mkdir");
    let reflog = logs.join("stash");
    std::fs::write(&reflog, b"whatever").expect("write");
    std::fs::set_permissions(&reflog, std::fs::Permissions::from_mode(0o000)).expect("chmod");

    // Running as root defeats the mode bits entirely, and a test that quietly passes because the
    // denial never happened is worse than no test — so the root case fails loudly rather than
    // reading as an absent stash.
    let truth = read_stash_truth(&common);
    assert_eq!(
        truth,
        StashTruth::Unreadable,
        "chmod 000 did not deny the read — this suite cannot run as root"
    );

    let mut inputs = clean_inputs(&root, &copy);
    inputs.unique.push(UninstallBlocker::StashUnreadable);
    let verdict = verdict_for(&inputs);

    assert_eq!(
        verdict.disposition,
        UninstallDisposition::Unknown,
        "an input nobody could read is not an absent stash"
    );
    assert!(verdict
        .blockers
        .contains(&UninstallBlocker::StashUnreadable));
    assert_ne!(verdict.disposition, UninstallDisposition::Safe);
}

/// The other half of the same criterion: a stash reachable **only** through a packed `refs/stash`
/// is a stash. A reader that checked the reflog and the loose ref would answer `None` here.
#[test]
fn ac_p2_24_15_a_packed_only_stash_is_a_stash() {
    let (_dir, _root, copy) = fixture();
    let common = copy.join(".git");
    std::fs::write(
        common.join("packed-refs"),
        b"# pack-refs with: peeled fully-peeled sorted \n\
          0123456789abcdef0123456789abcdef01234567 refs/stash\n",
    )
    .expect("write");

    assert_eq!(
        read_stash_truth(&common),
        StashTruth::Present(1),
        "one is a floor the ref proves, not a guess at how many"
    );
}

// ---------------------------------------------------------------------------
// AC-P2-24-16 — the four remote and history conditions, and the half that matters
// ---------------------------------------------------------------------------

/// *"None of the four ever yields `safe`."* Each per-gate test proves its own blocker appears;
/// **this proves the disposition**, which is the sentence the product rests on.
#[test]
fn ac_p2_24_16_no_remote_or_history_failure_ever_yields_safe() {
    let (_dir, root, copy) = fixture();

    // 401 / 403 / 404, and an offline machine.
    for outcome in [RemoteOutcome::Refused, RemoteOutcome::Unreachable] {
        let mut inputs = clean_inputs(&root, &copy);
        inputs.remote = outcome;
        let verdict = verdict_for(&inputs);
        assert_eq!(
            verdict.disposition,
            UninstallDisposition::Unknown,
            "{outcome:?} is *I could not check*, never *I checked and it is fine*"
        );
        assert_eq!(
            verdict.remote_verified_at, None,
            "{outcome:?} verified nothing"
        );
    }

    // A remote URL that resolves to a path on this machine is not a backup.
    let mut mirrored = clean_inputs(&root, &copy);
    mirrored.remote = RemoteOutcome::LocalMirror;
    let verdict = verdict_for(&mirrored);
    assert!(verdict
        .blockers
        .contains(&UninstallBlocker::RemoteIsLocalMirror));
    assert_ne!(verdict.disposition, UninstallDisposition::Safe);

    // A shallow clone holds history no remote has, whatever the remote says.
    let mut shallow = clean_inputs(&root, &copy);
    shallow.snapshot.is_shallow = true;
    let shallow_verdict = verdict_for(&shallow);
    assert!(shallow_verdict
        .blockers
        .contains(&UninstallBlocker::ShallowClone));
    assert_ne!(shallow_verdict.disposition, UninstallDisposition::Safe);
}

/// The fold's totality, stated as the criterion implies it rather than as the enum declares it:
/// **every blocker the schema declares, alone, keeps the answer away from `safe`.** The set is the
/// generated `UninstallBlocker::ALL` and its size is printed, never written here: a hand count
/// is one value stated twice, and it was wrong the moment §45 added eight.
#[test]
fn ac_p2_24_16_every_blocker_alone_keeps_the_answer_away_from_safe() {
    let (_dir, root, copy) = fixture();
    let all = UninstallBlocker::ALL;
    assert!(!all.is_empty(), "the schema declares zero blockers");
    eprintln!("ac_p2_24_16: {} blockers, each alone", all.len());

    for blocker in all {
        let mut inputs = clean_inputs(&root, &copy);
        inputs.unique.push(blocker);
        let verdict = verdict_for(&inputs);
        assert_ne!(
            verdict.disposition,
            UninstallDisposition::Safe,
            "{blocker:?} alone still permitted a removal"
        );
        let expected = if is_unknown_blocker(blocker) {
            UninstallDisposition::Unknown
        } else {
            UninstallDisposition::Blocked
        };
        assert_eq!(verdict.disposition, expected, "{blocker:?}");
    }
}

// ---------------------------------------------------------------------------
// AC-P2-24-18 — one warranted primitive, and the audit covers every variant of it
// ---------------------------------------------------------------------------

/// The file-walking, count-printing half is `core/tests/removal_audit.rs` (p2-24's, and it is
/// that file that fails at a zero scan). **This is the half p2-24 could not assert**: the audit is
/// not a claim about the finished boundary until every warrant kind is one it covers, and this
/// plan is what adds the second kind. R61 records that neither plan may report this id closed on
/// its own.
#[test]
fn ac_p2_24_18_the_warrant_set_grew_and_the_audit_covers_all_of_it() {
    assert_eq!(
        Warrant::ALL.len(),
        2,
        "install's staging sweep and uninstall; a third kind needs the audit's floor raised with it"
    );
    assert!(Warrant::ALL.contains(&WarrantVariant::Uninstall));
}

// ---------------------------------------------------------------------------
// AC-P2-25-11-unknown — the fall-through row p2-25 could not reach
// ---------------------------------------------------------------------------

/// §25.3's row 4: *"`ahead` NULL, `stash_count` unknown, or no fetch recorded → no block."*
///
/// **p2-25 may not claim this row.** Its `stash_count` limb was unreachable until Task 2 made
/// `read_stash_count` return an `Option`, so p2-25's suite exercises rows 1–3 and the other two
/// limbs of row 4 and names this test as the fourth's owner. Driving the producer with
/// `stash_count: None` is the whole assertion, and `Some(BackupState::Verified)` is the failure
/// it exists to catch — an unknown stash rendered as a verified backup.
#[test]
fn ac_p2_25_11_unknown_an_unknown_stash_count_produces_no_block() {
    use codotheca_core::remote::backup::backup_state;

    const KEY: Option<&str> = Some("example.invalid/acme/widget");
    const FETCHED: Option<i64> = Some(1_781_179_200);

    assert_eq!(
        backup_state(KEY, Some(0), None, FETCHED),
        None,
        "an unreadable stash is not a verified backup and is not a block either"
    );

    // The neighbouring rows still answer, so the assertion above is the fall-through and not a
    // producer that has stopped answering at all.
    assert_eq!(
        backup_state(KEY, Some(0), Some(0), FETCHED),
        Some(BackupState::Verified)
    );
    assert_eq!(
        backup_state(KEY, Some(0), Some(1), FETCHED),
        Some(BackupState::NotAnywhereElse)
    );
    assert_eq!(
        backup_state(None, Some(0), Some(0), FETCHED),
        Some(BackupState::OnlyCopy)
    );
}

// ---------------------------------------------------------------------------
// The criterion behind all of them: nothing here removes anything it did not prove safe.
// ---------------------------------------------------------------------------

/// A verdict of `unknown` is not a weaker `safe`. The mutating call takes a warrant sealed over
/// the answer the pre-flight reached, and a seal over a non-`safe` answer must not open the door.
#[test]
fn an_unknown_verdict_never_reaches_the_filesystem() {
    let (_dir, root, copy) = fixture();
    let index = codotheca_core::testing::TempIndex::new();
    let project = index.insert_project();
    let location = index.insert_location(project, "/r/widget");

    let mut inputs = clean_inputs(&root, &copy);
    inputs.snapshot.id = location;
    inputs.remote = RemoteOutcome::Unreachable;
    assert_eq!(
        verdict_for(&inputs).disposition,
        UninstallDisposition::Unknown
    );

    // A warrant that claims `safe` over a tree that is not: the recomputation is what refuses.
    let warrant = Warrant::for_uninstall_in_test(
        location,
        copy.clone(),
        identity(),
        VerdictSeal::of(&[], UninstallDisposition::Safe),
    );

    let _guard = codotheca_core::proto::txguard::TxGuard::enter();
    let binding = index.index();
    let tx = binding.conn().unchecked_transaction().expect("tx");
    assert!(
        uninstall_location(&tx, &inputs, &warrant, Some(&identity())).is_err(),
        "a forged seal must not outrank the core's own recomputation"
    );
    assert!(copy.exists(), "the copy is still on disk");
}
