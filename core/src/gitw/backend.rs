//! The mutating git seam, and its production implementation.
//!
//! **Both land in the same change.** A trait whose only implementation is a fake is unlanded work
//! — R1, R35a, R40 and R46 are four recorded instances of the same defect, each of which
//! compiled, passed against the fake, and failed at assembly.

use std::fmt;
use std::path::PathBuf;

use crate::cancel::CancelToken;
use crate::git::GitResult;
use crate::gitw::credential::CredentialChannel;
use crate::gitw::exec::{WriteEnv, WriteExec};
use crate::gitw::intent::Intent;

/// What a write child produced beyond its exit.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RunOutput {
    /// The child's stdout — filled **only** when the intent's `stdout_use()` is `Parse` (the
    /// verifying read's URL and advertisement), and empty otherwise.
    pub stdout: Vec<u8>,
}

/// The one seam through which the product asks git to write bytes.
pub trait MutatingGit: Send + Sync + fmt::Debug {
    /// Run the intent, streaming the child's stderr a line at a time.
    ///
    /// # Errors
    /// Fails when the filter enumeration cannot complete, when the child cannot be spawned, when
    /// it exits non-zero, when its deadline elapses, or when `cancel` fires.
    fn run(
        &self,
        intent: &Intent,
        cancel: &CancelToken,
        on_stderr: &mut dyn FnMut(&str),
    ) -> GitResult<RunOutput>;
}

/// The real thing: it spawns git.
#[derive(Debug, Clone)]
pub struct SystemMutatingGit {
    exec: WriteExec,
    hooks_dir: PathBuf,
}

impl SystemMutatingGit {
    /// Point the write path at a git binary and the empty hooks directory.
    #[must_use]
    pub const fn new(git: PathBuf, hooks_dir: PathBuf) -> Self {
        Self {
            exec: WriteExec::new(git),
            hooks_dir,
        }
    }

    /// The production write path with the test's one permitted difference (§47.9 C).
    #[cfg(feature = "testkit")]
    #[must_use]
    pub const fn with_transport_fixture(
        git: PathBuf,
        hooks_dir: PathBuf,
        fixture: crate::gitw::exec::TransportFixture,
    ) -> Self {
        Self {
            exec: WriteExec::with_transport_fixture(git, fixture),
            hooks_dir,
        }
    }
}

impl MutatingGit for SystemMutatingGit {
    fn run(
        &self,
        intent: &Intent,
        cancel: &CancelToken,
        on_stderr: &mut dyn FnMut(&str),
    ) -> GitResult<RunOutput> {
        // [p2-24b] **The gap p2-24 named is closed by the type, and that is why no arm is left
        // here.** An intent that runs in a repository carries that repository — `VerifyRead`'s
        // `repo`, rendered as `-C` by `write_base_args` — so no intent can run in whatever the
        // process's working directory happens to be. A `match` that has nothing to refuse would
        // be a guard asserting its own defaults. [p4] `Intent::Fetch`, which this comment used
        // to describe, is retired (§47.2).

        // The precondition read, and the reason it is a `?` rather than a default: a clone whose
        // filters cannot be enumerated is **refused, never run unfiltered** (§24.1b). A checkout
        // runs `.gitattributes` filters from the remote repository, so an empty list that was
        // never actually read would hand a remote's clean/smudge commands a live invocation.
        let filters = self.exec.filter_drivers()?;

        let env = WriteEnv {
            // **Left `None` on purpose, now that the intent answers for itself.** A clone's
            // destination does not exist yet, so there is no `-C` to render; the verifying read
            // carries its own `repo` and `write_base_args` prefers it. Supplying a second one here would
            // be two places that decide which repository is written to.
            work_dir: None,
            hooks_dir: self.hooks_dir.clone(),
            // §24.1c: the default tier's clone surface is anonymous HTTPS in full, so the
            // `public`-tier token never reaches git. p2-24b's authenticated clone selects the
            // helper channel here instead.
            credential: CredentialChannel::anonymous(),
            filters,
        };
        self.exec.run(intent, &env, cancel, on_stderr)
    }
}
