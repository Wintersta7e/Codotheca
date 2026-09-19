//! §30.8 — **the lifecycle verdict nobody computed**, in one core-owned tri-state evaluator.
//!
//! §35 excludes Done projects, §33 delegates Done filtering here, and §30.1's four states compute
//! no Done at all. It is computed once, beside the debt list, and expressed nowhere else.
//!
//! **`ProjectLifecycle` is a new enum and is NOT a variant added to `ConditionSignal`.** The
//! stored column `condition_material` is `Option<ConditionSignal>` over seven **activity** bands;
//! adding `done` there would put a lifecycle verdict into an activity vocabulary and add a
//! meaningless variant to `condition_signal`, which shares the enum.
//!
//! **The evaluator is a live measurement, not an earned thing** (A12). It may demote silently: no
//! animation, no delta row, no XP reversal, no notification.
//!
//! **Lifecycle drives no layer and no decay.** A project that is not `done` is not thereby
//! decayed — decay is the debt list (§33). Without that guard the activity input re-introduces
//! staleness-driven decay through the back door, which is the largest recorded error.

use crate::protocol::{ConditionSignal, HealthReading, HealthState, ProjectLifecycle};

/// §30.8's three values.
///
/// **`done` requires positive evidence from a `live` reading.** A project whose reading is
/// `absent`, `suppressed` or `frozen` is never `done` — the first conjunct is *unknown*, not
/// satisfied. **`frozen` included**: a freeze must never manufacture a verdict any more than it
/// manufactures a zero.
///
/// **The three extra conjuncts §30.8 also asked for are dropped** (R131/F10). *"HEAD clean and
/// pushed, or a tagged release; no red CI"* names `unpushed_commits`, `no_release` and `ci_red` —
/// **three of A4's nine debt sources** — so *"every eligible check `ok`"* already carries all
/// three. Keeping them reads one fact through two vocabularies, which is the defect A17 and A12b
/// were written against, inside the evaluator A16.1 created to stop Done being computed twice.
/// They also diverge the moment one switch is off: switch `unpushed_commits` off and it leaves
/// `eligible`, *"every eligible check `ok`"* holds, and the separate conjunct still demands
/// *pushed* from a producer §30 never named. A switch turned off honestly makes Done
/// unreachable-by-that-route rather than silently satisfied.
///
/// **"No recent activity" is an input to the classification and never a debt item** (A2). Its only
/// role is a **hold**: a project whose `condition_signal` is `live` or `idle` stays `active`
/// whatever its tree says, because a project being worked on this month is not finished. It never
/// opens an item, never enters a basis term, and never moves `scoredOpen`.
///
/// `active` is **the residual and carries no claim** — only *not archived and not provably done*.
#[must_use]
pub fn lifecycle_of(
    reading: &HealthReading,
    is_archived: bool,
    condition_signal: Option<ConditionSignal>,
) -> ProjectLifecycle {
    // Declared, never derived.
    if is_archived {
        return ProjectLifecycle::Archived;
    }
    // A project being worked on this month is not finished, whatever its tree says.
    if matches!(
        condition_signal,
        Some(ConditionSignal::Live | ConditionSignal::Idle)
    ) {
        return ProjectLifecycle::Active;
    }
    if reading.state != HealthState::Live {
        return ProjectLifecycle::Active;
    }
    // Every eligible check `ok` — which is `ran == eligible` with nothing scored open. A reading
    // with no basis has no eligible set to be complete over, so it cannot satisfy this.
    let Some(basis) = reading.basis.as_ref() else {
        return ProjectLifecycle::Active;
    };
    if basis.ran == basis.eligible && basis.ran > 0 && reading.scored_open == Some(0) {
        ProjectLifecycle::Done
    } else {
        ProjectLifecycle::Active
    }
}
