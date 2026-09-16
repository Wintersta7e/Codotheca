#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
#![cfg(feature = "testkit")]
//! §24.2a: the git **write** audit, asserted over `Intent::ALL` rather than over source text.
//!
//! This file is the sibling of `core/tests/git_readonly.rs` and makes the assertion that one
//! structurally cannot: **the flag denylist**. The read audit skips every literal beginning with
//! `-` except `--version`, and *the difference between an additive `fetch` and a destructive one
//! is a flag*.
//!
//! Every assertion here loops over a rendered set, so an empty set would pass all of them while
//! looking at nothing. [`rendered`] therefore checks the floor **once, before any caller sees
//! the set**, rather than leaving each test to remember.

use codotheca_core::accounts::keychain::SecretToken;
use codotheca_core::gitw::{AuditFixture, Intent};

/// §24.2a assertion 1's allow list.
const WRITE_ALLOWED: &[&str] = &["clone", "fetch"];

/// §24.2a assertion 2's deny list — the eight tokens that turn an additive invocation into a
/// destructive one.
const FLAG_FORBIDDEN: &[&str] = &[
    "--prune", "--force", "-f", "--hard", "--delete", "-d", "-D", "--mirror",
];

/// The sentinel the audit scans for. It is not a real credential and never reaches a forge.
const SENTINEL: &str = "credential-sentinel-do-not-leak";

/// Every variant, rendered over a fixture carrying the sentinel credential.
///
/// The fixture's temporary root is dropped before this returns, and that is deliberate: §24.1
/// requires a clone destination that **does not exist** at call time, so the audit renders
/// against a path guaranteed to be absent.
///
/// **The floor is checked here rather than in each test.** A rendered set shorter than
/// `Intent::ALL` would let every loop below pass by not looking — §26.2's *a gate whose passing
/// run scans zero files is a failing gate*, arriving inside the gate written to prevent it.
fn rendered() -> Vec<Intent> {
    let temp = tempfile::tempdir().expect("audit tempdir");
    let fixture =
        AuditFixture::new(temp.path(), SecretToken::new(SENTINEL.to_owned())).expect("fixture");
    let intents = Intent::all_for_audit(&fixture);
    assert_eq!(
        intents.len(),
        Intent::ALL.len(),
        "all_for_audit rendered {} of {} variants; every assertion in this file loops over this \
         set, so a short one passes by not looking",
        intents.len(),
        Intent::ALL.len()
    );
    assert!(
        !intents.is_empty(),
        "the write audit rendered zero variants"
    );
    intents
}

#[test]
fn every_rendered_subcommand_is_write_allowed() {
    let intents = rendered();
    let mut tokens = 0;
    for intent in &intents {
        let argv = intent.argv();
        tokens += argv.len();
        let subcommand = argv.first().and_then(|arg| arg.to_str()).unwrap_or("");
        assert!(
            WRITE_ALLOWED.contains(&subcommand),
            "{:?} rendered forbidden subcommand {subcommand:?}: {argv:?}",
            intent.kind()
        );
    }
    eprintln!(
        "git-write-audit: {} variants, {tokens} argv tokens, {} allowed subcommands",
        intents.len(),
        WRITE_ALLOWED.len()
    );
}

#[test]
fn no_rendered_argv_contains_a_forbidden_flag() {
    let intents = rendered();
    let mut scanned = 0;
    for intent in &intents {
        let argv = intent.argv();
        for token in &argv {
            scanned += 1;
            let token = token.to_string_lossy();
            assert!(
                !FLAG_FORBIDDEN.contains(&token.as_ref()),
                "{:?} rendered forbidden flag {token:?}: {argv:?}",
                intent.kind()
            );
        }
    }
    assert!(
        scanned > 0,
        "the flag denylist scanned zero argv tokens, so it asserts nothing"
    );
    eprintln!(
        "git-write-audit: {scanned} argv tokens checked against {} forbidden flags",
        FLAG_FORBIDDEN.len()
    );
}

#[test]
fn the_exhaustive_intent_list_has_two_renderable_variants() {
    let intents = rendered();
    assert_eq!(
        Intent::ALL.len(),
        2,
        "Intent::ALL changed; review every write-boundary assertion before accepting a new variant"
    );
    assert_eq!(
        intents.len(),
        Intent::ALL.len(),
        "all_for_audit did not render every Intent::ALL discriminant"
    );
}

/// **AC-P2-24-4, in the form that is expressible here.**
///
/// `Intent::Fetch` has no product caller in this plan — the in-session fetch is p2-24b's §24.7C —
/// so the criterion's *"under every scheduler retry path"* cannot be exercised by driving a
/// scheduler that does not exist. What **is** provable, and is the property the criterion rests
/// on, is that `argv()` is a **pure function of the variant**: rendering the same intent N times
/// yields byte-identical argv, so no retry can produce a token the first attempt did not.
///
/// Stated plainly because claiming otherwise would be a bar written past its defect: **this is
/// the property, not an exercised scheduler.**
#[test]
fn rendering_an_intent_repeatedly_is_byte_identical_and_never_grows_a_flag() {
    for intent in &rendered() {
        let first = intent.argv();
        for attempt in 1..8 {
            let again = intent.argv();
            assert_eq!(
                again,
                first,
                "{:?} rendered differently on attempt {attempt}; argv must be a pure function of \
                 the variant, or a retry can carry what the first attempt did not",
                intent.kind()
            );
            for token in &again {
                let token = token.to_string_lossy();
                assert!(
                    !FLAG_FORBIDDEN.contains(&token.as_ref()),
                    "{:?} grew forbidden flag {token:?} on attempt {attempt}",
                    intent.kind()
                );
            }
        }
    }
}
