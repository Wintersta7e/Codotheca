//! The work item, and the vocabulary its row is keyed by.
//!
//! **[`SyncTask`] is the work item and carries its key; [`SyncTaskKind`] is the schema's enum.**
//! That is `JobKind`-versus-the-schema's-`Job` again (`core/src/jobs/mod.rs:22-26`, `:149-154`):
//! the enum the wire declares names the *kind*, and the value the runner moves around carries the
//! kind plus the id it is about. §21.16's AC-P2-21-1 names `SyncTask::ALL`, which cannot exist —
//! a work item carrying a key has no array of values to walk — and the criterion is amended to
//! the kind vocabulary with the assertion unchanged, because it was always a slug comparison.
//!
//! **R31: neither enum is hand-written.** `SyncTaskKind` and `SyncTaskState` are declared by
//! `protocol/schema/protocol.json` and used generated. What this module adds is the inherent
//! `ALL` const (legal because both the type and the impl are in this crate) and [`kind_slug`],
//! whose agreement with the generated spelling is asserted by **reading the other side** through
//! serde rather than by restating it (R24).

use crate::protocol::{AccountId, ProjectId, SyncTaskKind};

/// One unit of forge work, with the id it is about.
///
/// `key` is nullable in `sync_task_state` so a process-wide task is representable. **Phase 2
/// declares none** — phase 3's dependency-advisory task is the one §21.6 names, and it needs no
/// migration when it arrives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncTask {
    AccountRepos {
        account_id: AccountId,
    },
    ProjectRemote {
        project_id: ProjectId,
    },
    /// **Keyed by account, not by project, which deviates from §21.3's table.** p2-22 shipped the
    /// repair as one bounded pass over every unmatched key
    /// (`core/src/identity/rename_repair.rs:50-97`), whose own bound is *one request per unmatched
    /// key, once*; re-keying it per project would mean a second copy of that selection and that
    /// loop against the module p2-22 owns, which is R1's shape. `key` is polymorphic by design, so
    /// an account id here is exactly as representable as a project id.
    RenameProbe {
        account_id: AccountId,
    },
}

impl SyncTask {
    #[must_use]
    pub fn kind(self) -> SyncTaskKind {
        match self {
            SyncTask::AccountRepos { .. } => SyncTaskKind::AccountRepos,
            SyncTask::ProjectRemote { .. } => SyncTaskKind::ProjectRemote,
            SyncTask::RenameProbe { .. } => SyncTaskKind::RenameProbe,
        }
    }

    /// The stored key.
    ///
    /// **It is polymorphic and that is load-bearing**: an account id for `account_repos` and for
    /// `rename_probe`, a *project* id for `project_remote`. Nothing may delete rows by key alone — see
    /// [`crate::sync::store::delete_account_tasks`], which filters by task as well and exists
    /// because a bare `key = <account id>` deletes another task's rows for whichever project
    /// happens to share that integer.
    #[must_use]
    pub fn key(self) -> i64 {
        match self {
            SyncTask::AccountRepos { account_id } | SyncTask::RenameProbe { account_id } => {
                account_id.0
            }
            SyncTask::ProjectRemote { project_id } => project_id.0,
        }
    }
}

// [p3] `SyncTaskKind::ALL` — the vocabulary `AC-P2-21-1` walks against `JobKind::ALL` — is
// **generated** now, from the schema's own variant list, so the inherent const that stood here is
// gone and its count with it. Every call site is unchanged.

/// The stored form, written into `sync_task_state.task` and constrained by that column's CHECK.
#[must_use]
pub fn kind_slug(kind: SyncTaskKind) -> &'static str {
    match kind {
        SyncTaskKind::AccountRepos => "account_repos",
        SyncTaskKind::ProjectRemote => "project_remote",
        SyncTaskKind::RenameProbe => "rename_probe",
    }
}
