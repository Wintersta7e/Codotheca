//! Codotheca core, the binary.
//!
//! stdout carries protocol frames and nothing else. Every diagnostic goes to stderr, which
//! the shell drains into its rolling log.

#![forbid(unsafe_code)]

use codotheca_core::lifecycle::{
    parse_args, CoreLock, LockError, OsParentProbe, EXIT_BAD_ARGS, EXIT_LOCK_HELD,
};
use codotheca_core::proto::dispatch::{run_loop, send_hello, RefusingHandler};
use codotheca_core::proto::pubsub::{Publisher, TOPIC_HIGH_WATER};
use codotheca_core::proto::transport::{claim_stdout, Transport, WRITER_CAPACITY};
use codotheca_core::proto::wire::Epoch;
use std::io::Write as _;
use std::process::ExitCode;

fn note(line: &str) {
    let _ = writeln!(std::io::stderr(), "{line}");
}

fn main() -> ExitCode {
    let args = match parse_args(std::env::args().skip(1)) {
        Ok(a) => a,
        Err(e) => {
            note(&format!("codotheca-core: {e}"));
            return ExitCode::from(EXIT_BAD_ARGS);
        }
    };

    let _lock = match CoreLock::acquire(&args.data_dir) {
        Ok(l) => l,
        Err(LockError::Held) => {
            note("codotheca-core: another core holds the advisory lock for this data directory");
            return ExitCode::from(EXIT_LOCK_HELD);
        }
        Err(e) => {
            note(&format!("codotheca-core: {e}"));
            return ExitCode::from(EXIT_BAD_ARGS);
        }
    };

    let Some(stdout) = claim_stdout() else {
        note("codotheca-core: stdout was already claimed");
        return ExitCode::FAILURE;
    };

    let transport = match Transport::start(std::io::stdin(), stdout, WRITER_CAPACITY) {
        Ok(t) => t,
        Err(e) => {
            note(&format!("codotheca-core: transport: {e}"));
            return ExitCode::FAILURE;
        }
    };

    let epoch = Epoch(args.epoch);
    if let Err(e) = send_hello(&transport.sink, epoch) {
        note(&format!("codotheca-core: hello: {e}"));
        return ExitCode::FAILURE;
    }

    let publisher = Publisher::new(transport.sink.clone(), epoch, TOPIC_HIGH_WATER);
    let mut handler = RefusingHandler;
    let parent = OsParentProbe::new(args.parent_pid);
    let exit = run_loop(transport, publisher, &mut handler, epoch, &parent);
    note(&format!("codotheca-core: exiting ({exit:?})"));
    ExitCode::SUCCESS
}
