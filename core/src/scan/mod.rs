//! §4 the scanner. This module owns J0 — the walk, discovery, presence and the run — and
//! nothing else: identity is plan 08's, the jobs are plan 09's.
//!
//! Submodules are declared by the task that creates them, so this file grows through the plan.

pub mod discover;
pub mod launcher;
pub mod links;
pub mod presence;
pub mod run;
pub mod skiplist;
pub mod store;
pub mod submodules;
pub mod walk;
pub mod wsl;

/// The six `scan_problem.kind` values §11.1 groups. Deferred-slow is the seventh group and reads
/// `project_job_state`, not this table; ambiguous lineage is the eighth and reads `project`.
/// Neither is a `ScanProblemKind` and neither may be added here.
///
/// R26: these strings are the serialised protocol values character for character, and the
/// `scan_problem.kind` CHECK constraint spells the same six. A rename here is a migration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanProblemKind {
    PermissionDenied,
    UntrustedRepo,
    UnreadableRepo,
    ClockSkew,
    NonUtf8Path,
    OfflineStore,
}

impl ScanProblemKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PermissionDenied => "permission_denied",
            Self::UntrustedRepo => "untrusted_repo",
            Self::UnreadableRepo => "unreadable_repo",
            Self::ClockSkew => "clock_skew",
            Self::NonUtf8Path => "non_utf8_path",
            Self::OfflineStore => "offline_store",
        }
    }

    /// Every kind, so a test can walk the vocabulary without restating it.
    pub const ALL: [Self; 6] = [
        Self::PermissionDenied,
        Self::UntrustedRepo,
        Self::UnreadableRepo,
        Self::ClockSkew,
        Self::NonUtf8Path,
        Self::OfflineStore,
    ];
}

/// One row of `scan_problem` (§1.9). `detail` is diagnostic and the shell owns the prose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanProblem {
    pub kind: ScanProblemKind,
    pub path_display: String,
    pub detail: String,
}

/// Everything the walk emits. Variants are added by later tasks in this plan; match with a
/// `_` arm.
///
/// **R8: this is `WalkEvent`, not ~~`ScanEvent`~~.** Plan 02 generates a `ScanEvent` into
/// `core/src/protocol.rs` from `protocol/schema/protocol.json` — the payload the renderer
/// receives. That name belongs to the schema. This enum carries `WslBridge` and the
/// walk-progress cases, which never reach the wire, so it yields the name and takes the one that
/// describes it.
#[derive(Debug)]
#[non_exhaustive]
pub enum WalkEvent {
    Repo(discover::RepoCandidate),
    Discovered(Box<run::Discovered>),
    SubmoduleEdge(Box<submodules::SubmoduleEdgeCandidate>),
    Problem(ScanProblem),
    WslBridge {
        distro: String,
        path_display: String,
    },
    /// Raw walk counter, emitted every 512 directories. The run layer converts these into
    /// `Progress`; nothing else should consume them.
    Walked {
        dirs: u64,
    },
    Progress {
        walked_dirs: u64,
        found_repos: u64,
    },
}

/// The sink every walk function writes through. It is called from walk worker threads.
///
/// **R16: this is `WalkSink`, not ~~`EventSink`~~.** `core::proto::EventSink` is a *trait*
/// (`emit(&self, topic, event, payload)`) that plan 03 declares and plans 09, 10 and 11b consume
/// as `Arc<dyn EventSink>` — the publish seam for protocol events. This is a closure alias
/// carrying `WalkEvent`, never a protocol event, and the two must not share a name.
pub type WalkSink<'a> = dyn Fn(WalkEvent) + Send + Sync + 'a;

// R4: cancellation is **not declared here**. This plan previously wrote
// `pub struct CancelToken(AtomicBool)` — cooperative cancellation for one scan run (§4.8), cheap
// enough to poll per directory. It is superseded: a bare `AtomicBool` is not `Clone`, and
// `walk_root` hands a token to every `ignore` worker thread, so it could never have compiled.
// Plan 05's `Arc<AtomicBool>` form is the one, and it lives at `crate::cancel::CancelToken`.

/// Per-root walk policy. `descend_into_repos` and `bare_candidates` come from the `scan_root`
/// row (§1.9); `follow_links` is off by default per §4.3.
#[derive(Debug, Clone, Copy)]
pub struct WalkOptions {
    pub threads: usize,
    pub follow_links: bool,
    pub descend_into_repos: bool,
    pub bare_candidates: bool,
}

impl Default for WalkOptions {
    fn default() -> Self {
        let cores = std::thread::available_parallelism().map_or(4, std::num::NonZeroUsize::get);
        Self {
            // The probe measured 101k dirs/s at 8 threads; more threads buy nothing and cost
            // file-handle pressure on a network root.
            threads: cores.min(8),
            follow_links: false,
            descend_into_repos: false,
            bare_candidates: true,
        }
    }
}

pub mod commands;

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use crate::cancel::CancelToken;
use crate::proto::dispatch::CommandFailure;
use crate::proto::EventSink;
use crate::protocol::{RootId, ScanMode, ScanRunId};
use crate::scan::presence::{ScanStore, ScanStoreError};

/// Everything a `scan.*` command needs. `now` is unix **seconds** (R3), supplied by the caller so
/// no handler reads a clock of its own.
///
/// **`store`, not an `Index`.** Plan 07 Task 12 gives this an `&Index` and has `scan.status` run
/// its own SQL. `ScanStore` exists so the scanner has exactly one seam onto the database (R1,
/// R40), and the production store owns the connection behind a mutex because
/// `rusqlite::Connection` is `Send` and not `Sync` (R39) — so a second handle here would mean
/// holding that lock for a whole command while the scan worker waits on it.
///
/// `Debug` is written by hand: neither `ScanStore` nor `EventSink` requires it, and
/// `missing_debug_implementations` is denied crate-wide.
pub struct ScanCtx<'a> {
    pub store: &'a dyn ScanStore,
    pub events: &'a dyn EventSink,
    pub scans: &'a ScanSupervisor,
    pub now: i64,
}

impl std::fmt::Debug for ScanCtx<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScanCtx")
            .field("now", &self.now)
            .finish_non_exhaustive()
    }
}

/// The live run's counters, shared with the walk.
///
/// `scan.status` reads them here because the `scan_run` row does not carry them until
/// `finish_scan_run` writes them at the end: reading the row mid-run would report a walk of
/// 214,903 directories as a walk of zero.
#[derive(Debug, Default)]
pub struct ScanProgressCell {
    walked_dirs: AtomicU64,
    found_repos: AtomicU64,
    finished: AtomicBool,
}

impl ScanProgressCell {
    /// Called from the run layer with the same figures `WalkEvent::Progress` carries.
    pub fn observe(&self, walked_dirs: u64, found_repos: u64) {
        self.walked_dirs.store(walked_dirs, Ordering::Relaxed);
        self.found_repos.store(found_repos, Ordering::Relaxed);
    }

    #[must_use]
    pub fn snapshot(&self) -> (u64, u64) {
        (
            self.walked_dirs.load(Ordering::Relaxed),
            self.found_repos.load(Ordering::Relaxed),
        )
    }

    /// The worker's last act, cancelled or not. This is what clears the supervisor's slot, so a
    /// launcher that forgets it leaves the library unable to start a second scan.
    pub fn finish(&self) {
        self.finished.store(true, Ordering::SeqCst);
    }

    #[must_use]
    pub fn is_finished(&self) -> bool {
        self.finished.load(Ordering::SeqCst)
    }
}

/// What a launcher reports back once the `scan_run` row exists and the walk is under way.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchedScan {
    pub scan_run_id: ScanRunId,
    pub generation: i64,
    pub roots: Vec<RootId>,
}

/// The one run this process has in flight.
#[derive(Debug, Clone)]
pub struct LiveScan {
    pub scan_run_id: ScanRunId,
    pub generation: i64,
    pub mode: ScanMode,
    pub started_at: i64,
    pub roots: Vec<RootId>,
    pub cancel: CancelToken,
    pub progress: Arc<ScanProgressCell>,
}

/// Starts a run and returns as soon as its `scan_run` row exists — **never** after the walk
/// finishes, which takes tens of seconds and would block the protocol loop.
///
/// This is a seam and not a concrete type on purpose. `rusqlite::Connection` is `Send` but not
/// `Sync` (R39), so no `Index` crosses this boundary. The implementation is also what maps
/// `WalkEvent` onto the `scan` topic (R8) and what calls `ScanRunner::run` with the roots
/// `ScanStore::scan_roots` returns — §2.4: no path reaches it from the caller.
pub trait ScanLauncher: Send + Sync {
    fn launch(
        &self,
        mode: ScanMode,
        now: i64,
        cancel: CancelToken,
        progress: Arc<ScanProgressCell>,
    ) -> Result<LaunchedScan, ScanStoreError>;
}

/// Owns the answer to "is a scan running", which is the one fact `scan.start`, `scan.cancel` and
/// `scan.status` all turn on.
pub struct ScanSupervisor {
    launcher: Arc<dyn ScanLauncher>,
    live: Mutex<Option<LiveScan>>,
}

impl std::fmt::Debug for ScanSupervisor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScanSupervisor")
            .field("live", &self.live().is_some())
            .finish_non_exhaustive()
    }
}

impl ScanSupervisor {
    #[must_use]
    pub fn new(launcher: Arc<dyn ScanLauncher>) -> Self {
        Self {
            launcher,
            live: Mutex::new(None),
        }
    }

    /// A poisoned lock must **not** read as "nothing is running": that is the one state in which
    /// a second run could be started beside a live one, which is what this type exists to
    /// prevent. `into_inner` keeps the slot readable instead.
    fn slot(&self) -> MutexGuard<'_, Option<LiveScan>> {
        self.live.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Drop a run whose worker has reported it stopped.
    ///
    /// The completion flag lives on the progress cell the launcher already holds, so a launcher
    /// needs no back-reference to the supervisor that owns it — a `Weak` cycle, set after
    /// construction, is the alternative and it is one more thing to wire up wrongly.
    fn reap(slot: &mut Option<LiveScan>) {
        if slot
            .as_ref()
            .is_some_and(|live| live.progress.is_finished())
        {
            *slot = None;
        }
    }

    #[must_use]
    pub fn live(&self) -> Option<LiveScan> {
        let mut slot = self.slot();
        Self::reap(&mut slot);
        slot.clone()
    }

    /// The second element is `true` when this call started the run, and `false` when it coalesced
    /// onto a live one. It is what decides whether `run_started` is emitted.
    pub fn start(&self, mode: ScanMode, now: i64) -> Result<(LiveScan, bool), ScanStoreError> {
        let mut slot = self.slot();
        Self::reap(&mut slot);
        if let Some(live) = slot.as_ref() {
            return Ok((live.clone(), false));
        }
        let cancel = CancelToken::new();
        let progress = Arc::new(ScanProgressCell::default());
        // The lock is held across the launch on purpose: released first, two callers would both
        // see an empty slot and two generations would race.
        let launched = self
            .launcher
            .launch(mode, now, cancel.clone(), Arc::clone(&progress))?;
        let live = LiveScan {
            scan_run_id: launched.scan_run_id,
            generation: launched.generation,
            mode,
            started_at: now,
            roots: launched.roots,
            cancel,
            progress,
        };
        *slot = Some(live.clone());
        Ok((live, true))
    }

    /// True when the id named the live run and its token was set. §4.8's cancellation is
    /// cooperative, so this returns before the run has stopped — and it removes no row (§17).
    pub fn cancel(&self, scan_run_id: ScanRunId) -> bool {
        let slot = self.slot();
        match slot.as_ref() {
            Some(live) if live.scan_run_id == scan_run_id => {
                live.cancel.cancel();
                true
            }
            _ => false,
        }
    }

    /// Clear the slot explicitly. Equivalent to the worker calling `progress.finish()`; both
    /// exist because a launcher that spawns no thread has nothing to set the flag from.
    pub fn finished(&self, scan_run_id: ScanRunId) {
        let mut slot = self.slot();
        if slot
            .as_ref()
            .is_some_and(|live| live.scan_run_id == scan_run_id)
        {
            *slot = None;
        }
    }
}

/// The commands this module owns, in the order the dispatcher matches them. Exposed so the table
/// can be asserted without constructing a store.
#[must_use]
pub fn dispatch_scan_command_names() -> [&'static str; 3] {
    ["scan.start", "scan.cancel", "scan.status"]
}

/// `None` means "this module does not own that command" — plan 21's router relies on it to try
/// the next dispatcher instead of refusing.
#[must_use]
pub fn dispatch_scan_command(
    ctx: &ScanCtx<'_>,
    command: &str,
    args: serde_json::Value,
) -> Option<Result<serde_json::Value, CommandFailure>> {
    match command {
        "scan.start" => Some(commands::handle_start(ctx, args)),
        "scan.cancel" => Some(commands::handle_cancel(ctx, args)),
        "scan.status" => Some(commands::handle_status(ctx)),
        _ => None,
    }
}
