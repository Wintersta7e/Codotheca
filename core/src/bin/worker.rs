//! §13 — the WSL worker: a Linux ELF that runs inside a distro and speaks the core's frames.
//!
//! It is shipped inside the Windows installer and copied into the distro on first use, because
//! running it from the Windows mount is the slow path §4.5 exists to avoid.
//!
//! stdout carries frames and nothing else. Every diagnostic here goes to stderr, which
//! `wsl.exe` forwards to the core's stderr drain.

use codotheca_core::clock::SystemClock;
use codotheca_core::git::{ensure_empty_hooks_dir, GitExec, GitSlots, SystemGit};
use codotheca_core::proto::transport::claim_stdout;
use codotheca_core::wsl::mounts::MountTable;
use codotheca_core::wsl::serve::{
    probe_git, serve, WorkerContext, EXIT_STDOUT_TAKEN, WORKER_VERSION,
};
use std::process::ExitCode;
use std::sync::Arc;

fn main() -> ExitCode {
    let mut argv = std::env::args().skip(1);
    if argv.next().as_deref() == Some("--version") {
        eprintln!("codotheca-worker {WORKER_VERSION}");
        return ExitCode::SUCCESS;
    }

    let Some(mut stdout) = claim_stdout() else {
        eprintln!("codotheca-worker: stdout is already claimed");
        return ExitCode::from(EXIT_STDOUT_TAKEN);
    };

    // Everything Codotheca writes inside the distro lives under the versioned directory it was
    // installed into, which is this executable's own parent.
    let base = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(std::path::Path::to_path_buf))
        .unwrap_or_else(std::env::temp_dir);
    let hooks = match ensure_empty_hooks_dir(&base) {
        Ok(dir) => dir,
        Err(err) => {
            eprintln!("codotheca-worker: cannot prepare an empty hooks dir: {err}");
            return ExitCode::FAILURE;
        }
    };

    let exec = Arc::new(GitExec::system(hooks));
    let slots = Arc::new(GitSlots::for_machine());
    let git = SystemGit::new(exec, slots, Arc::new(SystemClock::default()));
    let presence = probe_git(&git);

    let ctx = WorkerContext {
        // Set by WSL itself in every distro; empty only outside one, where the store key is
        // then honestly unscoped rather than attributed to a distro that was guessed.
        distro: std::env::var("WSL_DISTRO_NAME").unwrap_or_default(),
        git: Box::new(git),
        mounts: MountTable::read(),
        presence,
    };

    let mut stdin = std::io::stdin();
    match serve(&ctx, &mut stdin, &mut stdout, std::process::id()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("codotheca-worker: {err}");
            ExitCode::FAILURE
        }
    }
}
