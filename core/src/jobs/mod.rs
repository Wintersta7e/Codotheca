//! The six observation jobs, their budgets and their scheduling vocabulary (§4.1, §4.1a).

pub mod classify;
pub mod j15_authorship;
pub mod j1_refstate;
pub mod j2_status;
pub mod j3_inventory;
pub mod j4_history;
pub mod j6_content;
pub mod queue;
pub mod scheduler;
pub mod state;
pub mod visible;

use std::time::Duration;

// R31/R21: `ProjectId` and `LocationId` are declared in `protocol/schema/protocol.json` and
// generated into `crate::protocol`. Plan 09's interface block names them under `core::index`,
// which never produced them; a hand-written pair here would be the R31 shape exactly.
use crate::protocol::{LocationId, ProjectId};

/// Which of §4.1's six jobs a work item is.
///
/// A deliberate subset of the schema's `Job` (**R34**): `j0` is plan 07's walk, reported on the
/// wire when it finishes but never queued by this scheduler, and `j5` is plan 10's art. Every
/// slug here is a schema `Job` variant, which the test module holds mechanically.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum JobKind {
    /// Ref state from file reads.
    J1Refstate,
    /// The committer census that gates everything expensive.
    J15Authorship,
    /// One timestamped worktree observation.
    J2Status,
    /// Chunked tracked inventory.
    J3Inventory,
    /// Chunked history and commit-days.
    J4History,
    /// Card art: one scene, one raster, one file. A CPU job off the git path (§4.1).
    J5Art,
    /// Manifest and README content.
    J6Content,
}

/// How many locations a job runs against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobScope {
    /// Ref state and worktree state are per-copy facts.
    PerLocation,
    /// History and tracked inventory are properties of the project (§5.1's primary location).
    PrimaryOnly,
}

impl JobKind {
    /// Every kind, so a test can walk the vocabulary without restating it.
    pub const ALL: [JobKind; 7] = [
        JobKind::J1Refstate,
        JobKind::J15Authorship,
        JobKind::J2Status,
        JobKind::J3Inventory,
        JobKind::J4History,
        JobKind::J5Art,
        JobKind::J6Content,
    ];

    /// The stored form. **R34**: this value is written into `project_job_state.job`, carried by
    /// `JobDone.job` on the wire, and constrained by the column's own CHECK — renaming one is a
    /// migration, and the three must agree.
    #[must_use]
    pub fn slug(self) -> &'static str {
        match self {
            JobKind::J1Refstate => "j1",
            JobKind::J15Authorship => "j1_5",
            JobKind::J2Status => "j2",
            JobKind::J3Inventory => "j3",
            JobKind::J4History => "j4",
            JobKind::J5Art => "j5",
            JobKind::J6Content => "j6",
        }
    }

    /// `slug`'s inverse. `None` for `j0` and `j5`, which are real schema jobs this scheduler
    /// does not queue, and for anything a newer build wrote.
    #[must_use]
    pub fn from_slug(s: &str) -> Option<JobKind> {
        JobKind::ALL.into_iter().find(|k| k.slug() == s)
    }

    /// The interval between yield checks, not a deadline to fail against. J4 has none:
    /// it is resumable and chunked, and a deadline would strand the biggest histories.
    #[must_use]
    pub fn slice_budget(self) -> Option<Duration> {
        match self {
            JobKind::J1Refstate => Some(Duration::from_millis(50)),
            JobKind::J2Status => Some(Duration::from_millis(500)),
            JobKind::J3Inventory => Some(Duration::from_millis(300)),
            // J1.5 is the gate everything waits on and J6 is byte-capped, not time-capped.
            // §4.1: a CPU job with no deadline table entry. It yields no slices: one scene,
            // one raster, one file.
            JobKind::J15Authorship | JobKind::J4History | JobKind::J5Art | JobKind::J6Content => {
                None
            }
        }
    }

    /// §6: worktree state has no fingerprint and never will. Everything else caches on one.
    #[must_use]
    pub fn is_cacheable(self) -> bool {
        !matches!(self, JobKind::J2Status)
    }

    /// Ruling 1: ref and worktree state are per-copy; history and inventory are the project's.
    #[must_use]
    pub fn scope(self) -> JobScope {
        match self {
            JobKind::J1Refstate | JobKind::J2Status => JobScope::PerLocation,
            JobKind::J15Authorship
            | JobKind::J3Inventory
            | JobKind::J4History
            | JobKind::J5Art
            | JobKind::J6Content => JobScope::PrimaryOnly,
        }
    }

    /// Every job holds one store slot for its whole run; J4 additionally takes a J4 slot.
    #[must_use]
    pub fn takes_j4_slot(self) -> bool {
        matches!(self, JobKind::J4History)
    }
}

/// Queue band. `Interactive` is smallest so `Ord` sorts the queue directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Priority {
    /// A visible tile asked for a fresh worktree reading. Jumps the queue.
    Interactive = 0,
    /// J1, which every other job's freshness gate reads.
    RefState = 1,
    /// §4.1a: the cheap authorship gate runs before the expensive jobs.
    Authorship = 2,
    /// Ordinary background work.
    Standard = 3,
    /// §4.1a: a repository that resolves to Reference drops to the bottom for J2 and J3.
    Reference = 4,
    /// Whatever may wait indefinitely.
    Deferred = 5,
}

/// One queued unit of work. **Not** the schema's `Job` — `protocol/schema/protocol.json`
/// declares `Job` as the job *identifier* enum (`j0 … j6`), which is what `JobDone.job` carries
/// and which `JobKind::slug` produces above. This is a scheduler work item: a kind plus the
/// place, the store, the priority and the backoff gate. R31 does not apply to it — the two are
/// different shapes in different modules, and neither can be substituted for the other by
/// accident. **Do not delete this on a name match.**
#[derive(Debug, Clone)]
pub struct Job {
    /// Which of the six.
    pub kind: JobKind,
    /// The project the result is filed against.
    pub project_id: ProjectId,
    /// The copy on disk this run reads.
    pub location_id: LocationId,
    /// `location.store_key` — the slot table's key.
    pub store_key: String,
    /// Resolved once, by whoever holds the path — it is `MountFacts.class` (**R4**). The queue
    /// never touches the filesystem.
    pub store_kind: crate::mount::StoreClass,
    /// Queue band.
    pub priority: Priority,
    /// Backoff gate, epoch seconds. `0` means runnable now.
    pub not_before: i64,
}

/// `project_job_state.state`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobState {
    /// Waiting for a slot.
    Queued,
    /// Holding a slot right now.
    Running,
    /// A completed reading — including J2's degraded one.
    Done,
    /// Three transient failures. Durable, not permanent; [`state::reset_for`] clears it.
    DeferredSlow,
    /// A hard failure that retrying cannot fix.
    Failed,
}

impl JobState {
    /// Every state, so a test can walk the vocabulary without restating it.
    pub const ALL: [JobState; 5] = [
        JobState::Queued,
        JobState::Running,
        JobState::Done,
        JobState::DeferredSlow,
        JobState::Failed,
    ];

    /// The stored form.
    ///
    /// **Deviation from the plan body, which spells `Done` as `"done"`.** The shipped DDL
    /// constrains the column to `('queued', 'running', 'ok', 'failed', 'deferred_slow')`, so
    /// `"done"` is rejected at insert time by SQLite — R26's shape exactly, a CHECK rejecting
    /// the value its own core emits. The column wins because it is what is already on disk;
    /// `every_job_state_slug_is_accepted_by_the_column` in `core/tests/jobs_state.rs` reads the
    /// other side rather than restating it.
    #[must_use]
    pub fn slug(self) -> &'static str {
        match self {
            JobState::Queued => "queued",
            JobState::Running => "running",
            JobState::Done => "ok",
            JobState::DeferredSlow => "deferred_slow",
            JobState::Failed => "failed",
        }
    }

    /// `slug`'s inverse. `None` for a state a newer build wrote.
    #[must_use]
    pub fn from_slug(s: &str) -> Option<JobState> {
        match s {
            "queued" => Some(JobState::Queued),
            "running" => Some(JobState::Running),
            "ok" => Some(JobState::Done),
            "deferred_slow" => Some(JobState::DeferredSlow),
            "failed" => Some(JobState::Failed),
            _ => None,
        }
    }
}

/// Why one job run could not produce a result.
#[derive(Debug, thiserror::Error)]
pub enum JobError {
    /// Git itself refused, timed out or is not there.
    #[error("git: {0}")]
    Git(#[from] crate::git::GitError),
    /// The database refused.
    #[error("index: {0}")]
    Index(#[from] crate::index::IndexError),
    /// A filesystem read failed outside git.
    #[error("io: {0}")]
    Io(String),
    /// §3.5: `index.lock` or an operation marker is present. Back off, do not store a torn read.
    #[error("the repository is busy")]
    RepositoryBusy,
    /// §3.5: the fingerprint moved between the before and after reads.
    #[error("the repository changed while it was being read")]
    TornRead,
    /// The slice budget was exceeded and the job has no chunking path.
    #[error("the slice budget was exceeded")]
    BudgetExceeded,
}

/// What one job run produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobOutcome {
    /// A complete reading.
    Done,
    /// A chunk boundary. The cursor and the coverage pair are persisted and the job requeues.
    Partial {
        /// Where to resume.
        cursor: String,
        /// Units finished so far.
        done: i64,
        /// Units in total, `None` while it is still unknown — never a zero.
        total: Option<i64>,
    },
    /// J2 only: tracked-only reading committed, `untracked_count` left NULL.
    Degraded,
    /// Worth retrying after a backoff.
    TransientFail {
        /// Diagnostic, never shown raw (§2.4).
        reason: String,
    },
    /// Retrying cannot fix this.
    HardFail {
        /// The §11.1 error kind.
        error_kind: &'static str,
        /// Diagnostic, never shown raw (§2.4).
        detail: String,
    },
}

/// The scanner's hand-off point. Implemented by `scheduler::JobRunner`; [`NullJobSink`] keeps
/// plan 07's own tests standing.
pub trait JobSink: Send + Sync + std::fmt::Debug {
    /// A location row has just been written and its jobs may be queued.
    fn on_location_indexed(
        &self,
        project: ProjectId,
        location: LocationId,
        store_key: &str,
        store_kind: crate::mount::StoreClass,
    );
    /// A visible tile, an opened page or a Peek asked for a current worktree reading (§6).
    ///
    /// **`needs_art` is decided by the caller, and that is R75's rule arriving on `Route::Detail`
    /// and `Route::Projects`.** Every call site reaches this method while `Assembly` holds the one
    /// `Arc<Mutex<Index>>` guard (`assembly/mod.rs:508,519` pass `index: &guard`), so a sink that
    /// takes that mutex again self-deadlocks — `std::sync::Mutex` is not reentrant — and the guard
    /// is then never released, wedging every later command behind it. The caller already has the
    /// connection; it answers the art question there and hands the answer over.
    fn on_visible(
        &self,
        project: ProjectId,
        location: LocationId,
        store_key: &str,
        store_kind: crate::mount::StoreClass,
        needs_art: bool,
    );
}

/// A sink that queues nothing, for a scan that runs without a scheduler behind it.
#[derive(Debug, Default)]
pub struct NullJobSink;

impl JobSink for NullJobSink {
    fn on_location_indexed(
        &self,
        _: ProjectId,
        _: LocationId,
        _: &str,
        _: crate::mount::StoreClass,
    ) {
    }
    fn on_visible(
        &self,
        _: ProjectId,
        _: LocationId,
        _: &str,
        _: crate::mount::StoreClass,
        _: bool,
    ) {
    }
}

/// Everything one job run needs besides the index: the git seam, the clock, and the scan run's
/// cancellation token.
///
/// Bundled because `run_one` would otherwise take six arguments, four of which are the same for
/// every job in a run.
#[derive(Clone)]
pub struct JobDeps {
    /// The git seam (§15.2).
    pub git: std::sync::Arc<dyn crate::git::GitBackend>,
    /// The clock seam (§15.2). Wall time only; the jobs measure nothing.
    pub clock: std::sync::Arc<dyn crate::clock::Clock>,
    /// The scan run's token. Cancellation propagates to the whole git process tree.
    pub cancel: crate::cancel::CancelToken,
}

impl std::fmt::Debug for JobDeps {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JobDeps").finish_non_exhaustive()
    }
}

/// Where one location's copy lives on disk, and whether git may trust it.
#[derive(Debug, Clone)]
struct LocationPlace {
    work_dir: std::path::PathBuf,
    common_dir: Option<std::path::PathBuf>,
    repo_kind: String,
    trusted: bool,
}

fn read_place(
    index: &std::sync::Mutex<crate::index::Index>,
    location: LocationId,
) -> Result<LocationPlace, JobError> {
    let guard = index
        .lock()
        .map_err(|_| JobError::Io("index lock is poisoned".to_owned()))?;
    let place = guard.read(|conn| {
        conn.query_row(
            "SELECT path_bytes, common_dir_bytes, repo_kind, trusted_at FROM location WHERE id = ?1",
            [location.0],
            |r| {
                Ok((
                    r.get::<_, Vec<u8>>(0)?,
                    r.get::<_, Option<Vec<u8>>>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Option<i64>>(3)?,
                ))
            },
        )
        .map_err(crate::index::IndexError::Sqlite)
    })?;
    drop(guard);
    let (path_bytes, common_dir_bytes, repo_kind, trusted_at) = place;
    Ok(LocationPlace {
        work_dir: crate::paths::path_from_bytes(&path_bytes),
        common_dir: common_dir_bytes
            .as_deref()
            .map(crate::paths::path_from_bytes),
        repo_kind,
        trusted: trusted_at.is_some(),
    })
}

impl LocationPlace {
    fn handle(&self, job: &Job) -> crate::git::RepoHandle {
        let store = crate::git::StoreKey::new(&job.store_key);
        let handle = if self.repo_kind == "bare" {
            crate::git::RepoHandle::bare(&self.work_dir, store, job.store_kind)
        } else {
            let git_dir = self
                .common_dir
                .clone()
                .unwrap_or_else(|| self.work_dir.join(".git"));
            crate::git::RepoHandle {
                work_dir: self.work_dir.clone(),
                git_dir: git_dir.clone(),
                common_dir: git_dir,
                store,
                store_class: job.store_kind,
                trusted: false,
            }
        };
        handle.with_trust(self.trusted)
    }
}

/// The `JobClass` a kind runs under, which is what the slot pool caps on.
fn class_of(kind: JobKind, priority: Priority) -> crate::git::JobClass {
    if priority == Priority::Interactive {
        crate::git::JobClass::Interactive
    } else if kind == JobKind::J4History {
        crate::git::JobClass::History
    } else {
        crate::git::JobClass::Background
    }
}

/// Run one job, and write what that job owns.
///
/// **R39 is resolved here, and the shape of this function is the resolution.** `Index` holds a
/// `rusqlite::Connection`, which is `Send` but **not `Sync`**, so `Arc<Index>` is not `Send` and
/// cannot cross to a worker thread. This takes an owning `Mutex<Index>` instead, and the
/// discipline that makes that safe is structural rather than stated: **the lock is taken to read
/// the inputs, released, git runs, and the lock is taken again to write.** No arm below holds it
/// across a git invocation, so a twenty-second J4 blocks no command.
pub fn run_one(
    index: &std::sync::Mutex<crate::index::Index>,
    deps: &JobDeps,
    job: &Job,
) -> Result<JobOutcome, JobError> {
    let place = read_place(index, job.location_id)?;
    let repo = place.handle(job);
    let ctx = crate::git::JobContext::new(
        class_of(job.kind, job.priority),
        &deps.cancel,
        job.kind.slice_budget(),
    );
    let git = deps.git.as_ref();
    let now = deps.clock.now_unix();

    match job.kind {
        // §4.1: J5 spawns no git process, so it takes no git slot and reads no repository —
        // its inputs are columns J3 already wrote.
        //
        // **R39**: `run_one` holds `&Mutex<Index>`, so the plan's literal
        // `run_j5(index, ...)` does not typecheck — `run_j5` takes the `&Index` the plan
        // declares, and the lock is taken here. It is held across the raster, which is roughly
        // 100 ms of CPU for one card; plan 21 owns assembly and is where a narrower split
        // belongs if that ever proves to matter.
        JobKind::J5Art => {
            let guard = index
                .lock()
                .map_err(|_| JobError::Io("index lock is poisoned".to_owned()))?;
            let outcome = crate::art::job::run_j5(&guard, job.project_id.0, now);
            drop(guard);
            outcome
        }
        JobKind::J1Refstate => {
            let state = j1_refstate::observe(git, &repo, &ctx)?;
            write(index, |tx| {
                j1_refstate::persist(tx, job.location_id, &state)
            })?;
            Ok(JobOutcome::Done)
        }
        JobKind::J15Authorship => {
            let identities = identity_set(index)?;
            let facts = j15_authorship::census(git, &repo, &identities, &ctx)?;
            write(index, |tx| {
                j15_authorship::persist(tx, job.project_id, &facts)
            })?;
            Ok(JobOutcome::Done)
        }
        JobKind::J2Status => {
            let status = j2_status::observe(git, &repo, &ctx)?;
            let degraded = j2_status::is_degraded(&status);
            write(index, |tx| j2_status::persist(tx, job.location_id, &status))?;
            Ok(if degraded {
                JobOutcome::Degraded
            } else {
                JobOutcome::Done
            })
        }
        JobKind::J3Inventory => {
            let facts = j3_inventory::observe(git, &repo, &ctx, &place.work_dir)?;
            write(index, |tx| {
                j3_inventory::commit_inventory(tx, job.project_id, job.location_id, &facts)
            })?;
            Ok(JobOutcome::Done)
        }
        JobKind::J4History => {
            let identities = identity_set(index)?;
            let (roots, facts) = j4_history::observe(git, &repo, &identities, &ctx)?;
            write(index, |tx| {
                j4_history::commit_history(tx, job.project_id, &roots, &facts)
            })?;
            Ok(JobOutcome::Done)
        }
        JobKind::J6Content => {
            let facts = j6_content::read_content(&place.work_dir, j6_content::J6_BYTE_CAP);
            write(index, |tx| {
                j6_content::persist(tx, job.project_id, &facts, now)
            })?;
            Ok(JobOutcome::Done)
        }
    }
}

/// Take the lock, read, release. Never called with a git invocation inside `f`.
fn read<T>(
    index: &std::sync::Mutex<crate::index::Index>,
    f: impl FnOnce(&rusqlite::Connection) -> Result<T, crate::index::IndexError>,
) -> Result<T, JobError> {
    let guard = index
        .lock()
        .map_err(|_| JobError::Io("index lock is poisoned".to_owned()))?;
    let out = guard.read(f);
    drop(guard);
    out.map_err(JobError::Index)
}

/// `load_identity_set` fails with the identity module's error and the job vocabulary carries the
/// index's. The only failure it can actually have is a SQLite one; the rest are mapped rather
/// than flattened so a surprise is diagnosable instead of silent.
fn identity_set(
    index: &std::sync::Mutex<crate::index::Index>,
) -> Result<crate::identity::user::IdentitySet, JobError> {
    read(index, |conn| {
        crate::identity::user::load_identity_set(conn).map_err(|e| match e {
            crate::identity::IdentityError::Sqlite(inner) => {
                crate::index::IndexError::Sqlite(inner)
            }
            crate::identity::IdentityError::Index(inner) => inner,
            other => crate::index::IndexError::Corrupt {
                detail: format!("{other:?}"),
            },
        })
    })
}

/// Take the lock, write in one transaction, release.
fn write<T>(
    index: &std::sync::Mutex<crate::index::Index>,
    f: impl FnOnce(&rusqlite::Transaction<'_>) -> Result<T, crate::index::IndexError>,
) -> Result<T, JobError> {
    let mut guard = index
        .lock()
        .map_err(|_| JobError::Io("index lock is poisoned".to_owned()))?;
    let out = guard.with_tx(f);
    drop(guard);
    out.map_err(JobError::Index)
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn slugs_round_trip_and_are_stable() {
        for k in JobKind::ALL {
            assert_eq!(JobKind::from_slug(k.slug()), Some(k));
        }
        // The slug is a stored value in project_job_state.job. Renaming one is a migration.
        assert_eq!(JobKind::J1Refstate.slug(), "j1");
        assert_eq!(JobKind::J15Authorship.slug(), "j1_5");
        assert_eq!(JobKind::J2Status.slug(), "j2");
        assert_eq!(JobKind::J3Inventory.slug(), "j3");
        assert_eq!(JobKind::J4History.slug(), "j4");
        assert_eq!(JobKind::J6Content.slug(), "j6");
    }

    #[test]
    fn budgets_are_the_measured_ones() {
        assert_eq!(
            JobKind::J1Refstate.slice_budget(),
            Some(Duration::from_millis(50))
        );
        assert_eq!(
            JobKind::J2Status.slice_budget(),
            Some(Duration::from_millis(500))
        );
        assert_eq!(
            JobKind::J3Inventory.slice_budget(),
            Some(Duration::from_millis(300))
        );
        // J4 has no deadline at all; it yields and persists a cursor instead.
        assert_eq!(JobKind::J4History.slice_budget(), None);
    }

    #[test]
    fn worktree_state_is_never_cacheable() {
        assert!(JobKind::J1Refstate.is_cacheable());
        assert!(JobKind::J4History.is_cacheable());
        assert!(!JobKind::J2Status.is_cacheable());
    }

    #[test]
    fn history_and_inventory_run_against_the_primary_only() {
        assert_eq!(JobKind::J1Refstate.scope(), JobScope::PerLocation);
        assert_eq!(JobKind::J2Status.scope(), JobScope::PerLocation);
        assert_eq!(JobKind::J3Inventory.scope(), JobScope::PrimaryOnly);
        assert_eq!(JobKind::J4History.scope(), JobScope::PrimaryOnly);
        assert_eq!(JobKind::J15Authorship.scope(), JobScope::PrimaryOnly);
        assert_eq!(JobKind::J6Content.scope(), JobScope::PrimaryOnly);
    }

    /// **R34**: the slug is stored in three places that must agree — `JobKind::slug()`, the
    /// `project_job_state.job` column, and the schema's `Job` variants. Without this, adding a
    /// job kind emits a wire value the generated enum cannot represent and it fails at the
    /// receiver. Reading the other side is what makes the mirror a mirror and not two constants.
    #[test]
    fn every_job_slug_is_a_schema_job_variant() {
        for k in JobKind::ALL {
            let json = format!("\"{}\"", k.slug());
            let parsed: Result<crate::protocol::Job, _> = serde_json::from_str(&json);
            assert!(
                parsed.is_ok(),
                "{} is not a variant of the schema's Job enum",
                k.slug()
            );
        }
    }

    #[test]
    fn job_states_round_trip() {
        for s in JobState::ALL {
            assert_eq!(JobState::from_slug(s.slug()), Some(s));
        }
    }
}
