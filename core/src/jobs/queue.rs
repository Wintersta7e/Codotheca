//! Job ordering and slot admission (§4.1a). Pure: no threads, no I/O, no clock of its own.

use std::collections::{BTreeMap, HashMap};

use super::{Job, JobKind, Priority};
use crate::mount::StoreClass;

/// §3.4's global cap on concurrent git processes: `min(16, cores)`.
#[must_use]
pub fn global_cap() -> usize {
    std::thread::available_parallelism()
        .map_or(4, std::num::NonZeroUsize::get)
        .min(16)
}

/// §3.4: 1 for network/HDD/FUSE/removable, 4 for everything else, fixed.
///
/// **R4**: the rule itself lives on plan 06's `StoreClass::per_store_cap()`, so this is only the
/// `u32` → `usize` adapter the slot table wants. Restating the match here is the
/// one-value-in-two-places defect the rulings exist to kill.
#[must_use]
pub fn store_cap_for(kind: StoreClass) -> usize {
    usize::try_from(kind.per_store_cap()).unwrap_or(1)
}

/// The cap applied to a store the caller never registered. `StoreClass::Unknown`'s value, so a
/// forgotten `set_store_cap` is not silently more permissive than any real classification.
const DEFAULT_STORE_CAP: usize = 4;

/// Who is holding what right now. Handed to [`JobQueue::pop_ready`], which takes a slot when it
/// admits a job; the caller returns it with [`SlotState::release`] when the job is finished.
#[derive(Debug)]
pub struct SlotState {
    global_cap: usize,
    j4_cap: usize,
    in_flight: usize,
    j4_in_flight: usize,
    /// `store_key -> (in_flight, cap)`
    stores: HashMap<String, (usize, usize)>,
    j4_stores: HashMap<String, usize>,
}

impl SlotState {
    /// A fresh table under `global_cap` concurrent jobs.
    #[must_use]
    pub fn new(global_cap: usize) -> SlotState {
        SlotState {
            global_cap: global_cap.max(1),
            // §4.1a: J4 <= 25% of global slots.
            j4_cap: (global_cap / 4).max(1),
            in_flight: 0,
            j4_in_flight: 0,
            stores: HashMap::new(),
            j4_stores: HashMap::new(),
        }
    }

    /// Register a store's ceiling. Resolved from `MountFacts.class`, never from the queue.
    pub fn set_store_cap(&mut self, store_key: &str, cap: usize) {
        let entry = self.stores.entry(store_key.to_owned()).or_insert((0, cap));
        entry.1 = cap.max(1);
    }

    fn can_take(&self, job: &Job) -> bool {
        if self.in_flight >= self.global_cap {
            return false;
        }
        let (used, cap) = self
            .stores
            .get(&job.store_key)
            .copied()
            .unwrap_or((0, DEFAULT_STORE_CAP));
        if used >= cap {
            return false;
        }
        if job.kind.takes_j4_slot() {
            if self.j4_in_flight >= self.j4_cap {
                return false;
            }
            // §4.1a: J4 <= 1 per store.
            if self.j4_stores.get(&job.store_key).copied().unwrap_or(0) >= 1 {
                return false;
            }
        }
        true
    }

    fn take(&mut self, job: &Job) {
        self.in_flight = self.in_flight.saturating_add(1);
        let entry = self
            .stores
            .entry(job.store_key.clone())
            .or_insert((0, DEFAULT_STORE_CAP));
        entry.0 = entry.0.saturating_add(1);
        if job.kind.takes_j4_slot() {
            self.j4_in_flight = self.j4_in_flight.saturating_add(1);
            *self.j4_stores.entry(job.store_key.clone()).or_insert(0) += 1;
        }
    }

    /// Give back the slot a finished job held.
    pub fn release(&mut self, job: &Job) {
        self.in_flight = self.in_flight.saturating_sub(1);
        if let Some(entry) = self.stores.get_mut(&job.store_key) {
            entry.0 = entry.0.saturating_sub(1);
        }
        if job.kind.takes_j4_slot() {
            self.j4_in_flight = self.j4_in_flight.saturating_sub(1);
            if let Some(n) = self.j4_stores.get_mut(&job.store_key) {
                *n = n.saturating_sub(1);
            }
        }
    }
}

/// Priority first, then insertion order — so a `BTreeMap` keyed on this *is* the queue.
type QueueKey = (Priority, u64);

/// The pending work, ordered by band and then FIFO within it.
#[derive(Debug, Default)]
pub struct JobQueue {
    seq: u64,
    ready: BTreeMap<QueueKey, Job>,
    index: HashMap<(JobKind, i64), QueueKey>,
}

impl JobQueue {
    /// An empty queue.
    #[must_use]
    pub fn new() -> JobQueue {
        JobQueue::default()
    }

    /// Returns false when an equal-or-better entry for the same `(kind, location)` is already
    /// queued. Re-pushing at a better priority moves the entry and keeps its original
    /// sequence number, so jumping the queue never also jumps the FIFO within a band.
    pub fn push(&mut self, job: Job) -> bool {
        let ident = (job.kind, job.location_id.0);
        if let Some(existing) = self.index.get(&ident).copied() {
            if existing.0 <= job.priority {
                return false;
            }
            self.ready.remove(&existing);
            let key = (job.priority, existing.1);
            self.index.insert(ident, key);
            self.ready.insert(key, job);
            return true;
        }
        self.seq = self.seq.saturating_add(1);
        let key = (job.priority, self.seq);
        self.index.insert(ident, key);
        self.ready.insert(key, job);
        true
    }

    /// The highest-priority job that is past its backoff and whose store has a free slot.
    ///
    /// A blocked store is skipped, never waited on: one slow store must not stall a fast one,
    /// which is the whole reason the per-store cap exists rather than a single global one.
    pub fn pop_ready(&mut self, now: i64, slots: &mut SlotState) -> Option<Job> {
        let chosen = self
            .ready
            .iter()
            .find(|(_, job)| job.not_before <= now && slots.can_take(job))
            .map(|(key, _)| *key)?;
        let job = self.ready.remove(&chosen)?;
        self.index.remove(&(job.kind, job.location_id.0));
        slots.take(&job);
        Some(job)
    }

    /// How many jobs are queued.
    #[must_use]
    pub fn len(&self) -> usize {
        self.ready.len()
    }

    /// True when nothing is queued.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.ready.is_empty()
    }
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
    use crate::jobs::{Job, JobKind, Priority};
    use crate::mount::StoreClass;
    use crate::protocol::{LocationId, ProjectId};

    fn job(kind: JobKind, loc: i64, store: &str, priority: Priority) -> Job {
        Job {
            kind,
            project_id: ProjectId(loc),
            location_id: LocationId(loc),
            store_key: store.to_owned(),
            store_kind: StoreClass::Local,
            priority,
            not_before: 0,
            // The queue's ordering does not read the origin; this fixture names the walk because
            // that is what queues most jobs.
            origin: crate::jobs::JobOrigin::Walk,
        }
    }

    #[test]
    fn a_visible_tile_request_jumps_queued_background_work() {
        let mut q = JobQueue::new();
        let mut slots = SlotState::new(8);
        q.push(job(JobKind::J3Inventory, 1, "c", Priority::Standard));
        q.push(job(JobKind::J4History, 2, "c", Priority::Deferred));
        q.push(job(JobKind::J2Status, 3, "c", Priority::Interactive));
        let first = q.pop_ready(0, &mut slots).unwrap();
        assert_eq!(first.location_id, LocationId(3));
    }

    #[test]
    fn authorship_runs_before_the_expensive_jobs() {
        // §4.1a: gating cost 0.8% of what it saved and took 61% of scan work off the
        // critical path. v2 had authorship as the lowest-priority job, which is backwards.
        assert!(Priority::Authorship < Priority::Standard);
        assert!(Priority::RefState < Priority::Authorship);
        assert!(Priority::Standard < Priority::Reference);
    }

    #[test]
    fn ties_break_fifo_so_ordering_is_walk_independent() {
        let mut q = JobQueue::new();
        let mut slots = SlotState::new(8);
        q.push(job(JobKind::J2Status, 10, "c", Priority::Standard));
        q.push(job(JobKind::J2Status, 11, "c", Priority::Standard));
        assert_eq!(
            q.pop_ready(0, &mut slots).unwrap().location_id,
            LocationId(10)
        );
        assert_eq!(
            q.pop_ready(0, &mut slots).unwrap().location_id,
            LocationId(11)
        );
    }

    #[test]
    fn a_slow_store_admits_one_job_and_does_not_block_a_fast_one() {
        let mut q = JobQueue::new();
        let mut slots = SlotState::new(8);
        slots.set_store_cap("net", store_cap_for(StoreClass::Network));
        slots.set_store_cap("ssd", store_cap_for(StoreClass::Local));
        q.push(job(JobKind::J2Status, 1, "net", Priority::Standard));
        q.push(job(JobKind::J2Status, 2, "net", Priority::Standard));
        q.push(job(JobKind::J2Status, 3, "ssd", Priority::Standard));

        let a = q.pop_ready(0, &mut slots).unwrap();
        assert_eq!(a.store_key, "net");
        // The second network job cannot run; the queue must skip it, not stall on it.
        let b = q.pop_ready(0, &mut slots).unwrap();
        assert_eq!(b.store_key, "ssd");
        assert!(q.pop_ready(0, &mut slots).is_none());
        slots.release(&a);
        assert_eq!(
            q.pop_ready(0, &mut slots).unwrap().location_id,
            LocationId(2)
        );
    }

    #[test]
    fn j4_is_capped_at_a_quarter_of_the_global_slots_and_one_per_store() {
        let mut q = JobQueue::new();
        let mut slots = SlotState::new(8); // J4 ceiling 2
        for i in 0..6 {
            q.push(job(
                JobKind::J4History,
                i,
                &format!("s{i}"),
                Priority::Deferred,
            ));
        }
        assert!(q.pop_ready(0, &mut slots).is_some());
        assert!(q.pop_ready(0, &mut slots).is_some());
        assert!(
            q.pop_ready(0, &mut slots).is_none(),
            "J4 <= 25% of global slots"
        );
    }

    #[test]
    fn backoff_holds_a_job_until_its_time() {
        let mut q = JobQueue::new();
        let mut slots = SlotState::new(8);
        let mut j = job(JobKind::J2Status, 1, "c", Priority::Standard);
        j.not_before = 500;
        q.push(j);
        assert!(q.pop_ready(499, &mut slots).is_none());
        assert!(q.pop_ready(500, &mut slots).is_some());
    }

    #[test]
    fn pushing_the_same_job_twice_coalesces_and_keeps_the_better_priority() {
        let mut q = JobQueue::new();
        let mut slots = SlotState::new(8);
        assert!(q.push(job(JobKind::J2Status, 1, "c", Priority::Reference)));
        assert!(q.push(job(JobKind::J2Status, 1, "c", Priority::Interactive)));
        assert!(!q.push(job(JobKind::J2Status, 1, "c", Priority::Deferred)));
        assert_eq!(q.len(), 1);
        assert_eq!(
            q.pop_ready(0, &mut slots).unwrap().priority,
            Priority::Interactive
        );
    }
}
