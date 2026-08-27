//! Codotheca core.
//!
//! Runs as a child process of the shell, speaking length-prefixed JSON on stdin/stdout.
//! See the protocol specification §2 for the framing and §4 for the scanner.
//!
//! Two invariants that are easy to violate and expensive to fix:
//!   * **stdout carries protocol frames and nothing else.** No `println!`, no dependency
//!     writing to stdout, no panic text. Diagnostics go to stderr, which the shell drains.
//!   * **Never write a zero where the value is unknown.** Absent and empty are different
//!     states throughout the data model.

#![forbid(unsafe_code)]

use codotheca_core::protocol;

use std::io::Write as _;

fn main() -> std::process::ExitCode {
    // stderr is the only channel for diagnostics; the shell merges it into the rolling log.
    let mut err = std::io::stderr();
    let _ = writeln!(
        err,
        "codotheca-core {} protocol v{} starting",
        env!("CARGO_PKG_VERSION"),
        protocol::PROTOCOL_VERSION
    );

    // TODO(phase-1): handshake, then the command loop. Nothing is implemented yet;
    // this binary exists so the workspace, lints, CI and packaging are real from commit one.
    let _ = writeln!(err, "not implemented");
    std::process::ExitCode::SUCCESS
}
