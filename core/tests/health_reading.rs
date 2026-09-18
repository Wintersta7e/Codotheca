//! §30.1–§30.3 — the exclusion pipeline, the five check outcomes, the basis over them, and the
//! reason every `unknown` check carries.
#![cfg(feature = "testkit")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use codotheca_core::health::state::{health_state, StateInputs};
use codotheca_core::protocol::{HealthState, Presence};

/// A project with nothing wrong with it: every gate open, so a mutator below is the *only* reason
/// a verdict moves.
fn live() -> StateInputs {
    StateInputs {
        is_reference: Some(false),
        authored_by_user: Some(true),
        error_kind: None,
        ever_observed: true,
        locations: vec![Presence::Present],
        enrolled: true,
        is_archived: false,
        prior_reading: true,
    }
}

/// One gate: what turns it on, and what it produces on its own.
struct Gate {
    name: &'static str,
    apply: fn(&mut StateInputs),
    verdict: HealthState,
}

/// §30.2's six, **in precedence order**. The index is the precedence, so a pair's expected
/// verdict is the lower index's verdict and the table needs no second statement of the order.
fn gates() -> Vec<Gate> {
    vec![
        Gate {
            name: "1 is_reference",
            apply: |i| i.is_reference = Some(true),
            verdict: HealthState::Absent,
        },
        Gate {
            name: "2 authored_by_user IS NULL",
            apply: |i| i.authored_by_user = None,
            verdict: HealthState::Absent,
        },
        Gate {
            name: "3 error_kind with nothing ever observed",
            apply: |i| {
                i.error_kind = Some("REPO_UNREADABLE".to_owned());
                i.ever_observed = false;
            },
            verdict: HealthState::Absent,
        },
        Gate {
            // Additive, not a replacement: gates 4 and 6 are the one pair that share a field, and
            // a mutator that overwrote `locations` would let whichever ran last decide — which is
            // the precedence question this test exists to answer.
            name: "4 no copy that can be read",
            apply: |i| {
                i.locations.retain(|p| *p != Presence::Present);
                i.locations.push(Presence::Missing);
            },
            verdict: HealthState::Absent,
        },
        Gate {
            name: "5 not enrolled",
            apply: |i| i.enrolled = false,
            verdict: HealthState::Suppressed,
        },
        Gate {
            name: "6 offline with a prior reading",
            apply: |i| {
                i.locations.retain(|p| *p != Presence::Present);
                i.locations.push(Presence::Offline);
                i.prior_reading = true;
            },
            verdict: HealthState::Frozen,
        },
    ]
}

/// §30.2's strict precedence, asserted **per gate pair in both application orders** — a pipeline
/// whose gates are evaluated in the wrong order is a behaviour change that a per-gate test passes
/// straight through, because every gate is right on its own.
#[test]
fn ac_p3_30_7_the_six_gates_apply_in_precedence_order() {
    let gates = gates();

    // Each on its own first, or a pair assertion could hold with every gate producing the wrong
    // verdict for the same reason.
    for gate in &gates {
        let mut inputs = live();
        (gate.apply)(&mut inputs);
        assert_eq!(
            health_state(&inputs),
            gate.verdict,
            "gate {} alone",
            gate.name
        );
    }
    assert_eq!(health_state(&live()), HealthState::Live, "no gate fires");

    let mut pairs = 0usize;
    for (i, first) in gates.iter().enumerate() {
        for second in gates.iter().skip(i + 1) {
            for order in [[first, second], [second, first]] {
                let mut inputs = live();
                (order[0].apply)(&mut inputs);
                (order[1].apply)(&mut inputs);
                assert_eq!(
                    health_state(&inputs),
                    first.verdict,
                    "{} and {} together: the earlier gate wins whichever is applied first",
                    first.name,
                    second.name
                );
            }
            pairs += 1;
        }
    }
    eprintln!("gate pairs exercised in both orders: {pairs}");
    assert!(pairs > 0, "a precedence test over no pairs proves nothing");

    // The two the plan names by hand, because they are the ones a reader checks against the prose.
    let mut reference_offline_unenrolled = live();
    reference_offline_unenrolled.is_reference = Some(true);
    reference_offline_unenrolled.enrolled = false;
    reference_offline_unenrolled.locations = vec![Presence::Offline];
    assert_eq!(
        health_state(&reference_offline_unenrolled),
        HealthState::Absent
    );

    let mut unenrolled_offline = live();
    unenrolled_offline.enrolled = false;
    unenrolled_offline.locations = vec![Presence::Offline];
    assert_eq!(health_state(&unenrolled_offline), HealthState::Suppressed);

    // Gate 5's second cause. §30.7's *no health nagging for an archived project* is satisfied by
    // construction here rather than by a second rule somewhere else.
    let mut archived = live();
    archived.is_archived = true;
    assert_eq!(health_state(&archived), HealthState::Suppressed);
}

/// **A freeze must never manufacture a zero**, and freezing is not clearing.
///
/// `{offline, missing}` rolls up to `Missing` and `{offline, unscanned}` to `Unscanned`
/// (`core/src/scan/presence.rs:129-132`). Neither is `Offline`, so **neither freezes** — §4.6's
/// prose mentions neither case, and the code is right where the prose is under-specified.
#[test]
fn ac_p3_30_6_a_frozen_reading_carries_its_age_and_an_offline_project_with_no_prior_reading_is_absent(
) {
    let mut offline_first_time = live();
    offline_first_time.locations = vec![Presence::Offline];
    offline_first_time.prior_reading = false;
    assert_eq!(
        health_state(&offline_first_time),
        HealthState::Absent,
        "a freeze over a reading that was never computed would manufacture a zero"
    );

    let mut offline_again = live();
    offline_again.locations = vec![Presence::Offline];
    assert_eq!(health_state(&offline_again), HealthState::Frozen);

    for mixed in [
        vec![Presence::Offline, Presence::Missing],
        vec![Presence::Offline, Presence::Unscanned],
    ] {
        let mut inputs = live();
        inputs.locations.clone_from(&mixed);
        let state = health_state(&inputs);
        assert_eq!(state, HealthState::Absent, "{mixed:?} rolls up to absent");
        assert_ne!(state, HealthState::Frozen, "{mixed:?} must never freeze");
    }

    // A copy that cannot be read contributes nothing and does not freeze the copies that can:
    // `project_presence` answers `Present` the moment any copy is present, and §5.1 aggregates
    // over present copies.
    let mut one_present_one_offline = live();
    one_present_one_offline.locations = vec![Presence::Present, Presence::Offline];
    assert_eq!(health_state(&one_present_one_offline), HealthState::Live);

    // A project with no copies at all, which gate 4 answers without reaching the rollup.
    let mut no_copies = live();
    no_copies.locations = Vec::new();
    assert_eq!(health_state(&no_copies), HealthState::Absent);
}

/// Gate 3's conjunct. An error recorded **after** something was observed does not discard the
/// reading that observation produced — `notRead` is never `absent` (§30.3), and this is the gate
/// where that rule is either kept or lost.
#[test]
fn ac_p3_30_6_an_error_after_an_observation_does_not_hide_the_reading() {
    let mut failed_after_reading = live();
    failed_after_reading.error_kind = Some("BUDGET_EXCEEDED".to_owned());
    failed_after_reading.ever_observed = true;
    assert_eq!(health_state(&failed_after_reading), HealthState::Live);

    let mut failed_before_reading = live();
    failed_before_reading.error_kind = Some("BUDGET_EXCEEDED".to_owned());
    failed_before_reading.ever_observed = false;
    assert_eq!(health_state(&failed_before_reading), HealthState::Absent);
}

// ---------------------------------------------------------------------------------------------
// §30.3 — the five check outcomes, the basis over them, and the oldest input (Task 4).
// ---------------------------------------------------------------------------------------------

use codotheca_core::health::outcome::{
    basis_over, outcome_for, CheckObservation, SweepFacts, SwitchState,
};
use codotheca_core::protocol::{
    CheckOutcome, DebtSource, DebtSweepOutcome, HealthCheck, UnknownReason,
};

const ON: SwitchState = SwitchState {
    enabled: true,
    grant_missing: false,
    not_applicable: false,
};

fn swept(outcome: DebtSweepOutcome, scored_open: u32, unverified: u32) -> SweepFacts {
    SweepFacts {
        outcome: Some(outcome),
        scored_open,
        unverified,
        observed_at: Some(1_000),
    }
}

fn check(outcome: CheckOutcome, observed_at: Option<i64>) -> CheckObservation {
    CheckObservation {
        check: HealthCheck {
            id: DebtSource::MissingReadme,
            outcome,
            unknown_reason: match outcome {
                CheckOutcome::Unknown => Some(UnknownReason::NotRunYet),
                _ => None,
            },
        },
        observed_at,
    }
}

/// **`ran` is the only denominator this system has.** `off` and `notApplicable` appear in neither
/// `ran` nor `eligible`, and no arithmetic path moves a value into `ran` — which is what makes it
/// impossible to report a suppressed, switched-off or failed check as a passing one.
#[test]
fn ac_p3_30_4_no_arithmetic_path_moves_a_value_into_ran() {
    let all = [
        CheckOutcome::Ok,
        CheckOutcome::Failed,
        CheckOutcome::Unknown,
        CheckOutcome::Off,
        CheckOutcome::NotApplicable,
    ];

    // Every multiset of the five outcomes up to three checks long, which is enough to reach every
    // combination of the four counters without enumerating a library.
    let mut cases = 0usize;
    for a in all {
        for b in all {
            for c in all {
                let entries = [check(a, Some(10)), check(b, Some(20)), check(c, Some(30))];
                let expect = |want: CheckOutcome| {
                    u32::try_from([a, b, c].iter().filter(|o| **o == want).count()).unwrap()
                };
                let Some(basis) = basis_over(&entries) else {
                    // Every check `off` or `notApplicable`: nothing eligible was observed, so
                    // there is no coverage figure and none is invented. `AC-P3-30-1`'s
                    // every-check-off case is exactly this, and it must not be a zeroed basis.
                    assert_eq!(
                        expect(CheckOutcome::Off) + expect(CheckOutcome::NotApplicable),
                        3,
                        "{a:?}/{b:?}/{c:?} produced no basis while something was eligible"
                    );
                    cases += 1;
                    continue;
                };
                let ok = expect(CheckOutcome::Ok);
                let failed = expect(CheckOutcome::Failed);
                assert_eq!(basis.ran, ok + failed, "{a:?}/{b:?}/{c:?}");
                assert_eq!(basis.unknown, expect(CheckOutcome::Unknown));
                assert_eq!(basis.off, expect(CheckOutcome::Off));
                assert_eq!(basis.not_applicable, expect(CheckOutcome::NotApplicable));
                assert_eq!(basis.eligible, basis.ran + basis.unknown);
                // The claim stated as a claim: neither disjoint field is inside either total.
                assert_eq!(
                    basis.ran + basis.unknown + basis.off + basis.not_applicable,
                    3
                );
                cases += 1;
            }
        }
    }
    eprintln!("outcome combinations exercised: {cases}");
    assert!(cases > 0, "a basis test over no cases proves nothing");
}

/// §30.3's two hardest rows, and the `shown_only` property beside them (A7).
#[test]
fn ac_p3_30_5_a_partial_sweep_with_no_items_is_unknown_never_ok() {
    // An item a partial sweep did not reach looks exactly like an item that is gone.
    assert_eq!(
        outcome_for(&swept(DebtSweepOutcome::Partial, 0, 0), &ON),
        CheckOutcome::Unknown
    );
    // An item observed is an item: the under-claim risk is zero, so this is `failed`.
    assert_eq!(
        outcome_for(&swept(DebtSweepOutcome::Partial, 1, 0), &ON),
        CheckOutcome::Failed
    );
    assert_eq!(
        outcome_for(&swept(DebtSweepOutcome::Complete, 0, 0), &ON),
        CheckOutcome::Ok
    );
    // The R128/F11 conjunct: `unverified` items make a complete sweep `unknown`, not `ok`.
    assert_eq!(
        outcome_for(&swept(DebtSweepOutcome::Complete, 0, 3), &ON),
        CheckOutcome::Unknown
    );
    // No row at all, and the four outcomes that are reasons the sweep could not finish.
    let never = SweepFacts {
        outcome: None,
        scored_open: 0,
        unverified: 0,
        observed_at: None,
    };
    assert_eq!(outcome_for(&never, &ON), CheckOutcome::Unknown);
    for outcome in [
        DebtSweepOutcome::Failed,
        DebtSweepOutcome::Unobservable,
        DebtSweepOutcome::SkippedReference,
        DebtSweepOutcome::SkippedSuppressed,
    ] {
        assert_eq!(
            outcome_for(&swept(outcome, 0, 0), &ON),
            CheckOutcome::Unknown,
            "{outcome:?}"
        );
    }

    // A `shown_only` item moves neither `scoredOpen` nor the outcome — `SweepFacts.scored_open`
    // counts items that are both `open` and `scored`, so a project whose only item is
    // `shown_only` presents as a complete sweep with zero scored open items.
    assert_eq!(
        outcome_for(&swept(DebtSweepOutcome::Complete, 0, 0), &ON),
        CheckOutcome::Ok,
        "a shown_only item does not make a check failed"
    );

    // The two outcomes outside `eligible`, and the switch winning over not-applicable.
    let off = SwitchState {
        enabled: false,
        ..ON
    };
    let na = SwitchState {
        not_applicable: true,
        ..ON
    };
    let both = SwitchState {
        enabled: false,
        not_applicable: true,
        ..ON
    };
    assert_eq!(
        outcome_for(&swept(DebtSweepOutcome::Complete, 5, 0), &off),
        CheckOutcome::Off,
        "off is decided before anything is counted"
    );
    assert_eq!(
        outcome_for(&swept(DebtSweepOutcome::Complete, 5, 0), &na),
        CheckOutcome::NotApplicable
    );
    assert_eq!(
        outcome_for(&swept(DebtSweepOutcome::Complete, 0, 0), &both),
        CheckOutcome::Off,
        "a user's own switch is a different sentence from a repo that never needed the check"
    );
}

/// §30.3 — **the oldest input, not the newest.** A reading is only as current as its stalest
/// input; dating it by the freshest read is the staleness marker lying.
#[test]
fn ac_p3_30_14_observed_at_is_the_oldest_input() {
    let old = 1_700_000_000;
    let new = 1_700_090_000;
    let entries = [
        check(CheckOutcome::Ok, Some(new)),
        check(CheckOutcome::Failed, Some(old)),
    ];
    let basis = basis_over(&entries).expect("two ran checks carry a basis");
    eprintln!(
        "inputs observed at {old} and {new}; basis dated {}",
        basis.observed_at
    );
    assert_eq!(basis.observed_at, old);
    assert_eq!(basis.ran, 2);

    // An `unknown` check's own observation does not date a reading that has verdicts: its
    // unknownness is current, and its stale read produced no verdict to be stale about.
    let with_stale_unknown = [
        check(CheckOutcome::Ok, Some(new)),
        check(CheckOutcome::Unknown, Some(1)),
    ];
    assert_eq!(
        basis_over(&with_stale_unknown).expect("basis").observed_at,
        new
    );

    // §30.4's ordinary case: nothing ran, something was read. The basis exists and says when.
    let nothing_ran = [check(CheckOutcome::Unknown, Some(old))];
    let basis = basis_over(&nothing_ran).expect("an observed unknown still carries a basis");
    assert_eq!(basis.ran, 0);
    assert_eq!(basis.eligible, 1);
    assert_eq!(basis.observed_at, old);

    // Nothing has ever been observed: there is no coverage figure to state, and none is invented.
    let never = [
        check(CheckOutcome::Unknown, None),
        check(CheckOutcome::Off, None),
    ];
    assert!(
        basis_over(&never).is_none(),
        "a basis over nothing observed would have to invent a date"
    );
    assert!(basis_over(&[]).is_none());
}
