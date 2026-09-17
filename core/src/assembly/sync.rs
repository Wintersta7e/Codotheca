//! Building, starting and stopping the sync pump.
//!
//! **This is construction, not a second runner.** `SyncRunner` owns its one thread; what lives
//! here is the cancel-then-join pair, in the same shape and the same order as
//! `crate::assembly::jobs::JobPump` (`core/src/assembly/jobs.rs:99-108`).

use std::sync::{Arc, Mutex};

use crate::cancel::CancelToken;
use crate::index::Index;
use crate::proto::EventSink;
use crate::sync::events::SyncLive;
use crate::sync::runner::{SyncRunner, SyncSink};
use crate::sync::SyncDeps;

/// The running pump: the runner, and the token that stops it taking further work.
///
/// The two travel together because stopping is two acts in one order. `request_stop` only asks
/// the thread to take no *further* task; a thread parked inside a thirty-second request would
/// keep a process alive whose window is already closed.
#[derive(Clone)]
pub struct SyncPump {
    runner: Arc<SyncRunner>,
    cancel: CancelToken,
}

impl std::fmt::Debug for SyncPump {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SyncPump").finish_non_exhaustive()
    }
}

impl SyncPump {
    /// Build the runner, sweep the crash leftovers and spawn its thread.
    ///
    /// The cancel token is **taken from the deps** rather than made here, so the token the loop
    /// checks is the same one every request under it would see. A second token would stop the
    /// loop and leave the work it started running.
    #[must_use]
    pub fn start(index: Arc<Mutex<Index>>, deps: SyncDeps, events: Arc<dyn EventSink>) -> SyncPump {
        let cancel = deps.cancel.clone();
        let runner = SyncRunner::new(index, deps, events);
        runner.start();
        SyncPump { runner, cancel }
    }

    /// The pump seen as the two visibility call sites' hand-off point.
    #[must_use]
    pub fn sink(&self) -> Arc<dyn SyncSink> {
        Arc::clone(&self.runner) as Arc<dyn SyncSink>
    }

    /// The same seam borrowed rather than cloned, for a command context that lives one call.
    #[must_use]
    pub fn sink_ref(&self) -> &dyn SyncSink {
        self.runner.as_ref()
    }

    /// What only the running process knows, for `sync.status` and the `sync` snapshot.
    #[must_use]
    pub fn live(&self) -> SyncLive {
        self.runner.live()
    }

    /// Stop the loop and wait for it.
    ///
    /// **Cancel, then ask, then join** — the order `JobPump::stop` uses. Idempotent, and it must
    /// be: it runs from `CoreHandler::shutdown`, which the loop calls once, and a second call has
    /// to be harmless rather than a second join on a taken handle.
    pub fn stop(&self) {
        self.cancel.cancel();
        self.runner.request_stop();
        self.runner.join();
    }
}
