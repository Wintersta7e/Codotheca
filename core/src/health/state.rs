//! §30.2 — the exclusion pipeline: **six gates, in one order, applied once.**
//!
//! Strict precedence `absent` > `suppressed` > `frozen` > `live`, and **the order is the ruling**:
//! each stage answers a question that makes the next meaningless if answered the other way.
//!
//! The freeze predicate is `crate::scan::presence::project_presence` and health expresses it
//! nowhere else — a second *"is this project offline"* is one value stated twice with a merge
//! conflict already built in.

use crate::protocol::{HealthState, Presence};
use crate::scan::presence::project_presence;

/// Everything §30.2's six gates read, and nothing else.
///
/// `locations` is the per-copy presence slice `project_presence` is *total* over; health does not
/// pre-roll it, because rolling it up here would be the second offline predicate.
///
/// `struct_excessive_bools` fires on the four, and collapsing them is the wrong fix: each is a
/// **separately observed fact** read by a different gate, and folding two into one enum would put
/// two gates behind one value — which is the *one value stated twice* defect arriving from the
/// other direction. They are named fields on a struct, never positional arguments, so the
/// confusion that lint exists to prevent cannot occur here.
#[derive(Debug, Clone)]
#[allow(clippy::struct_excessive_bools)]
pub struct StateInputs {
    /// §4.1a's classification. `None` is **not computed**, which gate 2 catches, not gate 1.
    pub is_reference: Option<bool>,
    /// `None` is *authorship has not run*. **Unclassified is not permission to compute.**
    pub authored_by_user: Option<bool>,
    /// §11.1's error kind, if the project carries one.
    pub error_kind: Option<String>,
    /// Whether any observation was ever recorded — the conjunct that keeps gate 3 from hiding a
    /// project that failed *after* it was read.
    pub ever_observed: bool,
    /// One entry per copy. Empty is *no locations*, which gate 4 answers.
    pub locations: Vec<Presence>,
    /// §30.5's predicate, from `super::enrolment::is_enrolled`.
    pub enrolled: bool,
    pub is_archived: bool,
    /// Whether a reading was ever computed for this project. **A freeze may never manufacture a
    /// zero**, so with no prior reading an offline project is `absent`, not `frozen`.
    pub prior_reading: bool,
}

/// §30.2's six gates, in precedence order.
///
/// Three consequences of using `project_presence` rather than a second predicate, each verified
/// against that function and each one an implementer gets wrong otherwise:
///
/// - **`Present` with some copies offline is `live`, not `frozen`.** `project_presence` returns
///   `Present` the moment any copy is present, and §5.1 aggregates over present copies. A copy
///   that cannot be read contributes nothing and does not freeze the copies that can.
/// - **`Missing` is not a freeze.** `offline` means the store is unreachable, which is what
///   justifies *current state under glass*; `missing` means the store **is** mounted and the
///   directory is not there, which is a change, not a pause.
/// - **`{offline, missing}` rolls up to `Missing` and `{offline, unscanned}` to `Unscanned`.**
///   Neither freezes; both are `absent`. §4.6's prose mentions neither case — the code is right
///   and the prose is under-specified.
#[must_use]
pub fn health_state(inputs: &StateInputs) -> HealthState {
    // 1. A Reference project is a judgement that never applied, not a withheld one.
    if inputs.is_reference == Some(true) {
        return HealthState::Absent;
    }
    // 2. A reading computed while authorship is uncomputed can score a repository about to be
    //    classified Reference, and the app would then have to **retract a reading it already
    //    showed** — under §28 possibly retracting XP, which `level_floor` forbids.
    if inputs.authored_by_user.is_none() {
        return HealthState::Absent;
    }
    // 3. An error with **no observation ever recorded**. The conjunct matters: a project that
    //    failed after it was read still has a reading, and hiding it would discard one.
    if inputs.error_kind.is_some() && !inputs.ever_observed {
        return HealthState::Absent;
    }
    // 4. Nothing to read from.
    let presence = project_presence(&inputs.locations);
    if inputs.locations.is_empty()
        || presence == Presence::Unscanned
        || presence == Presence::Missing
    {
        return HealthState::Absent;
    }
    // 5. §30.5's suppression: the user has not acknowledged the project, or has archived it.
    if !inputs.enrolled || inputs.is_archived {
        return HealthState::Suppressed;
    }
    // 6. **Freezing is not clearing**: a frozen reading keeps its value *and* its age. Freezing
    //    a reading that was never computed is forbidden — that case is gate 4's `absent`,
    //    because a freeze must never manufacture a zero.
    if presence == Presence::Offline {
        return if inputs.prior_reading {
            HealthState::Frozen
        } else {
            HealthState::Absent
        };
    }
    HealthState::Live
}
