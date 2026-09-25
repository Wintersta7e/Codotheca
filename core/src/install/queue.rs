//! §24.3e's install queue: **one at a time, FIFO, and this plan's own** (R52).
//!
//! §24.3e defers the queue to §21; §21.12 already answered in the other direction, because a
//! clone is a user-initiated local git write with a real store and a real disk — its cost is the
//! per-store cap, not a rate budget. So there is **no `JobKind` variant, no `project_job_state`
//! row and no `SyncTask`** here, and R34's three-place slug agreement is undisturbed.
//!
//! **The queue holds no path.** An `InstallRequest` carries the `RootId` and the project's stored
//! `seed_basename` inside its `InstallDestination`; the real path is re-derived when the run
//! begins. That is what lets a queued request outlive the command that made it without holding a
//! path the renderer could never have originated.

use std::collections::{BTreeMap, VecDeque};
use std::sync::Mutex;

use rusqlite::Transaction;

use crate::cancel::CancelToken;
use crate::protocol::{InstallDestination, InstallRunId, ProjectId, RootId};

/// One accepted install, composed by Task 10 and never by the renderer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallRequest {
    /// The project being cloned.
    pub project: ProjectId,
    /// The root it was named against.
    pub root: RootId,
    /// `rootId` + `seedBasename` + the lossy display form. **No path.**
    pub destination: InstallDestination,
}

/// What the queue is doing, for a caller that must not start a second clone.
#[derive(Debug, Default)]
struct State {
    in_flight: Option<InstallRunId>,
    waiting: VecDeque<(InstallRunId, InstallRequest)>,
    /// The token that stops each running clone. §24.3c cancels by **killing the process group**,
    /// which `WriteExec` does when this fires — so cancelling is reaching the right token, not
    /// finding the right pid.
    tokens: BTreeMap<i64, CancelToken>,
}

/// A strictly serial, first-in-first-out queue of installs.
///
/// **Capacity one in flight.** Two `install.start` calls run one after the other, in the order
/// they arrived — never concurrently, because two clones under one root would contend for the
/// same staging directory and the second would meet a destination the first is still writing.
#[derive(Debug, Default)]
pub struct InstallQueue {
    state: Mutex<State>,
}

impl InstallQueue {
    /// An empty queue: nothing in flight, nothing waiting.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a durable run row and take a place in the queue.
    ///
    /// **The row is written before the first byte, which is what the staging warrant rests on**
    /// (§24.3c): a directory with no row behind it cannot be warranted and is left in place for
    /// the sweep to report. The row's state is `running` from this moment — it means *begun and
    /// not ended*, and §24.4's first stage, `plans`, is entered on `install.start` accepted and
    /// reported until the child spawns. A queued run is therefore accepted and honest about not
    /// yet cloning, rather than claiming a stage it has not reached.
    ///
    /// # Errors
    /// Fails when the run row cannot be written.
    pub fn push(
        &self,
        tx: &Transaction<'_>,
        request: InstallRequest,
        staging_bytes: &[u8],
        destination_bytes: &[u8],
        now: i64,
    ) -> rusqlite::Result<InstallRunId> {
        let id = tx.query_row(
            "INSERT INTO install_run
                (project_id, root_id, staging_bytes, destination_bytes, state, stage, started_at)
             VALUES (?1, ?2, ?3, ?4, 'running', 'plans', ?5)
             RETURNING id",
            rusqlite::params![
                request.project.0,
                request.root.0,
                staging_bytes,
                destination_bytes,
                now
            ],
            |row| row.get::<_, i64>(0),
        )?;
        let run = InstallRunId(id);

        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.in_flight.is_none() {
            state.in_flight = Some(run);
        } else {
            state.waiting.push_back((run, request));
        }
        drop(state);
        Ok(run)
    }

    /// The run currently permitted to spawn a child, if any.
    #[must_use]
    pub fn in_flight(&self) -> Option<InstallRunId> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .in_flight
    }

    /// Retire the in-flight run and promote the next one, in arrival order.
    ///
    /// Returns the request now permitted to run, so the caller cannot promote out of order or
    /// start two at once — the ordering is the queue's to decide, not its caller's.
    pub fn finish(&self, run: InstallRunId) -> Option<(InstallRunId, InstallRequest)> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.in_flight != Some(run) {
            return None;
        }
        state.tokens.remove(&run.0);
        let next = state.waiting.pop_front();
        state.in_flight = next.as_ref().map(|(id, _)| *id);
        next
    }

    /// Record the token that stops this run, so `cancel` can reach it.
    pub fn register_cancel(&self, run: InstallRunId, token: CancelToken) {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .tokens
            .insert(run.0, token);
    }

    /// Stop one run.
    ///
    /// Returns whether a live run was found. **A run this queue does not know is not an error to
    /// invent a kill for**: a replay after a restart would otherwise fire at a group a later run
    /// may by then own, which is exactly why `install.cancel` is non-idempotent.
    pub fn cancel(&self, run: InstallRunId) -> bool {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state
            .tokens
            .get(&run.0)
            .inspect(|token| token.cancel())
            .is_some()
    }

    /// How many runs are waiting behind the one in flight.
    #[must_use]
    pub fn waiting(&self) -> usize {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .waiting
            .len()
    }
}
