//! §43.10: when first run ends, written on the paths that end it.
//!
//! The stamp itself is [`crate::firstrun::stamp_first_run_completed`] — write-once, epoch
//! seconds. This module decides **when** to call it: at once when the turn is answered and no
//! scan run is in progress, otherwise at the end of the run in progress; and, for a core that
//! starts holding a live project and no stamp (a first run whose reveal was skipped), at the end
//! of the first run to end in its lifetime.
//!
//! **The end of the run, not the click:** after `SKIP AHEAD` the walk is still running when the
//! turn is answered, and a stamp there would mark every project the walk's tail discovers as
//! new — §10.5a's predicate is `created_at > first_run_completed_at`.

use rusqlite::OptionalExtension as _;

use crate::firstrun::{first_run_completed_at, stamp_first_run_completed};
use crate::index::IndexError;
use crate::projects::current::IS_CURRENT_SQL;
use crate::protocol::ScanRunId;
use crate::stats::reveal::LIVE;

/// What the stamp is waiting for.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FirstRunArm {
    /// Nothing: the stamp is written, or no path has armed it in this lifetime.
    #[default]
    Idle,
    /// The first scan run after `after` (a `scan_run.id`; 0 when none existed at the arming) to
    /// end stamps — whether or not the loop ever saw it live, so a run that starts and ends
    /// between two loop steps is not missed.
    AwaitRunStart {
        /// The highest `scan_run.id` when this arm was set; only later runs count.
        after: i64,
    },
    /// This run's end stamps — if it finished or was cancelled. A failed run re-arms for the
    /// next one.
    AwaitRunEnd(ScanRunId),
}

/// The first-run stamp's one owner in a running core: armed by the turn's answer or by
/// start-up, stamped by the loop when the watched run ends.
#[derive(Debug, Clone, Copy, Default)]
pub struct FirstRunStamp {
    arm: FirstRunArm,
}

impl FirstRunStamp {
    /// Path 2: armed for the next run to start when the index holds a live, current project and
    /// no stamp — an empty library has nothing a first run ended over, so it stays idle.
    ///
    /// # Errors
    /// Returns [`IndexError`] when either read fails.
    pub fn at_start(conn: &rusqlite::Connection) -> Result<Self, IndexError> {
        if first_run_completed_at(conn)?.is_some() {
            return Ok(Self::default());
        }
        let shelf: bool = conn.query_row(
            &format!("SELECT EXISTS (SELECT 1 FROM project p WHERE {LIVE} AND {IS_CURRENT_SQL})"),
            [],
            |r| r.get(0),
        )?;
        if !shelf {
            return Ok(Self::default());
        }
        let after: i64 = conn.query_row("SELECT COALESCE(MAX(id), 0) FROM scan_run", [], |r| {
            r.get(0)
        })?;
        Ok(Self {
            arm: FirstRunArm::AwaitRunStart { after },
        })
    }

    /// What the stamp is waiting for now.
    #[must_use]
    pub const fn arm(&self) -> FirstRunArm {
        self.arm
    }

    /// Path 1: the turn was answered. Stamps `now` when no run is `live`; otherwise waits for
    /// that run's end. `true` when this call wrote the stamp.
    ///
    /// # Errors
    /// Returns [`IndexError`] when the stamp cannot be read or written.
    pub fn on_first_run_ended(
        &mut self,
        conn: &rusqlite::Connection,
        live: Option<ScanRunId>,
        now: i64,
    ) -> Result<bool, IndexError> {
        if let Some(run) = live {
            if first_run_completed_at(conn)?.is_none() {
                self.arm = FirstRunArm::AwaitRunEnd(run);
            }
            return Ok(false);
        }
        self.arm = FirstRunArm::Idle;
        stamp_first_run_completed(conn, now)
    }

    /// One step of the loop. An awaited start looks at the lowest run after its mark: live, it
    /// becomes an awaited end; already over, it is judged as an awaited end is. An awaited end,
    /// once that run is no longer live, stamps `now` if the run's `ended_at` was written
    /// (finished or cancelled) and re-arms after that run if it was not (failed). `true` when
    /// this call wrote the stamp, so the caller re-publishes `core/snapshot`.
    ///
    /// # Errors
    /// Returns [`IndexError`] when the run's row or the stamp cannot be read, or the stamp
    /// cannot be written. The arm keeps what this step settled before the error, so the next step
    /// resumes there.
    pub fn poll(
        &mut self,
        conn: &rusqlite::Connection,
        live: Option<ScanRunId>,
        now: i64,
    ) -> Result<bool, IndexError> {
        loop {
            let (run, ended_at) = match self.arm {
                FirstRunArm::Idle => return Ok(false),
                FirstRunArm::AwaitRunEnd(run) if live == Some(run) => return Ok(false),
                FirstRunArm::AwaitRunEnd(run) => {
                    let ended_at: Option<Option<i64>> = conn
                        .query_row(
                            "SELECT ended_at FROM scan_run WHERE id = ?1",
                            [run.0],
                            |r| r.get(0),
                        )
                        .optional()?;
                    (run, ended_at.flatten())
                }
                FirstRunArm::AwaitRunStart { after } => {
                    let next: Option<(i64, Option<i64>)> = conn
                        .query_row(
                            "SELECT id, ended_at FROM scan_run WHERE id > ?1 ORDER BY id LIMIT 1",
                            [after],
                            |r| Ok((r.get(0)?, r.get(1)?)),
                        )
                        .optional()?;
                    let Some((id, ended_at)) = next else {
                        return Ok(false);
                    };
                    let run = ScanRunId(id);
                    if live == Some(run) {
                        self.arm = FirstRunArm::AwaitRunEnd(run);
                        return Ok(false);
                    }
                    (run, ended_at)
                }
            };
            if ended_at.is_some() {
                let stamped = stamp_first_run_completed(conn, now)?;
                self.arm = FirstRunArm::Idle;
                return Ok(stamped);
            }
            // A failed run (or a row that is gone) ended nothing: the next run after it does,
            // and it may already have ended too.
            self.arm = FirstRunArm::AwaitRunStart { after: run.0 };
        }
    }
}
