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
/// `key` is nullable in `sync_task_state` so a process-wide task is representable. **[p3] §32's
/// advisory sweep is the one that takes that seam** — the task §21.6 names, now arrived, and it
/// needed no migration for the column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncTask {
    /// The scheduled listing of an account's repositories, one page per step.
    AccountRepos {
        /// The account whose repositories are listed.
        account_id: AccountId,
    },
    /// The on-demand read of one repository's remote facts and its Actions runs.
    ProjectRemote {
        /// The project whose forge binding is read.
        project_id: ProjectId,
    },
    /// **Keyed by account, not by project, which deviates from §21.3's table.** p2-22 shipped the
    /// repair as one bounded pass over every unmatched key
    /// (`core/src/identity/rename_repair.rs:50-97`), whose own bound is *one request per unmatched
    /// key, once*; re-keying it per project would mean a second copy of that selection and that
    /// loop against the module p2-22 owns, which is R1's shape. `key` is polymorphic by design, so
    /// an account id here is exactly as representable as a project id.
    RenameProbe {
        /// The account whose token the repair pass resolves unmatched keys with.
        account_id: AccountId,
    },
    /// **A unit variant, because the task is process-wide and has no id it is about.**
    ///
    /// [p3] §32.2's sweep: one row, `key IS NULL`, occupying `sync_task_global`. It reads the
    /// forge's global advisory endpoint unauthenticated and writes only library-wide facts, so
    /// there is no account and no project to key it by — and a sentinel id would make it look
    /// like one account's work in every status surface that renders the key.
    Advisories,
}

impl SyncTask {
    /// The schema's kind for this work item, dropping the id it is about.
    #[must_use]
    pub const fn kind(self) -> SyncTaskKind {
        match self {
            Self::AccountRepos { .. } => SyncTaskKind::AccountRepos,
            Self::ProjectRemote { .. } => SyncTaskKind::ProjectRemote,
            Self::RenameProbe { .. } => SyncTaskKind::RenameProbe,
            Self::Advisories => SyncTaskKind::Advisories,
        }
    }

    /// The stored key, **or `None` for a process-wide task**.
    ///
    /// **It is polymorphic and that is load-bearing**: an account id for `account_repos` and for
    /// `rename_probe`, a *project* id for `project_remote`. Nothing may delete rows by key alone — see
    /// [`crate::sync::store::delete_account_tasks`], which filters by task as well and exists
    /// because a bare `key = <account id>` deletes another task's rows for whichever project
    /// happens to share that integer.
    ///
    /// **[p3] It returns an `Option` and never a sentinel.** The column has been nullable since
    /// `0011_sync.sql` and `SyncTaskStateRow.key`, `put` and `load` all took an `Option` already;
    /// this total signature was the one place that could not say *no key*, and every caller
    /// wrapped it in `Some`. `SyncTaskStarted.key` and `SyncTaskSettled.key` are `i64?` on the
    /// wire already, so widening it moves no schema.
    #[must_use]
    pub const fn key(self) -> Option<i64> {
        match self {
            Self::AccountRepos { account_id } | Self::RenameProbe { account_id } => {
                Some(account_id.0)
            }
            Self::ProjectRemote { project_id } => Some(project_id.0),
            Self::Advisories => None,
        }
    }
}

// [p3] `SyncTaskKind::ALL` — the vocabulary `AC-P2-21-1` walks against `JobKind::ALL` — is
// **generated** now, from the schema's own variant list, so the inherent const that stood here is
// gone and its count with it. Every call site is unchanged.
//
// Two lanes met here: one widened the hand-written const to four variants for `Advisories`, the
// other deleted it because the generator emits the list. R31 settles it — a type the schema
// declares must not also be hand-written — and the generated list carries `Advisories` anyway,
// because that variant is in the schema. Taking the widened const instead would have restored a
// fourth count for a human to maintain.

/// The stored form, written into `sync_task_state.task` and constrained by that column's CHECK.
#[must_use]
pub const fn kind_slug(kind: SyncTaskKind) -> &'static str {
    match kind {
        SyncTaskKind::AccountRepos => "account_repos",
        SyncTaskKind::ProjectRemote => "project_remote",
        SyncTaskKind::RenameProbe => "rename_probe",
        SyncTaskKind::Advisories => "advisories",
    }
}
