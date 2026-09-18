//! §30.3 — **where each `UnknownReason` comes from, and the one place a reason is chosen.**
//!
//! An `unknown` check is counted **and named**: it carries a reason, and the reason is never
//! omitted, defaulted, or collapsed to one string. A count of unknowns with no reason is the
//! prototype's single `UNKNOWN · NEEDS GITHUB` note in a different shape.
//!
//! **`notRead` is never `absent`, and that is the invariant, not a nicety.** *A timeout looks
//! exactly like a missing file.* It is one of the three routes §27.5 names to *rendering unknown
//! as zero*, and it is the one an implementer reaches by accident.
//!
//! **`notRunYet` and `unreachable` are `notObserved`'s two self-resolving neighbours and are
//! separate from it deliberately.** `notObserved` is a fact about the repository and does not
//! resolve on its own; the other two do, by different acts, and **a user owed a different act is
//! owed a different sentence.**
//!
//! §30 claims *"§31's `unknown_reason` CHECK stands at four (`31-completion.md:250`)"*. **Both
//! halves are false** (R129/F2): that line is prose about `readme_path`, and §31's DDL is a
//! pointer that never enumerates. The standing rule survives on its own and is what a migration
//! author needs: **every DDL CHECK mirroring `UnknownReason` spells all six, character-identical
//! to the generated enum.**

use super::outcome::{outcome_for, SweepFacts, SwitchState};
use crate::protocol::{
    CheckOutcome, DebtSource, DebtSweepOutcome, HealthCheck, Presence, UnknownReason,
};

/// What the evidence behind one check needs before it can be read at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GrantState {
    /// A forge account is required for this source and none is connected.
    pub account_missing: bool,
    /// The value is read back from its source asynchronously and has not been yet. **Not *"an
    /// account exists and …"*** (R131/F7): §32.4's provider method takes no token, so the advisory
    /// source never has an account, and the honest reading covers `description`, `ciGreen` and
    /// `deps` alike.
    pub awaiting_sync: bool,
}

/// The one place an `UnknownReason` is chosen.
///
/// Order is the ruling. The store being unreachable outranks everything a read could have said,
/// because nothing could be read; a missing account outranks a pending sync, because syncing
/// cannot begin without one; and only then does the sweep row get to speak.
#[must_use]
pub fn reason_for(facts: &SweepFacts, anchor: Presence, grant: &GrantState) -> UnknownReason {
    // The evidence exists and the **local store cannot be reached** — including a check with no
    // value inside a `frozen` reading. It resolves by mounting the store.
    if anchor == Presence::Offline || anchor == Presence::Missing {
        return UnknownReason::Unreachable;
    }
    if grant.account_missing {
        return UnknownReason::NeedsAccount;
    }
    if grant.awaiting_sync {
        return UnknownReason::NotSynced;
    }
    match facts.outcome {
        // A local read was attempted and did not complete. Two routes reach it and both are a
        // re-read, which is why they are one arm rather than two identical ones:
        //
        // - `failed` and `partial` are a budget exceedance, a timeout, an unreadable file or a
        //   sweep that errored. **Never `notObserved`**, which would say there was nothing there;
        // - a `complete` sweep that *still* leaves the check `unknown` leaves it because of
        //   `unverified` items, whose own copy was not the copy this sweep read.
        Some(DebtSweepOutcome::Failed | DebtSweepOutcome::Partial | DebtSweepOutcome::Complete) => {
            UnknownReason::NotRead
        }
        // There is nothing to observe: a bare repository is the case this outcome exists for.
        Some(DebtSweepOutcome::Unobservable) => UnknownReason::NotObserved,
        // Scheduled and not yet run, and it resolves on its own at the next sweep: a project
        // between enrolment and its first sweep, a check switched back on (whose sweep row went
        // with the switch), a source added by a migration, or a skip whose condition has lifted.
        Some(DebtSweepOutcome::SkippedReference | DebtSweepOutcome::SkippedSuppressed) | None => {
            UnknownReason::NotRunYet
        }
    }
}

/// One check, assembled: the outcome, and **the reason exactly when the outcome is `unknown`**.
///
/// A reason on any other outcome would be a second vocabulary for a verdict that already speaks,
/// and an `unknown` without one is the unnamed count this section exists to prevent.
#[must_use]
pub fn check_for(
    id: DebtSource,
    facts: &SweepFacts,
    switch: &SwitchState,
    anchor: Presence,
    grant: &GrantState,
) -> HealthCheck {
    let outcome = outcome_for(facts, switch);
    HealthCheck {
        id,
        outcome,
        unknown_reason: match outcome {
            CheckOutcome::Unknown => Some(reason_for(facts, anchor, grant)),
            CheckOutcome::Ok
            | CheckOutcome::Failed
            | CheckOutcome::Off
            | CheckOutcome::NotApplicable => None,
        },
    }
}
