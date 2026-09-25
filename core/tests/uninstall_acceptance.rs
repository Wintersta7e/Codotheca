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

mod support;

use std::path::{Path, PathBuf};

use codotheca_core::analyser::identity::LiveIdentity;
use codotheca_core::analyser::remote::DidNotAnswer;
use codotheca_core::analyser::verdict::{fold_disposition, is_unknown_blocker, VerdictSeal};
use codotheca_core::analyser::Analysis;
use codotheca_core::protocol::{
    BackupState, UninstallBlocker, UninstallDisposition, UninstallVerdict,
};
use codotheca_core::removal::{SystemTrash, Warrant, WarrantVariant};
use codotheca_core::testing::FixtureRemoteVerifier;
use codotheca_core::uninstall::uninstall_location;
use support::analyser_world::Library;

/// The row's lineage every warrant here expects; `LiveIdentity::Derived` of it is a match.
fn lineage() -> String {
    "b".repeat(64)
}

fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let dir = tempfile::tempdir().expect("tmp");
    let root = dir.path().join("library");
    let copy = root.join("widget");
    std::fs::create_dir_all(copy.join(".git")).expect("mkdir");
    std::fs::write(copy.join("a.txt"), b"one").expect("write");
    (dir, root, copy)
}

/// A pushed copy with one stash entry, registered: the shape both stash criteria start from.
fn stashed(lib: &Library) -> (PathBuf, codotheca_core::protocol::LocationId) {
    let copy = lib.pushed_repo("widget");
    std::fs::write(copy.join("a.txt"), b"stashed work\n").expect("edit");
    lib.git(&copy, &["stash", "push", "-q", "-m", "work"]);
    let id = lib.register(&copy);
    (copy, id)
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

    let lib = Library::new();
    let (copy, id) = stashed(&lib);
    let reflog = copy.join(".git").join("logs").join("refs").join("stash");
    assert!(reflog.is_file(), "the fixture's stash has a reflog to deny");
    std::fs::set_permissions(&reflog, std::fs::Permissions::from_mode(0o000)).expect("chmod");

    // Running as root defeats the mode bits entirely, and a test that quietly passes because the
    // denial never happened is worse than no test — so the root case fails loudly rather than
    // reading as an absent stash.
    assert!(
        std::fs::read(&reflog).is_err(),
        "chmod 000 did not deny the read — this suite cannot run as root"
    );

    let verdict = lib.preflight(id, &lib.verifier());
    std::fs::set_permissions(&reflog, std::fs::Permissions::from_mode(0o644)).expect("chmod back");
    eprintln!("an unreadable stash reflog: {verdict}");
    assert_eq!(
        verdict.disposition(),
        "unknown",
        "an input nobody could read is not an absent stash: {verdict}"
    );
    assert!(verdict.has("stash_unreadable"), "{verdict}");
}

/// The other half of the same criterion: a stash reachable **only** through a packed `refs/stash`
/// is a stash. A reader that checked the reflog and the loose ref would answer `None` here.
#[test]
fn ac_p2_24_15_a_packed_only_stash_is_a_stash() {
    let lib = Library::new();
    let (copy, id) = stashed(&lib);
    let common = copy.join(".git");
    lib.git(&copy, &["pack-refs", "--all"]);
    let _ = std::fs::remove_file(common.join("refs").join("stash"));
    let _ = std::fs::remove_file(common.join("logs").join("refs").join("stash"));
    assert!(
        std::fs::read_to_string(common.join("packed-refs"))
            .expect("packed-refs")
            .contains("refs/stash"),
        "the fixture's stash is reachable only through packed-refs"
    );

    let verdict = lib.preflight(id, &lib.verifier());
    eprintln!("a packed-only stash: {verdict}");
    assert!(
        verdict.has("stash_present"),
        "one is a floor the ref proves, not a guess at how many: {verdict}"
    );
    assert_eq!(verdict.disposition(), "blocked");
}

// ---------------------------------------------------------------------------
// AC-P2-24-16 — the four remote and history conditions, and the half that matters
// ---------------------------------------------------------------------------

/// *"None of the four ever yields `safe`."* Each per-gate test proves its own blocker appears;
/// **this proves the disposition**, which is the sentence the product rests on.
#[test]
fn ac_p2_24_16_no_remote_or_history_failure_ever_yields_safe() {
    // 401 / 403 / 404, an offline machine, and the deadline: a remote that did not answer.
    for why in [
        DidNotAnswer::Failed,
        DidNotAnswer::Deadline,
        DidNotAnswer::Unresolved,
    ] {
        let lib = Library::new();
        let copy = lib.pushed_repo("widget");
        let id = lib.register(&copy);
        let silent = FixtureRemoteVerifier::new();
        silent.silent("origin", why);
        let verdict = lib.preflight(id, &silent);
        assert_eq!(
            verdict.disposition(),
            "unknown",
            "{why:?} is *I could not check*, never *I checked and it is fine*: {verdict}"
        );
        assert!(
            verdict.0["remoteVerifiedAt"].is_null(),
            "{why:?} verified nothing"
        );
    }

    // A remote URL that resolves to a path on this machine is not a backup — read by the
    // production verifier, which admits no local transport.
    {
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
        lib.push_to(&copy, "origin", &mirror);
        let id = lib.register(&copy);
        let production =
            codotheca_core::analyser::remote::GitRemoteVerifier::new(&lib.write_git, &lib.read_git);
        let verdict = lib.preflight(id, &production);
        eprintln!("a same-machine mirror as the only remote: {verdict}");
        assert!(verdict.has("remote_is_local_mirror"), "{verdict}");
        assert_ne!(verdict.disposition(), "safe");
    }

    // A shallow clone holds history no remote has, whatever the remote says.
    {
        let lib = Library::new();
        let copy = lib.shallow_repo("widget");
        let id = lib.register(&copy);
        let shallow = lib.preflight(id, &lib.verifier());
        assert!(shallow.has("shallow_clone"), "{shallow}");
        assert_ne!(shallow.disposition(), "safe");
    }
}

/// The fold's totality, stated as the criterion implies it rather than as the enum declares it:
/// **every blocker the schema declares, alone, keeps the answer away from `safe`.** The set is the
/// generated `UninstallBlocker::ALL` and its size is printed, never written here: a hand count
/// is one value stated twice, and it was wrong the moment §45 added eight.
#[test]
fn ac_p2_24_16_every_blocker_alone_keeps_the_answer_away_from_safe() {
    let all = UninstallBlocker::ALL;
    assert!(!all.is_empty(), "the schema declares zero blockers");
    eprintln!("ac_p2_24_16: {} blockers, each alone", all.len());

    // [p4] The analyser's own fold, over each blocker alone: AC-P4-45-1 (every blocker produced
    // through production) replaces this loop as the coverage claim.
    for blocker in all {
        let disposition = fold_disposition(&[blocker]);
        assert_ne!(
            disposition,
            UninstallDisposition::Safe,
            "{blocker:?} alone still permitted a removal"
        );
        let expected = if is_unknown_blocker(blocker) {
            UninstallDisposition::Unknown
        } else {
            UninstallDisposition::Blocked
        };
        assert_eq!(disposition, expected, "{blocker:?}");
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
    let (_dir, _root, copy) = fixture();
    let index = codotheca_core::testing::TempIndex::new();
    let project = index.insert_project();
    let location = index.insert_location(project, "/r/widget");

    let blockers = vec![UninstallBlocker::RemoteUnreachable];
    let disposition = fold_disposition(&blockers);
    assert_eq!(disposition, UninstallDisposition::Unknown);
    let analysis = Analysis {
        seal: VerdictSeal::of(&blockers, disposition),
        verdict: UninstallVerdict {
            disposition,
            blockers,
            remote_verified_at: None,
            trash_available: true,
            computed_at: 1_700_000_000,
            nested: Vec::new(),
            precious: None,
            trash_refusal: None,
        },
    };

    // A warrant that claims `safe` over a tree that is not: the analysis is what refuses.
    let warrant = Warrant::for_uninstall_in_test(
        location,
        copy.clone(),
        Some(lineage()),
        VerdictSeal::of(&[], UninstallDisposition::Safe),
    );

    let _guard = codotheca_core::proto::txguard::TxGuard::enter();
    let binding = index.index();
    let tx = binding.conn().unchecked_transaction().expect("tx");
    assert!(
        uninstall_location(
            &tx,
            &analysis,
            &warrant,
            &SystemTrash,
            &LiveIdentity::Derived(Some(lineage())),
            1_700_000_000,
        )
        .is_err(),
        "a forged seal must not outrank the core's own analysis"
    );
    assert!(copy.exists(), "the copy is still on disk");
}
