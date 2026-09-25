//! The git **write** boundary (§24.1, restated for phase 4 by §47).
//!
//! **Every git invocation this product makes writes exactly what its intent's `effect()`
//! declares, whatever the user's config, environment or hooks say** (§47.1). Argv alone cannot
//! prove that: the phase-2 argv passed every argv assertion while a user's config made the
//! audited fetch delete a checked-out branch. So an invocation deletes and moves no ref, writes
//! no `FETCH_HEAD`, worktree file, index, config or reflog of an existing repository beyond its
//! declared effect, never shrinks an object store, and runs no program but git — and, for the
//! verifying read against an ssh remote, the user's own ssh transport.
//!
//! Lane 0's intent set is `{Clone, VerifyRead}`; `Intent::Fetch` is retired. `pull`, `worktree
//! remove`/`prune`, `clean`, `reset`, `restore`, `rm`, `gc`, `prune`, `merge`, `rebase`,
//! `update-ref` and `write-tree` never reach this boundary; `push`, `commit` and `remote add`
//! are declined outright (U1, U2).
//!
//! This module is the **sibling** of [`crate::git`], not a widening of it. That module stays
//! provably read-only: its `ALLOWED` list is byte-identical to phase 1's. `ls-remote` lives here,
//! where the transport pins, the deadline and the audit apply, and `core/tests/git_write_audit.rs`
//! is its audit.
//!
//! **The audit enumerates variants; it does not grep source text.** [`Intent`] is closed and
//! [`Intent::argv`] is total over it, so the subcommands, flags, pins and stdin each intent
//! renders can be asserted against what the product actually renders — and §47.9's differential
//! layer compares the repository itself, because config decides what a verb writes.

pub mod backend;
pub mod credential;
pub mod exec;
pub mod intent;

pub use backend::{MutatingGit, RunOutput, SystemMutatingGit};
pub use credential::CredentialChannel;
#[cfg(feature = "testkit")]
pub use exec::TransportFixture;
pub use exec::{write_base_args, FilterDrivers, WriteEnv, WriteExec};
#[cfg(feature = "testkit")]
pub use intent::AuditFixture;
pub use intent::{
    AdvertisedRef, DeclaredEffect, Intent, IntentKind, IntentRefusal, ObjectId, RemoteName,
    RemoteUrl, StdoutUse, VerifyStep, VerifyStepKind, GIT_INVOCATION_DEADLINE,
};
