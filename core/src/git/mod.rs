//! Git access. Every invocation is config-neutralised, argv only, never a shell (§3.2).
//!
//! **Every invocation in this module is read-only.** Phase 1 has no destructive operation at
//! all — no delete, clean, push, checkout or reset (§17) — and `core/tests/git_readonly.rs`
//! enforces that by auditing the subcommand literals in these files. Adding a writing
//! subcommand is a spec change, not a code change.
//!
//! `GIT_FLOOR` lives in [`version`] and the shell mirrors it, which a test pins.

mod backend;
mod error;
mod exec;
mod facts;
mod history;
mod ignore;
mod inventory;
mod invocation;
mod observe;
mod refstate;
mod repo;
mod slots;
mod status;
pub mod version;

pub use backend::{require_floor, GitBackend, JobContext, SystemGit};
pub use error::{classify, classify_spawn, BusyMarker, GitError, GitResult};
pub use exec::{GitExec, GitOutput, RunLimits};
pub use facts::{repo_facts, RepoFacts};
pub use history::{
    authorship, commit_subjects, local_day, parse_tz_offset_min, root_commits, Authorship,
    CommitSubject, CommitterTally, RootCommit,
};
pub use ignore::{check_ignore, CHECK_IGNORE_BATCH};
pub use inventory::{
    parse_ls_files_z, path_extension, submodule_gitlinks, tracked_inventory, IndexEntry,
    TrackedInventory,
};
pub use invocation::{base_args, ensure_empty_hooks_dir, neutralise_env, EMPTY_HOOKS_DIR_NAME};
pub use observe::{busy_marker, defer_while_locked, observe_stable, Backoff, Observation};
pub use refstate::{
    divergence, observation_fingerprint, read_ref_state, ref_fingerprint, Divergence,
    InterruptedOp, ObservationFingerprint, RefFingerprint, RefState, UpstreamRef,
};
pub use repo::{RepoHandle, StoreKey};
pub use slots::{GitSlots, JobClass, SlotGuard};
pub use status::{
    parse_status_v2, worktree_status, StatusCounts, StatusOptions, UntrackedMode, WorktreeStatus,
};
pub use version::{meets_floor, parse_version, GitVersion, GIT_FLOOR};
