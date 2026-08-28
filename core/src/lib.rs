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

pub mod git;
pub mod lifecycle;
pub mod proto;
pub mod protocol;
