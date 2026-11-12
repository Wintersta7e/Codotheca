//! `ExclusionGate` (GA1, GA23): the core's one exclusion list, asked before any read of a
//! repository and again before any write of a fact read from one.
//!
//! The gate is a pointer swap, never a lock held across work. Its mutex guards an
//! `Arc<ExclusionList>` and is held only to clone or replace that pointer; a check reads a
//! snapshot and decides with no lock held. `exclusions.add`/`.remove` swap inside the index
//! guard and before their commit, swapping the old list back if the commit fails, so a check
//! made under the index guard sees either the list before the change began or the list after it
//! committed.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use rusqlite::Transaction;

use super::list::{ExclusionList, LocationKey};

/// Where a check is made: one variant per row of §56.5's audit table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GateSite {
    /// The walk, before it descends into a directory.
    WalkDir,
    /// The hand-off, before it reads a discovered repository.
    Handoff,
    /// Presence, before it classifies a stored copy.
    Presence,
    /// The stored-location ownership probe.
    OwnershipProbe,
    /// The job scheduler, before it dispatches a job on a copy.
    Dispatch,
    /// A job's writer, inside its transaction.
    JobWrite,
    /// The visible-project enqueue.
    VisibleEnqueue,
    /// `projects.recheck`.
    Recheck,
    /// A requeue of a project's jobs.
    Requeue,
    /// The filesystem watch set's target selection.
    WatchSet,
    /// Remote sync's per-project enqueue.
    RemoteSync,
    /// The walk inside a WSL distro.
    Wsl,
    /// First run's reads of a chosen root.
    FirstRun,
    /// A README read for the project page.
    Readme,
    /// A relocation's read of the new path.
    Relocate,
    /// An uninstall's read of the copy it removes.
    Uninstall,
    /// The trust check's git read.
    Trust,
    /// A fetch's read of the repository.
    Fetch,
    /// A live session's `check-ignore` (GA33).
    Session,
}

impl GateSite {
    /// Every site, in §56.5's order.
    pub const ALL: [Self; 19] = [
        Self::WalkDir,
        Self::Handoff,
        Self::Presence,
        Self::OwnershipProbe,
        Self::Dispatch,
        Self::JobWrite,
        Self::VisibleEnqueue,
        Self::Recheck,
        Self::Requeue,
        Self::WatchSet,
        Self::RemoteSync,
        Self::Wsl,
        Self::FirstRun,
        Self::Readme,
        Self::Relocate,
        Self::Uninstall,
        Self::Trust,
        Self::Fetch,
        Self::Session,
    ];
}

/// A check's refusal: the target is covered, so the read does not start and the write does not
/// happen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Covered {
    /// The site that asked.
    pub site: GateSite,
}

/// The core's one exclusion list, swappable while readers hold snapshots of it.
#[derive(Debug)]
pub struct ExclusionGate {
    current: Mutex<Arc<ExclusionList>>,
    /// Moves on every swap, after the pointer is replaced, so a reader that caches a snapshot
    /// re-reads it only when this changed: one relaxed load per directory on the walk.
    epoch: AtomicU64,
    /// Every check's `(site, admitted)`, in the order made.
    #[cfg(feature = "testkit")]
    record: Mutex<Vec<(GateSite, bool)>>,
}

impl Default for ExclusionGate {
    /// A gate holding the built-ins alone.
    fn default() -> Self {
        Self::new(ExclusionList::default())
    }
}

impl ExclusionGate {
    /// A gate holding `list`.
    #[must_use]
    pub fn new(list: ExclusionList) -> Self {
        Self {
            current: Mutex::new(Arc::new(list)),
            epoch: AtomicU64::new(0),
            #[cfg(feature = "testkit")]
            record: Mutex::new(Vec::new()),
        }
    }

    /// The list in force now. A holder keeps the list it took for as long as it holds it.
    #[must_use]
    pub fn snapshot(&self) -> Arc<ExclusionList> {
        Arc::clone(&self.current.lock().unwrap_or_else(PoisonError::into_inner))
    }

    /// How many swaps have happened; moves after each swap's pointer is in place.
    #[must_use]
    pub fn epoch(&self) -> u64 {
        self.epoch.load(Ordering::Acquire)
    }

    /// The list in force, for a reader that applies the whole list itself rather than asking
    /// about one target: the in-distro walk (row 11), which cannot reach this gate. Recorded as an
    /// admitted check at `site`.
    #[must_use]
    pub fn snapshot_at(&self, site: GateSite) -> Arc<ExclusionList> {
        #[cfg(feature = "testkit")]
        self.record
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((site, true));
        #[cfg(not(feature = "testkit"))]
        let _ = site;
        self.snapshot()
    }

    /// Put `next` in force and answer the list it replaced, which a failed commit swaps back.
    #[must_use]
    pub fn swap(&self, next: Arc<ExclusionList>) -> Arc<ExclusionList> {
        let old = std::mem::replace(
            &mut *self.current.lock().unwrap_or_else(PoisonError::into_inner),
            next,
        );
        self.epoch.fetch_add(1, Ordering::AcqRel);
        old
    }

    /// Asked before the first filesystem touch or git spawn on `target`.
    ///
    /// # Errors
    /// `Covered` when the list in force covers `target`; the read must not start.
    pub fn check_read(&self, site: GateSite, target: &LocationKey) -> Result<(), Covered> {
        self.decide(site, target)
    }

    /// Asked inside the writer's transaction, before its first write of a fact read from
    /// `target`. The transaction is the witness that the writer holds the index guard, under
    /// which no add or remove is part-way through.
    ///
    /// # Errors
    /// `Covered` when the list in force covers `target`; the transaction writes nothing.
    pub fn check_write(
        &self,
        site: GateSite,
        _tx: &Transaction<'_>,
        target: &LocationKey,
    ) -> Result<(), Covered> {
        self.decide(site, target)
    }

    fn decide(&self, site: GateSite, target: &LocationKey) -> Result<(), Covered> {
        let covered = self.snapshot().covers_location(target);
        #[cfg(feature = "testkit")]
        self.record
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((site, !covered));
        if covered {
            Err(Covered { site })
        } else {
            Ok(())
        }
    }

    /// Every check made so far as `(site, admitted)`, which §56.5's audit reads.
    #[cfg(feature = "testkit")]
    #[must_use]
    pub fn checks(&self) -> Vec<(GateSite, bool)> {
        self.record
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}
