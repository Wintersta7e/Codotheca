//! §30.3 — the five check outcomes, the basis over them, and the oldest input.
//!
//! **A health check is a debt source** (A17) — §28's closed registry, *not* the ten completion
//! checks. §30 mirrors neither the registry's membership nor its size: **no number for it appears
//! in this module, in any schema type, or in any test this plan owns.**
//!
//! `ran = ok + failed`. `eligible = ran + unknown`. **`ran` is the only denominator this system
//! has, and a check enters it only by producing `ok` or `failed`** — which is the single mechanism
//! that makes it impossible to report a suppressed, switched-off or failed check as a passing one.
//! `off`, `notApplicable` and `unknown` are disjoint fields and **no arithmetic path may move a
//! value into `ran`**.

use crate::protocol::{CheckOutcome, DebtSweepOutcome, HealthBasis, HealthCheck};

/// What §28's store says about one source on one project.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SweepFacts {
    /// `None` is **no sweep row at all** — this source was never observed, which is a different
    /// fact from a sweep that finished and found nothing.
    pub outcome: Option<DebtSweepOutcome>,
    /// Items that are both `open` (§28's state) **and** `scored` (§28's `scoring`, per A7). A
    /// `shown_only` item renders, lights its layer, ranks nowhere, and is counted by neither this
    /// field nor any XP payout.
    pub scored_open: u32,
    /// §28's second stored state. **They are what makes a check `unknown`**, which is why `ok`
    /// carries the conjunct below.
    pub unverified: u32,
    /// `debt_sweep.observed_at`, or `None` when there is no row.
    pub observed_at: Option<i64>,
}

/// Why a check might not be evaluated at all — the two outcomes that are outside `eligible`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SwitchState {
    /// §30.9's per-check switch. An absent key is **on**.
    pub enabled: bool,
    /// **R128/F8.** The check's evidence needs a grant the user has not given, which makes the
    /// outcome `off` and not `unknown`: `off` already means *the user chose not to have this* and
    /// is already outside `eligible`, while `unknown` with reason `notRunYet` would tell the user
    /// to wait for a sweep that never comes and inflate `eligible` with a check that cannot be
    /// evaluated.
    pub grant_missing: bool,
    /// Marked not-applicable for this project through §31's archetype-proposed mechanism.
    pub not_applicable: bool,
}

/// §30.3's table, total over the five outcomes.
///
/// **The switch is tested before not-applicable**, because it is the user's own explicit
/// statement about this check and *"a user who switched a check off is owed a different sentence
/// from one whose repo never needed it"* (§30.9). The two are separate causes and the more
/// specific one wins.
///
/// **`ok` is the only outcome requiring a complete observation.** A `partial` sweep with zero
/// items is `unknown`, never `ok`: an item it did not reach looks exactly like an item that is
/// gone. A `partial` sweep that *did* find a scored open item is `failed` — an item observed is
/// an item, and the under-claim risk is zero.
#[must_use]
pub fn outcome_for(facts: &SweepFacts, switch: &SwitchState) -> CheckOutcome {
    if !switch.enabled || switch.grant_missing {
        return CheckOutcome::Off;
    }
    if switch.not_applicable {
        return CheckOutcome::NotApplicable;
    }
    if facts.scored_open > 0 {
        return CheckOutcome::Failed;
    }
    // **The third conjunct is R128/F11 and it is load-bearing.** Without it a `complete` sweep
    // with zero scored open items but some `unverified` items matches *both* `ok` and `unknown`,
    // and the overlap decides `CheckOutcome` for a common case — and therefore `ran`, `eligible`
    // and every figure over them.
    if facts.outcome == Some(DebtSweepOutcome::Complete) && facts.unverified == 0 {
        return CheckOutcome::Ok;
    }
    CheckOutcome::Unknown
}

/// §30.3's *In `eligible`?* column: `ok`, `failed` and `unknown` are in it.
///
/// `off` and `notApplicable` are not. **The one expression of that column** — the basis counts
/// over it and the producer's `eligible = 0` gate reads it, so the two cannot disagree about a
/// check.
#[must_use]
pub const fn in_eligible(outcome: CheckOutcome) -> bool {
    // Exhaustive, with no `_ =>` arm: a sixth outcome fails to compile here rather than landing
    // silently on either side.
    match outcome {
        CheckOutcome::Ok | CheckOutcome::Failed | CheckOutcome::Unknown => true,
        CheckOutcome::Off | CheckOutcome::NotApplicable => false,
    }
}

/// One check as the basis counts it: the outcome, and when the evidence behind it was read.
#[derive(Debug, Clone)]
pub struct CheckObservation {
    /// The check's wire row: its source, its outcome, and why it is `unknown` when it is.
    pub check: HealthCheck,
    /// `None` when this check has no sweep row — never a zero, which would date a reading from
    /// the epoch.
    pub observed_at: Option<i64>,
}

/// §30.3's basis over a project's checks.
///
/// **`observedAt` is the oldest input, not the newest**: a reading is only as current as its
/// stalest input, and dating it by the freshest read is the staleness marker lying.
///
/// Two clauses, and the second is not a fallback bolted on:
///
/// - with `ran > 0` it is the oldest observation among the checks that **entered `ran`**, which
///   are the checks whose evidence produced a verdict;
/// - with `ran = 0` it is the oldest among any eligible check that was observed at all, which is
///   §30.4's ordinary case — a `partial` sweep produces no verdict and was still a read, and the
///   basis has to be able to say when.
///
/// `None` means **no check has ever been observed**, so there is no coverage figure to state and
/// none is invented. That is the case `HealthSummary.observedAt` is null for, and it is why
/// `scoredOpen` is null with it: without `unknown` beside it, a `0` cannot be read as *nothing
/// open* (R117's gate) and would be the bare zero §30.1 exists to prevent.
#[must_use]
pub fn basis_over(entries: &[CheckObservation]) -> Option<HealthBasis> {
    let mut ran = 0u32;
    let mut unknown = 0u32;
    let mut off = 0u32;
    let mut not_applicable = 0u32;
    let mut ran_oldest: Option<i64> = None;
    let mut any_oldest: Option<i64> = None;

    for entry in entries {
        // Exhaustive, with no `_ =>` arm: a sixth outcome fails to compile here rather than
        // falling silently into none of the four counters.
        match entry.check.outcome {
            CheckOutcome::Ok | CheckOutcome::Failed => {
                ran += 1;
                ran_oldest = older(ran_oldest, entry.observed_at);
            }
            CheckOutcome::Unknown => unknown += 1,
            CheckOutcome::Off => off += 1,
            CheckOutcome::NotApplicable => not_applicable += 1,
        }
        if in_eligible(entry.check.outcome) {
            any_oldest = older(any_oldest, entry.observed_at);
        }
    }

    let observed_at = if ran > 0 { ran_oldest } else { any_oldest }?;
    Some(HealthBasis {
        ran,
        // Derived, never accumulated separately: `eligible` that could disagree with `ran +
        // unknown` is the one value stated twice.
        eligible: ran + unknown,
        unknown,
        off,
        not_applicable,
        observed_at,
    })
}

fn older(current: Option<i64>, candidate: Option<i64>) -> Option<i64> {
    match (current, candidate) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (Some(a), None) => Some(a),
        (None, b) => b,
    }
}
