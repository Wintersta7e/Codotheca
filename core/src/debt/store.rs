//! §28.3 — the item store: open, refresh, close, reap.

use std::collections::HashSet;

use rusqlite::Transaction;

use super::identity::DebtKey;
use super::sweep::{comparable, may_close, upsert_sweep, SweepObservation};
use super::{enum_from_text, enum_text, DebtError};
use crate::projects::rows::{locations_of, pick_primary};
use crate::protocol::{
    DebtItemState, DebtScoring, DebtSource, DebtSweepOutcome, LocationId, ObservationBasis,
    ProjectId,
};

/// One `debt_item` row as read back.
///
/// `layer` is deliberately **not** here and is not a column: it is a property of the item's
/// source, read from §28.2's registry, so the wire's `DebtItem.layer` is a join and not a stored
/// value that could drift from the source it describes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredItem {
    pub id: i64,
    pub key: DebtKey,
    pub state: DebtItemState,
    /// Set from the source's registry row and **overridable per item by the producer** — an
    /// advisory is `scored` when a fix is available and `shown_only` when one is not.
    pub scoring: DebtScoring,
    /// The anchor. **READ, not diagnostic**: [`super::sweep::comparable`] compares it against the
    /// sweep's location before any closure (§28.3 rule 1).
    pub last_seen_location_id: Option<LocationId>,
    pub basis: Option<ObservationBasis>,
    pub path_display: Option<String>,
    pub line: Option<u32>,
    pub column: Option<u32>,
    pub salient_text: Option<String>,
    pub first_seen_at: i64,
    pub last_seen_at: i64,
}

/// What a producer saw, ready to be opened or refreshed.
///
/// **`layer` is not here**, deliberately and against an earlier draft of this shape: the layer is
/// a pure function of `source` through §28.2's registry, so carrying it on the observation would
/// be one value in two places (R12) and the second copy could disagree with the first. `scoring`
/// **is** here, because §32 overrides it per item — an advisory is `scored` when a fix exists and
/// `shown_only` when one does not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservedItem {
    pub key: DebtKey,
    pub scoring: DebtScoring,
    pub location: Option<LocationId>,
    pub basis: Option<ObservationBasis>,
    /// Arbitrary bytes; `path_display` beside it is lossy and for the UI only.
    pub path_bytes: Option<Vec<u8>>,
    pub path_display: Option<String>,
    pub line: Option<u32>,
    pub column: Option<u32>,
    pub salient_text: Option<String>,
}

/// Why an item stopped existing.
///
/// **Two reasons, never one.** `Fixed` is a transition the user performed and enters the day's
/// payout; `Invalidated` is a third party retracting the evidence — a withdrawn advisory — and
/// **pays nothing**. **It is not a column**: the row is deleted, so there is nowhere to store it
/// and nothing that could read it back. XP already paid is never clawed back.
///
/// **Nothing in §28 produces `Invalidated`**, and that is a statement about wave 2 rather than a
/// gap: every source §28 sweeps is evidence the *user* controls, so its disappearance is the user
/// having acted. §32 owns the advisory feed and is the only producer that can tell a withdrawal
/// from an upgrade; it builds the [`SweepEffect`] carrying this variant, and [`pay_debt_day`]
/// already excludes it.
///
/// [`pay_debt_day`]: crate::debt::xp::pay_debt_day
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DebtCloseReason {
    Fixed,
    Invalidated,
}

/// What one `observe` changed — the input the XP writer and §34 both read.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SweepEffect {
    pub opened: Vec<DebtKey>,
    pub closed: Vec<(DebtKey, DebtCloseReason)>,
    pub refreshed: u32,
    pub unverified: u32,
}

/// §28.3's four operations.
///
/// **The trait and its production implementation land together**, which is R1's rule and five
/// recorded failures: a trait declared for testability gets its fake and never its real impl,
/// compiles, passes against the fake, and fails at assembly.
pub trait DebtStore: Send + Sync {
    /// Diff one source's observation against what is stored, writing the sweep row with it.
    fn observe(
        &self,
        tx: &Transaction<'_>,
        obs: &SweepObservation,
        seen: &[ObservedItem],
    ) -> Result<SweepEffect, DebtError>;

    /// Delete `unverified` items whose anchor is gone and which this sweep did not re-observe.
    ///
    /// **A reap is not a closure**: no XP, no close reason, no layer movement.
    fn reap(
        &self,
        tx: &Transaction<'_>,
        project: ProjectId,
        obs: &SweepObservation,
    ) -> Result<u32, DebtError>;

    /// Mark every open item of this project `unverified`, in the caller's transaction.
    fn mark_unverified(&self, tx: &Transaction<'_>, project: ProjectId) -> Result<u32, DebtError>;
}

/// The production implementation.
#[derive(Debug, Clone, Copy, Default)]
pub struct SqliteDebtStore;

impl DebtStore for SqliteDebtStore {
    fn observe(
        &self,
        tx: &Transaction<'_>,
        obs: &SweepObservation,
        seen: &[ObservedItem],
    ) -> Result<SweepEffect, DebtError> {
        upsert_sweep(tx, obs)?;

        let stored = stored_items(tx, obs.project, obs.source)?;
        let seen_keys: HashSet<&DebtKey> = seen.iter().map(|o| &o.key).collect();
        let mut effect = SweepEffect::default();

        for item in seen {
            if stored.iter().any(|s| s.key == item.key) {
                refresh(tx, obs.project, item, obs.observed_at)?;
                effect.refreshed = effect.refreshed.saturating_add(1);
            } else {
                open(tx, obs.project, item, obs.observed_at)?;
                effect.opened.push(item.key.clone());
            }
        }

        for item in &stored {
            if seen_keys.contains(&item.key) {
                continue;
            }
            if may_close(item, obs) {
                // Every source §28 sweeps is evidence the user controls, so its disappearance is
                // the user having acted. See `DebtCloseReason`.
                tx.execute("DELETE FROM debt_item WHERE id = ?1", [item.id])?;
                effect
                    .closed
                    .push((item.key.clone(), DebtCloseReason::Fixed));
            } else if comparable(item, obs) && obs.outcome == DebtSweepOutcome::Partial {
                // A budget cut-off at the item's own anchor and basis observed the root; it
                // simply did not finish. The item is still believed present, and marking it
                // unverified would drop it out of the health count on every budget-limited run.
            } else if item.state == DebtItemState::Open {
                tx.execute(
                    "UPDATE debt_item SET state = 'unverified' WHERE id = ?1",
                    [item.id],
                )?;
                effect.unverified = effect.unverified.saturating_add(1);
            }
        }

        Ok(effect)
    }

    fn reap(
        &self,
        tx: &Transaction<'_>,
        project: ProjectId,
        obs: &SweepObservation,
    ) -> Result<u32, DebtError> {
        if obs.outcome != DebtSweepOutcome::Complete {
            return Ok(0);
        }
        let locations = locations_of(tx, project)
            .map_err(|e| DebtError::Codec(format!("locations_of: {e}")))?;
        let Some(primary) = pick_primary(&locations).map(|l| l.id) else {
            return Ok(0);
        };
        if obs.location != Some(primary) {
            return Ok(0);
        }

        // An item with no anchor never had one to lose — `abandoned_with_debt` observes no root
        // — so it is not stranded and is not reaped.
        let deleted = tx.execute(
            "DELETE FROM debt_item
              WHERE project_id = ?1
                AND state = 'unverified'
                AND last_seen_location_id IS NOT NULL
                AND last_seen_location_id <> ?2
                AND NOT EXISTS (SELECT 1 FROM location
                                 WHERE id = debt_item.last_seen_location_id
                                   AND removed_at IS NULL)",
            rusqlite::params![project.0, primary.0],
        )?;
        Ok(u32::try_from(deleted).unwrap_or(u32::MAX))
    }

    fn mark_unverified(&self, tx: &Transaction<'_>, project: ProjectId) -> Result<u32, DebtError> {
        let marked = tx.execute(
            "UPDATE debt_item SET state = 'unverified'
              WHERE project_id = ?1 AND state = 'open'",
            [project.0],
        )?;
        Ok(u32::try_from(marked).unwrap_or(u32::MAX))
    }
}

/// Every stored item of one `(project, source)`.
fn stored_items(
    tx: &Transaction<'_>,
    project: ProjectId,
    source: DebtSource,
) -> Result<Vec<StoredItem>, DebtError> {
    let source_text = enum_text(&source)?;
    let mut st = tx.prepare(
        "SELECT id, subject_key, fingerprint, state, scoring, last_seen_location_id, basis,
                path_display, line, column, salient_text, first_seen_at, last_seen_at
           FROM debt_item WHERE project_id = ?1 AND source = ?2
          ORDER BY id",
    )?;
    let rows = st.query_map(rusqlite::params![project.0, source_text], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, String>(4)?,
            r.get::<_, Option<i64>>(5)?,
            r.get::<_, Option<String>>(6)?,
            r.get::<_, Option<String>>(7)?,
            r.get::<_, Option<u32>>(8)?,
            r.get::<_, Option<u32>>(9)?,
            r.get::<_, Option<String>>(10)?,
            r.get::<_, i64>(11)?,
            r.get::<_, i64>(12)?,
        ))
    })?;

    let mut out = Vec::new();
    for row in rows {
        let row = row?;
        out.push(StoredItem {
            id: row.0,
            key: DebtKey {
                subject_key: row.1,
                source,
                fingerprint: row.2,
            },
            state: enum_from_text(&row.3)
                .ok_or_else(|| DebtError::Codec(format!("debt_item.state holds {:?}", row.3)))?,
            scoring: enum_from_text(&row.4)
                .ok_or_else(|| DebtError::Codec(format!("debt_item.scoring holds {:?}", row.4)))?,
            last_seen_location_id: row.5.map(LocationId),
            basis: match row.6 {
                None => None,
                Some(raw) => {
                    Some(enum_from_text::<ObservationBasis>(&raw).ok_or_else(|| {
                        DebtError::Codec(format!("debt_item.basis holds {raw:?}"))
                    })?)
                }
            },
            path_display: row.7,
            line: row.8,
            column: row.9,
            salient_text: row.10,
            first_seen_at: row.11,
            last_seen_at: row.12,
        });
    }
    Ok(out)
}

fn open(
    tx: &Transaction<'_>,
    project: ProjectId,
    item: &ObservedItem,
    now: i64,
) -> Result<(), DebtError> {
    tx.execute(
        "INSERT INTO debt_item
            (project_id, subject_key, source, fingerprint, state, scoring,
             last_seen_location_id, basis, path_bytes, path_display, line, column, salient_text,
             first_seen_at, last_seen_at)
         VALUES (?1, ?2, ?3, ?4, 'open', ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?13)",
        rusqlite::params![
            project.0,
            item.key.subject_key,
            enum_text(&item.key.source)?,
            item.key.fingerprint,
            enum_text(&item.scoring)?,
            item.location.map(|l| l.0),
            item.basis.as_ref().map(enum_text).transpose()?,
            item.path_bytes,
            item.path_display,
            item.line,
            item.column,
            item.salient_text,
            now,
        ],
    )?;
    Ok(())
}

/// **Never `fingerprint`, and never `first_seen_at`.** That is what makes a rename and a line
/// move close nothing: the attributes move and the identity does not.
///
/// `state` returns to `open`, because an item this sweep just saw is verified by definition.
fn refresh(
    tx: &Transaction<'_>,
    project: ProjectId,
    item: &ObservedItem,
    now: i64,
) -> Result<(), DebtError> {
    tx.execute(
        "UPDATE debt_item
            SET state = 'open',
                scoring = ?4,
                last_seen_location_id = ?5,
                basis = ?6,
                path_bytes = ?7,
                path_display = ?8,
                line = ?9,
                column = ?10,
                salient_text = ?11,
                last_seen_at = ?12
          WHERE project_id = ?1 AND source = ?2 AND fingerprint = ?3",
        rusqlite::params![
            project.0,
            enum_text(&item.key.source)?,
            item.key.fingerprint,
            enum_text(&item.scoring)?,
            item.location.map(|l| l.0),
            item.basis.as_ref().map(enum_text).transpose()?,
            item.path_bytes,
            item.path_display,
            item.line,
            item.column,
            item.salient_text,
            now,
        ],
    )?;
    Ok(())
}
