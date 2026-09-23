//! §28.5 — the sweep record, and the four rules that make zero mean something.
//!
//! **`debt_sweep` is the non-obvious half of this section and it is not optional.** Without it,
//! `SELECT count(*) FROM debt_item WHERE project_id = ?` returns `0` for a project with no debt
//! **and** for a project nobody ever looked at — *never render unknown as zero*, on the first day
//! the product is capable of it. Phase 1 already reached this shape independently for README
//! (`core/src/jobs/j6_content.rs:172-174`) and §29's `blob_scan` is the third arrival.
//!
//! **One row per `(project_id, source)`, upserted.** The pair a closure needs is the stored row
//! and the sweep in hand — never a history of sweeps, which nothing reads.

use rusqlite::{OptionalExtension as _, Transaction};

use super::store::StoredItem;
use super::{enum_from_text, enum_text, DebtError};
use crate::protocol::{DebtSource, DebtSweepOutcome, LocationId, ObservationBasis, ProjectId};

/// One source, looked at once, and what it saw.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SweepObservation {
    pub project: ProjectId,
    pub source: DebtSource,
    pub outcome: DebtSweepOutcome,
    /// The anchor. `None` is a sweep with no location — `abandoned_with_debt` derives from other
    /// stored observations and observes no root at all.
    pub location: Option<LocationId>,
    /// Diagnostic only: `location.scan_generation` as it stood. Read by no surface and by no
    /// closure rule.
    pub generation: Option<i64>,
    pub basis: Option<ObservationBasis>,
    /// `Some` only for `complete` and `partial`; the DDL refuses any other pairing.
    pub item_count: Option<u32>,
    pub observed_at: i64,
}

/// **Rule 1's comparison.** An observation is comparable with an item when they share an anchor
/// **and** a basis.
///
/// **A worktree-basis observation and a HEAD-basis observation are not comparable and may not be
/// diffed for a closure** (A9): one reads what is written down and the other reads what is on
/// disk, and an item present in one and absent from the other has not been fixed.
///
/// The comparison is NULL-safe in both columns — the `IS`-comparison
/// `core/src/index/subject.rs:144` already uses for `remote_key`, and in Rust that is exactly
/// `Option == Option`. **Written as `Some(a) == Some(b)` it would be SQL's `=`**, under which a
/// NULL-anchored item could never close, because `NULL = NULL` is NULL and not true.
#[must_use]
pub fn comparable(item: &StoredItem, obs: &SweepObservation) -> bool {
    item.last_seen_location_id == obs.location && item.basis == obs.basis
}

/// **Rules 1 and 2.** A closure needs a `complete` sweep, at the item's anchor, on the item's
/// basis.
///
/// **Rule 2: a `partial` sweep may open items and may never close one.** An item it did not
/// reach looks exactly like an item that is gone.
#[must_use]
pub fn may_close(item: &StoredItem, obs: &SweepObservation) -> bool {
    obs.outcome == DebtSweepOutcome::Complete && comparable(item, obs)
}

/// **Rule 4.** A sweep may be `complete` only if the repository root was present and readable.
///
/// `presence = 'present' AND removed_at IS NULL`
/// (`core/migrations/0002_locations_and_roots.sql:22-24`,
/// `core/migrations/0010_install.sql:19`). **The second half is the uninstall guard**:
/// `locations.uninstall` removes the bytes and **keeps the row**, so a naive sweep afterwards
/// finds a readable-looking absence, reports `complete` with zero items, closes every item and
/// pays for it.
pub fn root_is_observable(
    conn: &rusqlite::Connection,
    location: LocationId,
) -> Result<bool, DebtError> {
    let answer: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM location
              WHERE id = ?1 AND presence = 'present' AND removed_at IS NULL",
            [location.0],
            |r| r.get(0),
        )
        .optional()?;
    Ok(answer.is_some())
}

/// **Rule 3, applied before the diff and never after.** A sweep of a root that is not there is
/// not a sweep with zero results — **it is not a sweep**
/// (`.dev/spec/04-scanner.md:112-113`).
///
/// Zero items at a present root is `complete` with `item_count = 0`; a root that is not there is
/// `unobservable`, whatever the producer proposed. A sweep with no anchor at all is left alone:
/// `abandoned_with_debt` observes no root and has none to freeze against.
pub fn outcome_at_root(
    tx: &Transaction<'_>,
    location: Option<LocationId>,
    proposed: DebtSweepOutcome,
) -> Result<DebtSweepOutcome, DebtError> {
    let Some(location) = location else {
        return Ok(proposed);
    };
    if root_is_observable(tx, location)? {
        Ok(proposed)
    } else {
        Ok(DebtSweepOutcome::Unobservable)
    }
}

/// The outcome of the one sweep row this `(project, source)` has, or `None` when it has none.
///
/// # Errors
/// Fails when SQLite refuses the read or the stored outcome is not one this build declares.
pub fn stored_outcome(
    conn: &rusqlite::Connection,
    project: ProjectId,
    source: DebtSource,
) -> Result<Option<DebtSweepOutcome>, DebtError> {
    let raw: Option<String> = conn
        .query_row(
            "SELECT outcome FROM debt_sweep WHERE project_id = ?1 AND source = ?2",
            rusqlite::params![project.0, enum_text(&source)?],
            |r| r.get(0),
        )
        .optional()?;
    raw.map(|raw| {
        enum_from_text(&raw)
            .ok_or_else(|| DebtError::Codec(format!("debt_sweep.outcome holds {raw:?}")))
    })
    .transpose()
}

/// Write the one row this `(project, source)` has, replacing whatever it held.
///
/// The DDL's own CHECK refuses an `item_count` on an outcome that did not observe, so a caller
/// that forgets to clear it fails here rather than shipping a count nobody measured.
pub fn upsert_sweep(tx: &Transaction<'_>, obs: &SweepObservation) -> Result<(), DebtError> {
    tx.execute(
        "INSERT INTO debt_sweep
            (project_id, source, outcome, location_id, generation, basis, item_count, observed_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(project_id, source) DO UPDATE SET
            outcome     = excluded.outcome,
            location_id = excluded.location_id,
            generation  = excluded.generation,
            basis       = excluded.basis,
            item_count  = excluded.item_count,
            observed_at = excluded.observed_at",
        rusqlite::params![
            obs.project.0,
            enum_text(&obs.source)?,
            enum_text(&obs.outcome)?,
            obs.location.map(|l| l.0),
            obs.generation,
            obs.basis.as_ref().map(enum_text).transpose()?,
            obs.item_count,
            obs.observed_at,
        ],
    )?;
    Ok(())
}

/// The stored sweep for one `(project, source)`, or `None` if this source was **never observed**.
///
/// `None` is not an outcome. It is what makes a project with no row render as *not computed*
/// rather than as zero.
pub fn latest_sweep(
    tx: &Transaction<'_>,
    project: ProjectId,
    source: DebtSource,
) -> Result<Option<SweepObservation>, DebtError> {
    let source_text = enum_text(&source)?;
    let row = tx
        .query_row(
            "SELECT outcome, location_id, generation, basis, item_count, observed_at
               FROM debt_sweep WHERE project_id = ?1 AND source = ?2",
            rusqlite::params![project.0, source_text],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Option<i64>>(1)?,
                    r.get::<_, Option<i64>>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, Option<u32>>(4)?,
                    r.get::<_, i64>(5)?,
                ))
            },
        )
        .optional()?;
    let Some((outcome, location, generation, basis, item_count, observed_at)) = row else {
        return Ok(None);
    };
    let outcome: DebtSweepOutcome = enum_from_text(&outcome)
        .ok_or_else(|| DebtError::Codec(format!("debt_sweep.outcome holds {outcome:?}")))?;
    let basis = match basis {
        None => None,
        Some(raw) => Some(
            enum_from_text::<ObservationBasis>(&raw)
                .ok_or_else(|| DebtError::Codec(format!("debt_sweep.basis holds {raw:?}")))?,
        ),
    };
    Ok(Some(SweepObservation {
        project,
        source,
        outcome,
        location: location.map(LocationId),
        generation,
        basis,
        item_count,
        observed_at,
    }))
}
