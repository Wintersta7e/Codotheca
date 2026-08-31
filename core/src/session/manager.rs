//! The live session lifecycle. Synchronous: the command layer calls `tick`, nothing sleeps.
//!
//! **One writer (§1.10).** The manager holds no connection of its own; the command layer that
//! already owns the `Index` passes it in. There is no `Arc<Mutex<Index>>` here and no thread — a
//! driver that slept on the clock would spin under `FakeClock`, whose `sleep` returns at once,
//! so the tick is *called*, never awaited.
//!
//! **The farming hole, closed by routing.** A session sees `Signal::OwnView` only when the
//! focused project is its own *and* the focus report is younger than `FOCUS_STALE_MS`. Browsing
//! the shelf focuses no project and therefore extends nothing, anywhere; a renderer that died
//! holding focus stops extending within two minutes.
//!
//! **Nothing here kills a process.** §17: phase 1 has no destructive operation, and §7.8 says
//! Stop "writes nothing to disk". `stop` closes the ledger, never the editor.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::index::Index;
use crate::protocol::{
    CloseReason, LocationId, ProjectId, SegmentClosed, SessionEnded, SessionEvent, SessionId,
    SessionStarted, TargetId,
};
use crate::session::activity::{IgnoreCheck, ScopeFilter};
use crate::session::segment::{SegmentMachine, SegmentOutcome, Signal, Tick};
use crate::session::store::{self, SessionOpen};
use crate::session::watch::ActivitySource;
use crate::session::{SessionError, FOCUS_STALE_MS};

/// Everything a successful spawn hands the ledger.
#[derive(Debug)]
pub struct LaunchedSession {
    pub project_id: ProjectId,
    pub location_id: Option<LocationId>,
    pub target_id: Option<TargetId>,
    pub repo: crate::git::RepoHandle,
    /// `Some` in wait mode only (§9 mechanism 1). Joined, never killed (§17).
    pub waiter: Option<std::thread::JoinHandle<Option<i32>>>,
}

#[derive(Debug)]
struct Live {
    project_id: ProjectId,
    location_id: Option<LocationId>,
    repo: crate::git::RepoHandle,
    machine: SegmentMachine,
    segment_id: Option<i64>,
    scope: ScopeFilter,
    waiter: Option<std::thread::JoinHandle<Option<i32>>>,
}

/// The live sessions, keyed on the raw row id: the generated ids carry `Hash` but not `Ord`,
/// and every pass over the set has to be in a stable order.
pub struct SessionManager {
    clock: Arc<dyn crate::clock::Clock>,
    events: Arc<dyn crate::proto::EventSink>,
    activity: Box<dyn ActivitySource>,
    ignore: Arc<dyn IgnoreCheck>,
    live: BTreeMap<i64, Live>,
    /// The focused project and the monotonic time it was last reported.
    focus: Option<(ProjectId, u64)>,
    condition_changes: Vec<ProjectId>,
}

/// Written by hand because `EventSink` is not `Debug` — it is held as `Arc<dyn EventSink>` so a
/// worker thread needs neither the `Topic` enum nor a `&mut Publisher`.
impl std::fmt::Debug for SessionManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionManager")
            .field("live", &self.live)
            .field("focus", &self.focus)
            .field("condition_changes", &self.condition_changes)
            .finish_non_exhaustive()
    }
}

impl SessionManager {
    #[must_use]
    pub fn new(
        clock: Arc<dyn crate::clock::Clock>,
        events: Arc<dyn crate::proto::EventSink>,
        activity: Box<dyn ActivitySource>,
        ignore: Arc<dyn IgnoreCheck>,
    ) -> SessionManager {
        SessionManager {
            clock,
            events,
            activity,
            ignore,
            live: BTreeMap::new(),
            focus: None,
            condition_changes: Vec::new(),
        }
    }

    fn now(&self) -> Tick {
        Tick {
            mono_ms: self.clock.monotonic_ms(),
            unix: self.clock.now_unix(),
        }
    }

    fn publish(&self, event: &SessionEvent) {
        if let Ok(payload) = serde_json::to_value(event) {
            self.events.emit("session", event.name(), payload);
        }
    }

    /// Open the session and its first segment, and start watching the worktree.
    ///
    /// A failed watch does not fail the launch: a session with no watcher still credits by focus
    /// and by the 20-minute bound, which is honest, where refusing to launch would not be.
    pub fn launch(
        &mut self,
        index: &mut Index,
        launched: LaunchedSession,
    ) -> Result<SessionId, SessionError> {
        let now = self.now();
        let (machine, opened) = SegmentMachine::open(now, launched.waiter.is_some());

        let (session, segment_id, started) = {
            let _guard = crate::proto::txguard::TxGuard::enter();
            let tx = index.conn_mut().transaction()?;
            let session = store::open_session(
                &tx,
                &SessionOpen {
                    project_id: launched.project_id,
                    location_id: launched.location_id,
                    target_id: launched.target_id,
                    started_at: now.unix,
                },
            )?;
            let segment_id =
                store::open_segment(&tx, session, opened.opened_segment_at.unwrap_or(now.unix))?;
            let started = store::session_ref(&tx, session)?;
            tx.commit()?;
            (session, segment_id, started)
        };

        let _ = self.activity.watch(session, &launched.repo.work_dir);
        self.live.insert(
            session.0,
            Live {
                project_id: launched.project_id,
                location_id: launched.location_id,
                repo: launched.repo,
                machine,
                segment_id: Some(segment_id),
                scope: ScopeFilter::new(Arc::clone(&self.ignore)),
                waiter: launched.waiter,
            },
        );
        self.publish(&SessionEvent::Started(SessionStarted { session: started }));
        Ok(session)
    }

    /// Which project's own view is focused, and when it was reported. `None` is the shelf, a
    /// settings pane, or any other view that belongs to no project.
    pub fn set_focus(&mut self, project: Option<ProjectId>) {
        self.focus = project.map(|p| (p, self.clock.monotonic_ms()));
    }

    /// §7.8's Stop: close the ledger. It writes nothing to disk and kills nothing.
    pub fn stop(&mut self, index: &mut Index, session: SessionId) -> Result<(), SessionError> {
        self.end_one(index, session.0, CloseReason::Stop)
    }

    /// A location going offline ends the sessions launched from it.
    ///
    /// §1.6's `close_reason` enum is closed, is in the DDL CHECK and is generated into both
    /// languages, and it has no `offline` value; the credit is identical under either label,
    /// because the segments are already closed at their last observed activity. So `idle` is
    /// what is written. Recorded as a spec gap rather than a preference.
    pub fn location_offline(
        &mut self,
        index: &mut Index,
        location: LocationId,
    ) -> Result<(), SessionError> {
        let affected: Vec<i64> = self
            .live
            .iter()
            .filter(|(_, live)| live.location_id == Some(location))
            .map(|(id, _)| *id)
            .collect();
        for id in affected {
            self.end_one(index, id, CloseReason::Idle)?;
        }
        Ok(())
    }

    /// §12-15: an update landing mid-session closes it with `close_reason='app_exit'`.
    pub fn shutdown(&mut self, index: &mut Index) -> Result<(), SessionError> {
        for id in self.live.keys().copied().collect::<Vec<_>>() {
            self.end_one(index, id, CloseReason::AppExit)?;
        }
        Ok(())
    }

    fn end_one(
        &mut self,
        index: &mut Index,
        id: i64,
        reason: CloseReason,
    ) -> Result<(), SessionError> {
        let now = self.now();
        let Some(live) = self.live.get_mut(&id) else {
            // Closing a closed ledger is not a second close.
            return Ok(());
        };
        let outcome = live.machine.end(now, reason);
        self.apply(index, SessionId(id), outcome, now)
    }

    /// One pass of the whole of §9.
    pub fn tick(&mut self, index: &mut Index) -> Result<(), SessionError> {
        let now = self.now();

        // One drain per tick is what coalesces a burst into a single pass of the filter.
        let mut signals: BTreeMap<i64, Signal> = BTreeMap::new();
        for batch in self.activity.drain() {
            let Some(live) = self.live.get_mut(&batch.session.0) else {
                continue;
            };
            let signal = live.scope.fold(&live.repo, &batch.paths);
            if signal != Signal::None {
                signals.insert(batch.session.0, signal);
            }
        }

        // Focus reaches only the session whose own project is focused, and only while the report
        // is fresh. Worktree activity already recorded above wins; both are equivalent here.
        if let Some((project, reported_at)) = self.focus {
            if now.mono_ms.saturating_sub(reported_at) < FOCUS_STALE_MS {
                for (id, live) in &self.live {
                    if live.project_id == project {
                        signals.entry(*id).or_insert(Signal::OwnView);
                    }
                }
            }
        }

        for id in self.live.keys().copied().collect::<Vec<_>>() {
            let signal = signals.get(&id).copied().unwrap_or(Signal::None);
            let Some(live) = self.live.get_mut(&id) else {
                continue;
            };
            let outcome = live.machine.observe(now, signal);
            self.apply(index, SessionId(id), outcome, now)?;
        }

        // §9 mechanism 1, the preferred one. `is_finished` does not block, so each tick asks the
        // handle rather than parking on it.
        for id in self.live.keys().copied().collect::<Vec<_>>() {
            let finished = self.live.get(&id).is_some_and(|live| {
                live.waiter
                    .as_ref()
                    .is_some_and(std::thread::JoinHandle::is_finished)
            });
            if !finished {
                continue;
            }
            let Some(live) = self.live.get_mut(&id) else {
                continue;
            };
            if let Some(waiter) = live.waiter.take() {
                let _ = waiter.join();
            }
            let outcome = live.machine.end(now, CloseReason::ProcessExit);
            self.apply(index, SessionId(id), outcome, now)?;
        }
        Ok(())
    }

    /// Write one [`SegmentOutcome`] in a single transaction, then publish what it produced.
    ///
    /// The events are held until after the commit: publishing inside the transaction would
    /// announce a credit a rollback then took back.
    fn apply(
        &mut self,
        index: &mut Index,
        session: SessionId,
        outcome: SegmentOutcome,
        now: Tick,
    ) -> Result<(), SessionError> {
        if outcome == SegmentOutcome::default() {
            return Ok(());
        }
        let Some((project_id, segment_id)) = self
            .live
            .get(&session.0)
            .map(|live| (live.project_id, live.segment_id))
        else {
            return Ok(());
        };

        let mut published: Vec<SessionEvent> = Vec::new();
        let mut next_segment: Option<Option<i64>> = None;
        let mut condition_changed = false;
        let mut ended = false;

        {
            let _guard = crate::proto::txguard::TxGuard::enter();
            let tx = index.conn_mut().transaction()?;
            if let (Some(at), Some(segment)) = (outcome.activity_at, segment_id) {
                store::mark_activity(&tx, segment, at)?;
            }
            if let Some(closed) = outcome.closed_segment {
                if let Some(segment) = segment_id {
                    store::close_segment(&tx, segment, &closed)?;
                }
                next_segment = Some(None);
                published.push(SessionEvent::SegmentClosed(SegmentClosed {
                    session_id: session,
                    project_id,
                    started_at: closed.started_at,
                    ended_at: closed.ended_at,
                    credited_seconds: closed.credited_seconds,
                    closed_by: closed.closed_by,
                    session_credited_seconds: store::credited_seconds(&tx, session)?,
                }));
            }
            if let Some(at) = outcome.opened_segment_at {
                next_segment = Some(Some(store::open_segment(&tx, session, at)?));
            }
            if let Some((ended_at, reason)) = outcome.ended_session {
                let closed = store::close_session(&tx, session, ended_at, reason, now.unix)?;
                condition_changed = closed.condition_changed;
                published.push(SessionEvent::Ended(SessionEnded {
                    session: store::session_ref(&tx, session)?,
                }));
                ended = true;
            }
            tx.commit()?;
        }

        if let Some(segment) = next_segment {
            if let Some(live) = self.live.get_mut(&session.0) {
                live.segment_id = segment;
            }
        }
        if condition_changed {
            self.condition_changes.push(project_id);
        }
        if ended {
            self.activity.unwatch(session);
            self.live.remove(&session.0);
        }
        for event in &published {
            self.publish(event);
        }
        Ok(())
    }

    /// Every session still open, in id order.
    #[must_use]
    pub fn live(&self) -> Vec<SessionId> {
        self.live.keys().copied().map(SessionId).collect()
    }

    /// The projects whose `condition_signal` moved when a session closed. Publishing
    /// `projects/condition_changed` is the command layer's, not this module's.
    pub fn take_condition_changes(&mut self) -> Vec<ProjectId> {
        let mut out = std::mem::take(&mut self.condition_changes);
        out.dedup_by_key(|p| p.0);
        out
    }
}
