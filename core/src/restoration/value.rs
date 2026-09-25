//! §34.2 — a layer's value, and the observation §30.1's precondition compares.
//!
//! **A layer's value is §33.2's: the count of open debt items mapped to it**, both scorings — a
//! `shown_only` item lights its layer exactly as a `scored` one does (A7). It is recomputable from
//! the item set, so a delta is a function of two observed readings and never a third stored
//! quantity.
//!
//! **The precondition is `crate::health::delta_gate`'s, adopted unchanged.** Each layer's end of a
//! candidate delta is a `delta_gate::Observation` — state, enrolment, layer, the eligible sources
//! that feed it, and the value — so the question *are these two numbers comparable* has exactly
//! one owner and this module holds no second eligibility notion.
//!
//! **Why a value is `None` so often, and why that is what makes the precondition hold.** Both
//! snapshots are taken inside ONE write transaction, so an enrolment, a switch toggle or a freeze
//! can never fall *between* them — each happens in a transaction of its own. What can happen is
//! that the evidence the first snapshot counts was observed on the far side of one. So a layer's
//! value is *not observed* whenever any source feeding it is not currently observed:
//!
//! - **no sweep row** — never observed, or a check switched off, which deletes the row (§30.9), so
//!   the first sweep after it comes back is a first observation and not a restoration;
//! - **a sweep that is neither `complete` nor `partial`** — a frozen root sweeps `unobservable`,
//!   so neither the freeze nor the unfreeze after it can be diffed across;
//! - **an `unverified` item** — `open → unverified` is not an improvement, because the store only
//!   stopped seeing the evidence, and `unverified → open` is not a regression (§28.10); the
//!   precondition refuses both rather than each being special-cased;
//! - **evidence observed before `acknowledged_at`** — it predates the reading (§30.5), so a delta
//!   from it would cross the enrolment boundary.
//!
//! Only the **eligible** sources are counted, because the precondition compares values over the
//! same eligible set: an item of a switched-off source is hidden, never closed, and must neither
//! animate nor count.

use std::collections::HashMap;

use rusqlite::Connection;

use crate::debt::store::{DebtCloseReason, DebtClosure};
use crate::debt::{enum_from_text, registry_for};
use crate::health::delta_gate::{delta_admissible, Observation};
use crate::index::IndexError;
use crate::projects::ProjectsError;
use crate::protocol::{
    CheckOutcome, DebtSource, DebtSweepOutcome, DecayLayer, HealthState, ProjectId,
};

/// One layer's end of a candidate delta.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayerObservation {
    /// The precondition's own shape. `value: None` is *not observed* — never a zero standing in
    /// for one.
    pub observation: Observation,
    /// A source feeding the layer was last swept `partial`. §27.5: a partial sweep may open and
    /// may never close, because an item it did not reach looks exactly like an item that is gone.
    pub partial: bool,
}

/// Every layer's observation for one project, in `DecayLayer::ALL` order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayerValues {
    layers: Vec<LayerObservation>,
}

/// One written transition. **Both ends are observed values** — the precondition guarantees it —
/// so neither is optional here; they become `f64` only at the column and on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayerTransition {
    /// The layer whose value moved.
    pub layer: DecayLayer,
    /// The open-item count in the first snapshot, before the write.
    pub from_value: u32,
    /// The open-item count in the second snapshot, after it.
    pub to_value: u32,
    /// Closures in this layer the user did not cause — §32's withdrawn advisories, closed
    /// `Invalidated`. Carried so the emit gate can tell a restoration from a retraction.
    pub not_user_caused: u32,
}

impl LayerValues {
    /// Every layer unobserved: the value for a project with no reading at all.
    #[must_use]
    pub fn unobserved() -> Self {
        Self {
            layers: DecayLayer::ALL
                .iter()
                .map(|&layer| LayerObservation {
                    observation: Observation {
                        state: HealthState::Absent,
                        enrolled: false,
                        layer,
                        eligible: Vec::new(),
                        value: None,
                    },
                    partial: false,
                })
                .collect(),
        }
    }

    /// Read one project's five observations through the caller's connection — inside a write
    /// transaction, that is the transaction's own view.
    ///
    /// A project that is gone — merged away while a job ran — has no reading and is every layer
    /// unobserved, which writes nothing rather than failing the caller's settle.
    ///
    /// # Errors
    /// Fails when the index refuses a read or holds a value this build's schema does not declare.
    pub fn read(conn: &Connection, project: ProjectId) -> Result<Self, IndexError> {
        let reading = match crate::health::reading_for_project(conn, project) {
            Ok(reading) => reading,
            Err(ProjectsError::UnknownProject(_)) => {
                return Ok(Self::unobserved());
            }
            Err(other) => return Err(projects_error(other)),
        };
        let acknowledged_at: Option<i64> = conn.query_row(
            "SELECT acknowledged_at FROM project WHERE id = ?1",
            [project.0],
            |r| r.get(0),
        )?;
        let sweeps = sweeps_of(conn, project)?;
        let counts = counts_of(conn, project)?;

        // `eligible = ran + unknown` (§30.3): a check is eligible iff it produced `ok`, `failed`
        // or `unknown`. `off` and `notApplicable` are outside it, and `absent`/`suppressed`
        // readings carry no checks at all.
        let eligible_all: Vec<DebtSource> = reading
            .checks
            .iter()
            .filter(|c| {
                matches!(
                    c.outcome,
                    CheckOutcome::Ok | CheckOutcome::Failed | CheckOutcome::Unknown
                )
            })
            .map(|c| c.id)
            .collect();

        let layers = DecayLayer::ALL
            .iter()
            .map(|&layer| {
                // The reading's checks are in `DebtSource::ALL` order, so this is a stable order.
                let eligible: Vec<DebtSource> = eligible_all
                    .iter()
                    .copied()
                    .filter(|s| registry_for(*s).layer == layer)
                    .collect();
                let partial = eligible.iter().any(|s| {
                    sweeps.get(s).map(|(outcome, _)| *outcome) == Some(DebtSweepOutcome::Partial)
                });
                let value =
                    observed_value(reading.state, acknowledged_at, &eligible, &sweeps, &counts);
                LayerObservation {
                    observation: Observation {
                        state: reading.state,
                        enrolled: crate::health::enrolment::is_enrolled(acknowledged_at),
                        layer,
                        eligible,
                        value,
                    },
                    partial,
                }
            })
            .collect();
        Ok(Self { layers })
    }

    /// One layer's observation.
    #[must_use]
    pub fn layer(&self, layer: DecayLayer) -> Option<&LayerObservation> {
        self.layers.iter().find(|l| l.observation.layer == layer)
    }

    /// The transitions from `self` to `after` that may be written.
    ///
    /// Per layer, so there is at most one transition per layer per event: two rows for one layer
    /// would make the origin ambiguous and A3's tie-break undecidable. A layer is skipped when
    /// the precondition refuses the pair, when nothing changed, and — for a **decrease** — when
    /// the newer sweep was `partial`.
    #[must_use]
    pub fn diff(&self, after: &Self, closed: &[DebtClosure]) -> Vec<LayerTransition> {
        let mut out = Vec::new();
        for (from, to) in self.layers.iter().zip(&after.layers) {
            // §30.1: *"writes no row on `Err`"*.
            if delta_admissible(&from.observation, &to.observation).is_err() {
                continue;
            }
            let (Some(from_value), Some(to_value)) = (from.observation.value, to.observation.value)
            else {
                continue;
            };
            if from_value == to_value {
                continue;
            }
            if to_value < from_value && to.partial {
                continue;
            }
            let layer = to.observation.layer;
            let not_user_caused = closed
                .iter()
                .filter(|c| {
                    c.reason == DebtCloseReason::Invalidated
                        && registry_for(c.key.source).layer == layer
                })
                .count();
            out.push(LayerTransition {
                layer,
                from_value,
                to_value,
                not_user_caused: u32::try_from(not_user_caused).unwrap_or(u32::MAX),
            });
        }
        out
    }
}

/// The count over the eligible sources, or `None` when any of them is not currently observed.
/// The module doc gives each refusal's reason.
fn observed_value(
    state: HealthState,
    acknowledged_at: Option<i64>,
    eligible: &[DebtSource],
    sweeps: &HashMap<DebtSource, (DebtSweepOutcome, i64)>,
    counts: &HashMap<DebtSource, (u32, u32)>,
) -> Option<u32> {
    if matches!(state, HealthState::Absent | HealthState::Suppressed) || eligible.is_empty() {
        return None;
    }
    let mut open = 0u32;
    for source in eligible {
        let (outcome, observed_at) = sweeps.get(source)?;
        if !matches!(
            outcome,
            DebtSweepOutcome::Complete | DebtSweepOutcome::Partial
        ) {
            return None;
        }
        if acknowledged_at.is_some_and(|at| *observed_at < at) {
            return None;
        }
        let (source_open, unverified) = counts.get(source).copied().unwrap_or((0, 0));
        if unverified > 0 {
            return None;
        }
        open = open.saturating_add(source_open);
    }
    Some(open)
}

fn sweeps_of(
    conn: &Connection,
    project: ProjectId,
) -> Result<HashMap<DebtSource, (DebtSweepOutcome, i64)>, IndexError> {
    let mut st =
        conn.prepare("SELECT source, outcome, observed_at FROM debt_sweep WHERE project_id = ?1")?;
    let mut rows = st.query([project.0])?;
    let mut out = HashMap::new();
    while let Some(r) = rows.next()? {
        let source: String = r.get(0)?;
        let outcome: String = r.get(1)?;
        out.insert(
            decode::<DebtSource>("debt_sweep.source", &source)?,
            (
                decode::<DebtSweepOutcome>("debt_sweep.outcome", &outcome)?,
                r.get(2)?,
            ),
        );
    }
    Ok(out)
}

/// Source → (open items, unverified items), both scorings.
fn counts_of(
    conn: &Connection,
    project: ProjectId,
) -> Result<HashMap<DebtSource, (u32, u32)>, IndexError> {
    let mut st = conn.prepare(
        "SELECT source, sum(state = 'open'), sum(state = 'unverified')
           FROM debt_item WHERE project_id = ?1 GROUP BY source",
    )?;
    let mut rows = st.query([project.0])?;
    let mut out = HashMap::new();
    while let Some(r) = rows.next()? {
        let source: String = r.get(0)?;
        let open = u32::try_from(r.get::<_, i64>(1)?).unwrap_or(u32::MAX);
        let unverified = u32::try_from(r.get::<_, i64>(2)?).unwrap_or(u32::MAX);
        out.insert(
            decode::<DebtSource>("debt_item.source", &source)?,
            (open, unverified),
        );
    }
    Ok(out)
}

fn decode<T: serde::de::DeserializeOwned>(column: &str, raw: &str) -> Result<T, IndexError> {
    enum_from_text(raw).ok_or_else(|| IndexError::Corrupt {
        detail: format!("{column} holds {raw:?}"),
    })
}

fn projects_error(error: ProjectsError) -> IndexError {
    match error {
        ProjectsError::Index(inner) => inner,
        ProjectsError::Sqlite(inner) => IndexError::Sqlite(inner),
        other => IndexError::Corrupt {
            detail: other.to_string(),
        },
    }
}
