//! §30.1's closing paragraph — **`health_delta`'s write precondition, which §34 may not relax.**
//!
//! A `health_delta` row may be written **only when `from_value` and `to_value` are both values
//! the app observed, of the same layer, over the same `eligible` set.** Therefore **no delta may
//! cross an enrolment boundary, a freeze, an unfreeze, a switch toggle, or a first observation**
//! — otherwise the first successful sweep after a check is enabled plays a full restoration for
//! work the user did not do.
//!
//! §34 decides what an observed delta *earns*; **the precondition is not its to widen.** It is
//! enforced core-side, in the `completion_lit` shape (§1.10), and **not in the view layer, which
//! cannot enforce a write rule.**
//!
//! # The call site §34 must use
//!
//! **`§34`'s `health_delta` producer calls [`delta_admissible`] and writes no row on `Err`.**
//! `health_delta` has no producer until p3-34 (wave 5) and its table is not rebuilt until
//! migration `0016`, so no row can be written or withheld in this worktree: this module lands the
//! **predicate** and drives all five boundaries through it, and the row-count half of
//! `AC-P3-30-18` lands with p3-34's producer under the same tag.
//!
//! A predicate with no caller is R1's defect — a trait declared for testability that gets its
//! fake and never its real implementation — so the call site is **named here** rather than
//! discovered, and p3-34 must fail if it does not use it.

use crate::protocol::{DecayLayer, HealthState};

/// Why a delta may not be written.
///
/// Five boundaries, each one a case where the two endpoints are not two readings of the same
/// thing. They are separate variants because §34 renders a different restoration for each, and a
/// single *"not comparable"* would make them one sentence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeltaBoundary {
    /// The project became enrolled, or stopped being. Before enrolment there was no reading to
    /// move from.
    Enrolment,
    /// The store went away. A frozen reading keeps its value **and** its age; it did not change.
    Freeze,
    /// The store came back. The first reading after a freeze is a new observation, not the far
    /// end of an old one.
    Unfreeze,
    /// The eligible set moved — a check switched on or off, or the two endpoints are about
    /// different layers. **Either way the two numbers count different things**, and a delta
    /// between them is a measurement of the switch rather than of the project.
    SwitchToggle,
    /// There was no *from* value. **A first observation is not an improvement**, and treating it
    /// as one plays a full restoration for work the user did not do.
    FirstObservation,
}

/// One end of a candidate delta.
///
/// `layer` rides the endpoint rather than the call, so two endpoints about different layers are
/// **representable and refusable** instead of being a caller contract nothing checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Observation {
    /// The reading's state when this endpoint was observed.
    pub state: HealthState,
    /// Whether the project was enrolled when this endpoint was observed.
    pub enrolled: bool,
    /// Which layer this endpoint measures.
    pub layer: DecayLayer,
    /// The switches that were **eligible** when this endpoint was observed, in a stable order.
    /// Two different sets are two different quantities.
    pub eligible: Vec<crate::protocol::DebtSource>,
    /// The value observed, or `None` when nothing was. **Never a zero standing in for one.**
    pub value: Option<u32>,
}

/// Whether a delta between these two endpoints may be written.
///
/// **The outermost boundary is the one named.** Enrolment outranks a freeze because a project
/// that was not enrolled had no reading to freeze; a freeze outranks a switch toggle because a
/// frozen endpoint was not observed at all.
///
/// # Errors
/// One [`DeltaBoundary`] per refusal. §34's producer writes no row on `Err`.
pub fn delta_admissible(from: &Observation, to: &Observation) -> Result<(), DeltaBoundary> {
    if from.enrolled != to.enrolled {
        return Err(DeltaBoundary::Enrolment);
    }
    if from.state != HealthState::Frozen && to.state == HealthState::Frozen {
        return Err(DeltaBoundary::Freeze);
    }
    if from.state == HealthState::Frozen && to.state != HealthState::Frozen {
        return Err(DeltaBoundary::Unfreeze);
    }
    if from.layer != to.layer || from.eligible != to.eligible {
        return Err(DeltaBoundary::SwitchToggle);
    }
    if from.value.is_none() {
        return Err(DeltaBoundary::FirstObservation);
    }
    if to.value.is_none() {
        // The far end was not observed either, which is the same refusal from the other side: a
        // delta needs two values the app observed and this pair has one.
        return Err(DeltaBoundary::FirstObservation);
    }
    Ok(())
}
