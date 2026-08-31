//! The §9 state machine, pure. No clock, no connection, no filesystem, no handle.
//!
//! Two decisions are encoded here that §9 leaves implicit, and both are load-bearing.
//!
//! **A segment credits open-to-close, not open-to-last-activity.** §9 bounds a session with no
//! worktree change at all to 20 minutes total; under open-to-last-activity such a session would
//! credit zero and the bound would say nothing. The 20-minute window at the tail of a segment is
//! credited: it is the bound the threshold exists to impose on time the app cannot observe.
//!
//! **An idle close is stamped when the rule fired, not when the tick noticed.** `session.ended_at`
//! has one downstream reader — §5.1's *last session end* term in `last_interaction_at` — and
//! stamping it at the tick would claim interaction that did not happen. An *explicit* end is
//! stamped at the tick, because that event did happen then.

use crate::session::{CloseReason, ClosedBy, SEGMENT_IDLE_MS, SEGMENT_IDLE_SECS, SESSION_IDLE_MS};

/// One reading of both clocks, taken together (R3): `mono_ms` decides, `unix` records.
///
/// The idle rule runs on the monotonic clock because a wall clock can be moved backwards and
/// would otherwise be a way to hold a segment open; every value written to a column is the wall
/// clock, in epoch seconds, because that is what the columns hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tick {
    pub mono_ms: u64,
    pub unix: i64,
}

/// Everything §9 lets extend a segment, and nothing else. There is deliberately **no** variant
/// for another project's view: shelf browsing cannot reach this machine at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signal {
    /// An in-scope worktree change, already filtered by `activity.rs`.
    Worktree,
    /// This project's own view — its page or its live tile — is focused.
    OwnView,
    /// Only time passed.
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClosedSegment {
    pub started_at: i64,
    pub ended_at: i64,
    pub credited_seconds: i64,
    pub closed_by: ClosedBy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SegmentOutcome {
    pub opened_segment_at: Option<i64>,
    /// The wall time of the activity that extended the open segment, for the durable watermark.
    pub activity_at: Option<i64>,
    pub closed_segment: Option<ClosedSegment>,
    pub ended_session: Option<(i64, CloseReason)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    InSegment {
        started_at: i64,
        last_at: i64,
        last_mono_ms: u64,
    },
    Between {
        since_at: i64,
        since_mono_ms: u64,
    },
    Ended,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SegmentMachine {
    phase: Phase,
    /// §9 mechanism 1: while a wait-mode process is alive the session ignores gaps entirely.
    wait_mode: bool,
}

/// Credit is never negative, however the wall clock moved between two readings.
fn credit(started_at: i64, ended_at: i64) -> i64 {
    ended_at.saturating_sub(started_at).max(0)
}

impl SegmentMachine {
    /// §9: a segment opens on launch.
    #[must_use]
    pub fn open(at: Tick, wait_mode: bool) -> (SegmentMachine, SegmentOutcome) {
        let machine = SegmentMachine {
            phase: Phase::InSegment {
                started_at: at.unix,
                last_at: at.unix,
                last_mono_ms: at.mono_ms,
            },
            wait_mode,
        };
        (
            machine,
            SegmentOutcome {
                opened_segment_at: Some(at.unix),
                ..SegmentOutcome::default()
            },
        )
    }

    #[must_use]
    pub fn in_segment(&self) -> bool {
        matches!(self.phase, Phase::InSegment { .. })
    }

    #[must_use]
    pub fn is_ended(&self) -> bool {
        matches!(self.phase, Phase::Ended)
    }

    /// One tick of the machine: a reading of both clocks and whatever §9 counts as having
    /// happened since the last one.
    pub fn observe(&mut self, at: Tick, signal: Signal) -> SegmentOutcome {
        let mut out = SegmentOutcome::default();
        match self.phase {
            Phase::Ended => out,
            Phase::InSegment {
                started_at,
                last_at,
                last_mono_ms,
            } => {
                if signal != Signal::None {
                    let last_at = at.unix.max(last_at);
                    self.phase = Phase::InSegment {
                        started_at,
                        last_at,
                        last_mono_ms: at.mono_ms.max(last_mono_ms),
                    };
                    out.activity_at = Some(last_at);
                    return out;
                }
                if at.mono_ms.saturating_sub(last_mono_ms) < SEGMENT_IDLE_MS {
                    return out;
                }
                // Open-to-close, stamped when the rule fired rather than when the tick noticed.
                // `.min(at.unix)` is what keeps three suspended hours from crediting three hours.
                let ended_at = last_at
                    .saturating_add(SEGMENT_IDLE_SECS)
                    .min(at.unix)
                    .max(started_at);
                out.closed_segment = Some(ClosedSegment {
                    started_at,
                    ended_at,
                    credited_seconds: credit(started_at, ended_at),
                    closed_by: ClosedBy::Idle,
                });
                self.phase = Phase::Between {
                    since_at: ended_at,
                    since_mono_ms: last_mono_ms.saturating_add(SEGMENT_IDLE_MS),
                };
                out
            }
            Phase::Between {
                since_at,
                since_mono_ms,
            } => {
                if signal != Signal::None {
                    // §9: a segment re-opens on a worktree change or on this project's own view,
                    // and it is the same session that continues.
                    self.phase = Phase::InSegment {
                        started_at: at.unix,
                        last_at: at.unix,
                        last_mono_ms: at.mono_ms,
                    };
                    out.opened_segment_at = Some(at.unix);
                    return out;
                }
                if self.wait_mode {
                    return out;
                }
                if at.mono_ms.saturating_sub(since_mono_ms) < SESSION_IDLE_MS {
                    return out;
                }
                self.phase = Phase::Ended;
                out.ended_session = Some((since_at, CloseReason::Idle));
                out
            }
        }
    }

    /// Stop, process exit, app exit, or a location going offline. The end mechanism decides
    /// **when** the session closes, never **what** it credits.
    pub fn end(&mut self, at: Tick, reason: CloseReason) -> SegmentOutcome {
        let mut out = SegmentOutcome::default();
        if self.is_ended() {
            return out;
        }
        let closed_by = if reason == CloseReason::AppExit {
            ClosedBy::AppExit
        } else {
            ClosedBy::SessionEnd
        };
        if let Phase::InSegment { started_at, .. } = self.phase {
            // No trailing idle window: the end was observed, so there is no unobserved time to
            // bound. The window exists only for time the app could not see.
            let ended_at = at.unix.max(started_at);
            out.closed_segment = Some(ClosedSegment {
                started_at,
                ended_at,
                credited_seconds: credit(started_at, ended_at),
                closed_by,
            });
        }
        self.phase = Phase::Ended;
        out.ended_session = Some((at.unix, reason));
        out
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use super::*;

    const T0: i64 = 1_700_000_000;

    fn at(secs_in: i64) -> Tick {
        Tick {
            mono_ms: secs_in.unsigned_abs() * 1_000,
            unix: T0 + secs_in,
        }
    }

    #[test]
    fn a_tree_that_never_changes_credits_at_most_twenty_minutes() {
        // Criterion 11, clause 2. §9: "A session with no worktree change at all credits
        // <= 20 minutes total, regardless of length."
        let (mut m, opened) = SegmentMachine::open(at(0), false);
        assert_eq!(opened.opened_segment_at, Some(T0));

        assert_eq!(
            m.observe(at(1_199), Signal::None),
            SegmentOutcome::default(),
            "not yet"
        );
        let closed = m
            .observe(at(1_200), Signal::None)
            .closed_segment
            .expect("idle close");
        assert_eq!(closed.credited_seconds, 1_200);
        assert_eq!(closed.closed_by, ClosedBy::Idle);

        // Eight hours later the session has ended and nothing further is credited.
        let ended = m.observe(at(28_800), Signal::None);
        assert!(ended.closed_segment.is_none());
        assert!(m.is_ended());
    }

    #[test]
    fn a_save_after_an_idle_close_reopens_a_segment_on_the_same_session() {
        // §9's own example: a 09:30 idle close, a save at 10:10, one session.
        let (mut m, _) = SegmentMachine::open(at(0), false);
        assert!(m.observe(at(1_200), Signal::None).closed_segment.is_some());
        let out = m.observe(at(2_400), Signal::Worktree);
        assert_eq!(out.opened_segment_at, Some(T0 + 2_400));
        assert!(out.ended_session.is_none(), "the session did not end");
        assert!(m.in_segment());
    }

    #[test]
    fn focus_on_this_projects_own_view_extends_a_segment() {
        let (mut m, _) = SegmentMachine::open(at(0), false);
        assert_eq!(
            m.observe(at(600), Signal::OwnView).activity_at,
            Some(T0 + 600)
        );
        assert_eq!(
            m.observe(at(1_500), Signal::None),
            SegmentOutcome::default(),
            "extended past 1200"
        );
        let closed = m
            .observe(at(1_800), Signal::None)
            .closed_segment
            .expect("close");
        assert_eq!(
            closed.credited_seconds, 1_800,
            "600 of work plus the 20-minute window"
        );
    }

    #[test]
    fn without_wait_mode_the_session_ends_an_hour_after_the_last_segment() {
        let (mut m, _) = SegmentMachine::open(at(0), false);
        let closed = m
            .observe(at(1_200), Signal::None)
            .closed_segment
            .expect("close");
        assert_eq!(
            m.observe(at(1_200 + 3_599), Signal::None).ended_session,
            None
        );
        let (ended_at, reason) = m
            .observe(at(1_200 + 3_600), Signal::None)
            .ended_session
            .expect("the session ends when no new segment opens within 60 minutes");
        assert_eq!(reason, CloseReason::Idle);
        assert_eq!(
            ended_at, closed.ended_at,
            "stamped when it stopped, not when we noticed"
        );
    }

    #[test]
    fn a_wait_mode_session_survives_a_gap_that_would_end_an_idle_one() {
        // §9: "while a wait-mode process is alive the session stays open regardless of gaps"
        // -- lunch closes a segment, and the afternoon rejoins the same session.
        let (mut m, _) = SegmentMachine::open(at(0), true);
        assert!(m.observe(at(1_200), Signal::None).closed_segment.is_some());
        assert!(m.observe(at(20_000), Signal::None).ended_session.is_none());
        assert!(!m.is_ended());
        assert_eq!(
            m.observe(at(20_400), Signal::Worktree).opened_segment_at,
            Some(T0 + 20_400)
        );
    }

    #[test]
    fn an_explicit_end_closes_the_open_segment_at_the_moment_it_happened() {
        let (mut m, _) = SegmentMachine::open(at(0), true);
        let out = m.end(at(900), CloseReason::ProcessExit);
        let closed = out
            .closed_segment
            .expect("the open segment closes with the session");
        assert_eq!(
            closed.credited_seconds, 900,
            "no trailing idle window on an observed end"
        );
        assert_eq!(closed.closed_by, ClosedBy::SessionEnd);
        assert_eq!(
            out.ended_session,
            Some((T0 + 900, CloseReason::ProcessExit))
        );
    }

    #[test]
    fn app_exit_marks_its_segment_as_such_rather_than_as_a_session_end() {
        let (mut m, _) = SegmentMachine::open(at(0), false);
        let closed = m
            .end(at(60), CloseReason::AppExit)
            .closed_segment
            .expect("close");
        assert_eq!(closed.closed_by, ClosedBy::AppExit);
    }

    #[test]
    fn a_second_end_is_not_a_second_close() {
        // The manager has several end paths -- stop, process exit, app exit, a location going
        // offline -- and more than one can fire for the same session in the same tick.
        let (mut m, _) = SegmentMachine::open(at(0), false);
        assert!(m.end(at(300), CloseReason::Stop).closed_segment.is_some());
        assert_eq!(
            m.end(at(400), CloseReason::AppExit),
            SegmentOutcome::default(),
            "nothing is credited or ended twice"
        );
    }

    #[test]
    fn three_hours_of_suspend_credit_twenty_minutes_not_three_hours() {
        // The idle rule is decided on the monotonic clock; the record is wall time. A tick that
        // arrives long after the rule fired must not credit the gap it slept through.
        let (mut m, _) = SegmentMachine::open(at(0), false);
        let closed = m
            .observe(at(10_800), Signal::None)
            .closed_segment
            .expect("close");
        assert_eq!(closed.credited_seconds, 1_200);
        assert_eq!(closed.ended_at, T0 + 1_200);
    }

    #[test]
    fn a_backwards_wall_clock_never_credits_a_negative_span() {
        let (mut m, _) = SegmentMachine::open(at(0), false);
        let late = Tick {
            mono_ms: 1_200_000,
            unix: T0 - 500,
        };
        let closed = m.observe(late, Signal::None).closed_segment.expect("close");
        assert!(closed.credited_seconds >= 0, "credit is never negative");
        assert!(closed.ended_at >= closed.started_at);
    }
}
