//! A recording stand-in for the **write** git seam.
//!
//! §24.2a already ships `codotheca-recording-git`, a real binary the write audit spawns so it can
//! read the argv and environment the child actually received. This is the other half: a
//! `MutatingGit` that never spawns anything, for the tests that care about **what the run does
//! around the clone** — the rename, the commit order, the queue's ordering — rather than about
//! the argv.
//!
//! It is a fake beside a real implementation, never instead of one: `SystemMutatingGit`
//! (`core/src/gitw/backend.rs:49`) is the production impl and lands in the same crate.

use std::path::PathBuf;
use std::sync::Mutex;

use crate::cancel::CancelToken;
use crate::git::{GitError, GitResult};
use crate::gitw::backend::{MutatingGit, RunOutput};
use crate::gitw::intent::Intent;

/// What the fake should do when asked to clone.
#[derive(Debug, Clone)]
pub enum CloneBehaviour {
    /// Create the destination directory with a `.git` inside it, as a real clone would, and
    /// report success.
    Succeed,
    /// Create the directory, then fail — the shape of a clone killed mid-transfer, which leaves
    /// a partial tree in staging.
    PartialThenFail,
    /// Fail before writing anything.
    FailImmediately,
}

/// A `MutatingGit` that records intents and writes a plausible tree instead of spawning git.
#[derive(Debug)]
pub struct FakeMutatingGit {
    behaviour: Mutex<CloneBehaviour>,
    calls: Mutex<Vec<Intent>>,
    /// Set by the test to run between the clone returning and the caller's next step, which is
    /// how the crash-between-clone-and-rename case is reached without a real process.
    destinations: Mutex<Vec<PathBuf>>,
}

impl FakeMutatingGit {
    /// A fake that answers every clone with `behaviour`, with nothing recorded yet.
    #[must_use]
    pub const fn new(behaviour: CloneBehaviour) -> Self {
        Self {
            behaviour: Mutex::new(behaviour),
            calls: Mutex::new(Vec::new()),
            destinations: Mutex::new(Vec::new()),
        }
    }

    /// Every intent this seam was asked to run, in order.
    #[must_use]
    pub fn calls(&self) -> Vec<Intent> {
        self.calls
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Every destination a clone was pointed at, in order.
    #[must_use]
    pub fn destinations(&self) -> Vec<PathBuf> {
        self.destinations
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

impl MutatingGit for FakeMutatingGit {
    fn run(
        &self,
        intent: &Intent,
        cancel: &CancelToken,
        on_stderr: &mut dyn FnMut(&str),
    ) -> GitResult<RunOutput> {
        self.calls
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(intent.clone());

        let Intent::Clone { dest, .. } = intent else {
            // Only a clone is scripted here; the verifying read's tests drive the real backend
            // or `FixtureRemoteVerifier`, never an empty answer from this fake.
            return Ok(RunOutput::default());
        };
        self.destinations
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(dest.clone());

        let behaviour = self
            .behaviour
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();

        // A real clone writes progress to stderr; the stage machine parses that stream, so the
        // fake emits the same shape rather than nothing.
        on_stderr("Cloning into 'staging'...");
        match behaviour {
            CloneBehaviour::FailImmediately => {
                return Err(GitError::Cancelled);
            }
            CloneBehaviour::PartialThenFail => {
                let _ = std::fs::create_dir_all(dest.join("objects"));
                // The partial tree stays where it is: §24.3c's sweep, not this seam, decides
                // whether it can be warranted. Cancellation and an ordinary mid-transfer death
                // look identical from here, which is why the token is only read, not branched on.
                let _ = cancel.is_cancelled();
                return Err(GitError::Cancelled);
            }
            CloneBehaviour::Succeed => {
                let _ = std::fs::create_dir_all(dest.join(".git"));
                let _ = std::fs::write(dest.join(".git").join("HEAD"), b"ref: refs/heads/main\n");
                on_stderr("Receiving objects: 100% (3/3), done.");
            }
        }
        Ok(RunOutput::default())
    }
}
