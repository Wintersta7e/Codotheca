//! §34.2's row writer, §34.4's emit gate, and `detected_in`'s provenance.

use rusqlite::Transaction;

use super::value::{LayerTransition, LayerValues};
use crate::debt::enum_text;
use crate::debt::identity::DebtKey;
use crate::debt::store::DebtCloseReason;
use crate::index::IndexError;
use crate::jobs::JobOrigin;
use crate::proto::EventSink;
use crate::protocol::{HealthDetectedIn, HealthLayerDelta, ProjectHealthDelta, ProjectId};

/// Write one `health_delta` row per transition `before.diff(after)` admits, in the caller's
/// transaction, and return what was written.
///
/// **Inside the caller's transaction or not at all**: a delta committed beside a debt set that
/// was not is a history of a state that never existed.
///
/// # Errors
/// Fails when the index refuses the write.
pub fn record_layer_deltas(
    tx: &Transaction<'_>,
    project: ProjectId,
    before: &LayerValues,
    after: &LayerValues,
    closed: &[(DebtKey, DebtCloseReason)],
    detected_in: HealthDetectedIn,
    now: i64,
) -> Result<Vec<LayerTransition>, IndexError> {
    let written = before.diff(after, closed);
    if written.is_empty() {
        return Ok(written);
    }
    let provenance = text(&detected_in)?;
    for transition in &written {
        tx.execute(
            "INSERT INTO health_delta (project_id, ts, layer, from_value, to_value, detected_in)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                project.0,
                now,
                text(&transition.layer)?,
                f64::from(transition.from_value),
                f64::from(transition.to_value),
                provenance,
            ],
        )?;
    }
    Ok(written)
}

/// The transitions the event may carry.
///
/// §28.10 left §34 one question — *does an `invalidated` decrease originate a surge* — and this is
/// the answer (R135): **a layer whose whole decrease is closures the user did not cause is
/// written and never announced.** A withdrawn advisory is a third party changing its mind, not a
/// restoration, and the surge is a reward; the row still records that the value moved. An
/// increase is always kept, and a decrease only partly withdrawn is kept with its true values —
/// no value is fabricated to shave the withdrawn part off.
#[must_use]
pub fn emittable_layers(written: &[LayerTransition]) -> Vec<LayerTransition> {
    written
        .iter()
        .filter(|t| t.to_value >= t.from_value || t.not_user_caused < t.from_value - t.to_value)
        .copied()
        .collect()
}

/// The event for `emittable`, or `None` when there is nothing to announce — **an empty `layers`
/// is no event at all**, never an event saying nothing happened.
#[must_use]
pub fn health_delta_event(
    project: ProjectId,
    ts: i64,
    detected_in: HealthDetectedIn,
    emittable: &[LayerTransition],
) -> Option<ProjectHealthDelta> {
    if emittable.is_empty() {
        return None;
    }
    Some(ProjectHealthDelta {
        id: project,
        ts,
        detected_in,
        layers: emittable
            .iter()
            .map(|t| HealthLayerDelta {
                layer: t.layer,
                from_value: Some(f64::from(t.from_value)),
                to_value: Some(f64::from(t.to_value)),
            })
            .collect(),
    })
}

/// The call every debt-set writer makes after its write: read the second snapshot, write the
/// rows, and return the event to announce **once the transaction has committed** — never inside
/// it, where a rollback could still remove the state it announces.
///
/// `before` is the caller's first snapshot, taken before its write. Reading only after the write
/// would leave no `before` to recover, and a producer that invented one would invent the history.
///
/// # Errors
/// Fails when the index refuses the read or the write.
pub fn record_after_write(
    tx: &Transaction<'_>,
    project: ProjectId,
    before: &LayerValues,
    closed: &[(DebtKey, DebtCloseReason)],
    detected_in: HealthDetectedIn,
    now: i64,
) -> Result<Option<ProjectHealthDelta>, IndexError> {
    let after = LayerValues::read(tx, project)?;
    let written = record_layer_deltas(tx, project, before, &after, closed, detected_in, now)?;
    Ok(health_delta_event(
        project,
        now,
        detected_in,
        &emittable_layers(&written),
    ))
}

/// `projects.health_delta`, on the existing topic.
pub fn emit_health_delta(events: &dyn EventSink, delta: &ProjectHealthDelta) {
    if let Ok(payload) = serde_json::to_value(delta) {
        events.emit("projects", "health_delta", payload);
    }
}

/// **The provenance of the job that observed the change, not the user's attention** (§34.2).
///
/// A chain a user started — opening a page, Peek, the rescan that follows the window regaining
/// focus — is `foreground`; a walk-queued chain is `background`. **`JobOrigin`, not `Priority`**:
/// `projects.get` → `on_visible` queues J7 at `Standard`, not `Interactive`, so a priority mapping
/// would record the payoff path itself — close a TODO, reopen the page — as `background`.
#[must_use]
pub const fn detected_in_for(origin: JobOrigin) -> HealthDetectedIn {
    match origin {
        JobOrigin::Interactive => HealthDetectedIn::Foreground,
        JobOrigin::Walk => HealthDetectedIn::Background,
    }
}

fn text<T: serde::Serialize>(value: &T) -> Result<String, IndexError> {
    enum_text(value).map_err(|e| IndexError::Corrupt {
        detail: e.to_string(),
    })
}
