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

/// The one seam through which the product asks git to write bytes.
pub trait MutatingGit: Send + Sync + fmt::Debug {
    /// Run the intent, streaming the child's stderr a line at a time.
    ///
    /// # Errors
    /// Fails when the filter enumeration cannot complete, when the child cannot be spawned, when
    /// it exits non-zero, or when `cancel` fires.
    fn run(
        &self,
        intent: &Intent,
        cancel: &CancelToken,
        on_stderr: &mut dyn FnMut(&str),
    ) -> GitResult<()>;
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
    pub fn new(git: PathBuf, hooks_dir: PathBuf) -> SystemMutatingGit {
        SystemMutatingGit {
            exec: WriteExec::new(git),
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
    ) -> GitResult<()> {
        // [p2-24b] **The gap p2-24 named is closed by the type, and that is why no arm is left
        // here.** `Intent::Fetch` carried only a remote name, so a fetch would have run in
        // whatever the process's working directory happened to be — a write into a repository
        // nobody named — and this function refused it rather than guess. The variant now carries
        // its `work_dir` and `write_base_args` renders it as `-C`, so the state the refusal
        // guarded against cannot be built. A `match` that now has nothing to refuse would be a
        // guard asserting its own defaults.

        // The precondition read, and the reason it is a `?` rather than a default: a clone whose
        // filters cannot be enumerated is **refused, never run unfiltered** (§24.1b). A checkout
        // runs `.gitattributes` filters from the remote repository, so an empty list that was
        // never actually read would hand a remote's clean/smudge commands a live invocation.
        let filters = self.exec.filter_drivers()?;

        let env = WriteEnv {
            // **Left `None` on purpose, now that the intent answers for itself.** A clone's
            // destination does not exist yet, so there is no `-C` to render; a fetch carries its
            // own `work_dir` and `write_base_args` prefers it. Supplying a second one here would
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
