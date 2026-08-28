//! Compiled only under `testkit`: these link `codotheca_core::testing`, which the feature
//! gates. Without the gate a bare `cargo test` fails to compile rather than skipping, and the
//! real gate — `npm test`, and CI — passes `--features testkit`. `app/test/toolchain.test.ts`
//! asserts that flag is still there, so it cannot be dropped silently.
#![cfg(feature = "testkit")]
// The linkage assertion below is deliberately on a constant: its whole purpose is to prove the
// library target links from an integration test, before any real surface exists to call.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::assertions_on_constants
)]

//! The library target exists and the `testkit` feature reaches an integration test.
//! Without both, nothing else in this plan can be tested at all.

#[test]
fn the_library_target_is_linkable_from_an_integration_test() {
    assert!(codotheca_core::PROTOCOL_VERSION_MAJOR_IS_POSITIVE);
}

#[test]
fn testkit_is_enabled_when_the_suite_runs() {
    assert!(
        codotheca_core::testkit_enabled(),
        "run the suite with --features testkit; the seams are gated on it"
    );
}
