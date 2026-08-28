//! Concurrency caps (§3.4).
//!
//! Global cap `min(16, cores)`; per-store cap 1 for network, HDD, FUSE and removable stores and
//! 4 for everything else, overridable from settings; `JobClass::History` (J4) additionally
//! bounded to a quarter of the global slots and one per store, because J4 runs git and would
//! otherwise crowd out the jobs a user is waiting on.

use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use crate::cancel::CancelToken;
use crate::mount::StoreClass;

use super::error::{GitError, GitResult};
use super::repo::StoreKey;

/// What kind of work wants a slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobClass {
    /// A visible tile's request. Served ahead of anything queued (§4.1a).
    Interactive,
    /// Ordinary scan work: J1, J1.5, J2, J3.
    Background,
    /// J4, the history walk.
    History,
}

#[derive(Debug, Default)]
struct SlotState {
    global_used: usize,
    history_used: usize,
    per_store: HashMap<StoreKey, usize>,
    per_store_history: HashMap<StoreKey, usize>,
    store_overrides: HashMap<StoreKey, usize>,
    interactive_waiting: usize,
}

/// The git process pool.
#[derive(Debug)]
pub struct GitSlots {
    state: Mutex<SlotState>,
    wake: Condvar,
    global_limit: usize,
    history_limit: usize,
}

impl GitSlots {
    /// A pool with an explicit global cap. `history_limit` is a quarter of it, at least 1.
    #[must_use]
    pub fn new(global_limit: usize) -> Self {
        let global_limit = global_limit.max(1);
        Self {
            state: Mutex::new(SlotState::default()),
            wake: Condvar::new(),
            global_limit,
            history_limit: (global_limit / 4).max(1),
        }
    }

    /// The pool this machine gets: `min(16, cores)`.
    #[must_use]
    pub fn for_machine() -> Self {
        let cores = std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get);
        Self::new(cores.min(16))
    }

    /// The global cap.
    #[must_use]
    pub fn global_limit(&self) -> usize {
        self.global_limit
    }

    /// The J4 ceiling.
    #[must_use]
    pub fn history_limit(&self) -> usize {
        self.history_limit
    }

    /// Replace one store's cap from settings (§3.4, "Settings override").
    pub fn set_store_limit(&self, store: &StoreKey, limit: usize) {
        if let Ok(mut s) = self.state.lock() {
            s.store_overrides.insert(store.clone(), limit.max(1));
        }
        self.wake.notify_all();
    }

    // R4: §3.4's per-store cap is stated once, in `StoreClass::per_store_cap()` (plan 06).
    // Matching on the class here would be the same rule written a second time.
    fn store_limit(state: &SlotState, store: &StoreKey, class: StoreClass) -> usize {
        state
            .store_overrides
            .get(store)
            .copied()
            .unwrap_or_else(|| usize::try_from(class.per_store_cap()).unwrap_or(1))
    }

    fn admissible(
        &self,
        state: &SlotState,
        store: &StoreKey,
        class: StoreClass,
        job: JobClass,
    ) -> bool {
        if state.global_used >= self.global_limit {
            return false;
        }
        if job != JobClass::Interactive && state.interactive_waiting > 0 {
            return false;
        }
        let used = state.per_store.get(store).copied().unwrap_or(0);
        if used >= Self::store_limit(state, store, class) {
            return false;
        }
        if job == JobClass::History {
            if state.history_used >= self.history_limit {
                return false;
            }
            if state.per_store_history.get(store).copied().unwrap_or(0) >= 1 {
                return false;
            }
        }
        true
    }

    /// Wait for a slot, or return [`GitError::Cancelled`] if the token fires first.
    pub fn acquire(
        self: &Arc<Self>,
        store: &StoreKey,
        class: StoreClass,
        job: JobClass,
        cancel: &CancelToken,
    ) -> GitResult<SlotGuard> {
        let mut state = self.state.lock().map_err(|_| GitError::Internal {
            detail: "git slot mutex poisoned".to_owned(),
        })?;
        if job == JobClass::Interactive {
            state.interactive_waiting += 1;
        }
        let outcome = loop {
            if cancel.is_cancelled() {
                break Err(GitError::Cancelled);
            }
            let ok = if job == JobClass::Interactive {
                // Do not let this waiter's own registration block itself.
                state.interactive_waiting -= 1;
                let ok = self.admissible(&state, store, class, job);
                state.interactive_waiting += 1;
                ok
            } else {
                self.admissible(&state, store, class, job)
            };
            if ok {
                state.global_used += 1;
                *state.per_store.entry(store.clone()).or_insert(0) += 1;
                if job == JobClass::History {
                    state.history_used += 1;
                    *state.per_store_history.entry(store.clone()).or_insert(0) += 1;
                }
                break Ok(());
            }
            let (next, _) = self
                .wake
                .wait_timeout(state, Duration::from_millis(10))
                .map_err(|_| GitError::Internal {
                    detail: "git slot mutex poisoned".to_owned(),
                })?;
            state = next;
        };
        if job == JobClass::Interactive {
            state.interactive_waiting -= 1;
        }
        drop(state);
        self.wake.notify_all();
        outcome?;
        Ok(SlotGuard {
            slots: Arc::clone(self),
            store: store.clone(),
            job,
        })
    }

    fn release(&self, store: &StoreKey, job: JobClass) {
        if let Ok(mut state) = self.state.lock() {
            state.global_used = state.global_used.saturating_sub(1);
            if let Some(n) = state.per_store.get_mut(store) {
                *n = n.saturating_sub(1);
            }
            if job == JobClass::History {
                state.history_used = state.history_used.saturating_sub(1);
                if let Some(n) = state.per_store_history.get_mut(store) {
                    *n = n.saturating_sub(1);
                }
            }
        }
        self.wake.notify_all();
    }
}

/// Holds one git slot until dropped.
#[derive(Debug)]
pub struct SlotGuard {
    slots: Arc<GitSlots>,
    store: StoreKey,
    job: JobClass,
}

impl Drop for SlotGuard {
    fn drop(&mut self) {
        self.slots.release(&self.store, self.job);
    }
}
