//! Starting a target. Argv only, never a shell.
//!
//! Four rules this file exists to hold:
//!
//! - **Never a shell.** `Command::new(program).args(argv)` and nothing else — no `sh -c`, no
//!   `cmd /c`, no string joining. An integration test greps this crate's source to keep a later
//!   reader from adding one.
//! - **All three stdio handles are `null`.** An editor that inherited our stdout would write
//!   into the protocol frame stream, which §2.1 forbids absolutely.
//! - **A non-waiting child is detached** so it outlives the core: `process_group(0)` on Unix,
//!   `DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP` on Windows. Both APIs are safe; the crate
//!   forbids `unsafe`, so `pre_exec` is neither available nor needed.
//! - **Nothing here ever kills anything.** §17: phase 1 has no destructive operation, and §7.8
//!   says Stop "writes nothing to disk". `session.stop` closes the ledger, never the editor.

use std::process::{Command, Stdio};

use crate::launch::argv::Invocation;
use crate::launch::LaunchError;

#[derive(Debug)]
pub struct Spawned {
    pub pid: u32,
    /// `Some` only in wait mode; joining it yields the child's exit code (§9, mechanism 1).
    pub waiter: Option<std::thread::JoinHandle<Option<i32>>>,
}

pub trait Spawner: Send + Sync + std::fmt::Debug {
    fn spawn(&self, inv: &Invocation, exec_display: &str) -> Result<Spawned, LaunchError>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct OsSpawner;

impl Spawner for OsSpawner {
    fn spawn(&self, inv: &Invocation, exec_display: &str) -> Result<Spawned, LaunchError> {
        let mut command = Command::new(&inv.program);
        command
            .args(&inv.argv)
            // Never inherited: an editor writing to our stdout corrupts the frame stream (§2.1).
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if let Some(cwd) = &inv.cwd {
            command.current_dir(cwd);
        }
        for (key, value) in &inv.env {
            command.env(key, value);
        }

        if !inv.wait {
            #[cfg(unix)]
            {
                use std::os::unix::process::CommandExt as _;
                command.process_group(0);
            }
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt as _;
                const DETACHED_PROCESS: u32 = 0x0000_0008;
                const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
                command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
            }
        }

        let mut child = command.spawn().map_err(|err| LaunchError::Spawn {
            exec_display: exec_display.to_owned(),
            detail: err.to_string(),
        })?;
        let pid = child.id();
        let waiter = inv
            .wait
            .then(|| std::thread::spawn(move || child.wait().ok().and_then(|s| s.code())));
        Ok(Spawned { pid, waiter })
    }
}

#[cfg(feature = "testkit")]
#[derive(Debug, Default)]
pub struct RecordingSpawner {
    calls: std::sync::Mutex<Vec<Invocation>>,
    fail: std::sync::Mutex<Option<String>>,
    exit_after_ms: std::sync::Mutex<Option<u64>>,
}

#[cfg(feature = "testkit")]
impl RecordingSpawner {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn calls(&self) -> Vec<Invocation> {
        self.calls.lock().map_or_else(|_| Vec::new(), |c| c.clone())
    }

    pub fn fail_next(&self, detail: &str) {
        if let Ok(mut slot) = self.fail.lock() {
            *slot = Some(detail.to_owned());
        }
    }

    /// A wait-mode child that exits after `ms`, so §9's `process_exit` closure is testable.
    pub fn exit_after(&self, ms: u64) {
        if let Ok(mut slot) = self.exit_after_ms.lock() {
            *slot = Some(ms);
        }
    }
}

#[cfg(feature = "testkit")]
impl Spawner for RecordingSpawner {
    fn spawn(&self, inv: &Invocation, exec_display: &str) -> Result<Spawned, LaunchError> {
        if let Ok(mut slot) = self.calls.lock() {
            slot.push(inv.clone());
        }
        if let Some(detail) = self.fail.lock().ok().and_then(|mut s| s.take()) {
            return Err(LaunchError::Spawn {
                exec_display: exec_display.to_owned(),
                detail,
            });
        }
        let delay = self.exit_after_ms.lock().ok().and_then(|s| *s);
        let waiter = inv.wait.then(|| {
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(delay.unwrap_or(0)));
                Some(0)
            })
        });
        Ok(Spawned { pid: 1, waiter })
    }
}
