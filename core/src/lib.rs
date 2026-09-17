//! Codotheca core library.
//!
//! The binary in `src/main.rs` is a thin supervisor around this crate; everything the core does
//! lives here, so integration tests in `core/tests/` can link against it.
//!
//! Two invariants that are easy to violate and expensive to fix:
//!   * **stdout carries protocol frames and nothing else.** No `println!`, no dependency writing
//!     to stdout, no panic text. Diagnostics go to stderr, which the shell drains.
//!   * **Never write a zero where the value is unknown.** Absent and empty are different states
//!     throughout the data model.

#![forbid(unsafe_code)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

pub mod accounts;
pub mod art;
pub mod assembly;
pub mod cancel;
pub mod clock;
pub mod commands;
#[cfg(feature = "testkit")]
pub mod corpus;
pub mod derive;
pub mod detail;
pub mod firstrun;
pub mod freshness;
pub mod git;
pub mod gitw;
pub mod http;
pub mod identity;
pub mod index;
pub mod install;
pub mod jobs;
pub mod launch;
pub mod lifecycle;
pub mod mount;
pub mod paths;
pub mod projects;
pub mod proto;
pub mod protocol;
pub mod provider;
pub mod query;
pub mod readme;
pub mod remote;
pub mod removal;
pub mod scan;
pub mod session;
pub mod stats;
pub mod surfaces;
pub mod sync;
pub mod uninstall;
pub mod view;
pub mod wsl;

#[cfg(feature = "testkit")]
pub mod testing;

/// A trivially true constant that exists so an integration test can prove the library target
/// links before any real surface exists to call.
pub const PROTOCOL_VERSION_MAJOR_IS_POSITIVE: bool = protocol::PROTOCOL_VERSION > 0;

/// True when the crate was compiled with the `testkit` feature.
///
/// Production code may ask exactly this one question about the seams: a release build asserts
/// it is false, which is how a test double can never reach a user.
#[must_use]
pub const fn testkit_enabled() -> bool {
    cfg!(feature = "testkit")
}
