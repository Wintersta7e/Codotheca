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

// ---------------------------------------------------------------------------------------------
// §30.3 — `UnknownReason`: one vocabulary, never omitted (Task 5).
// ---------------------------------------------------------------------------------------------

use codotheca_core::health::reason::{check_for, reason_for, GrantState};

const GRANTED: GrantState = GrantState {
    account_missing: false,
    awaiting_sync: false,
};

/// A reason rides `unknown` and **no other outcome**: one is the app saying it could not tell,
/// and a reason on a verdict that already speaks would be a second vocabulary for it.
#[test]
fn ac_p3_30_16_every_unknown_check_carries_a_reason_and_no_other_outcome_does() {
    let mut seen = 0usize;
    for sweep in [
        None,
        Some(DebtSweepOutcome::Complete),
        Some(DebtSweepOutcome::Partial),
        Some(DebtSweepOutcome::Failed),
        Some(DebtSweepOutcome::Unobservable),
        Some(DebtSweepOutcome::SkippedReference),
        Some(DebtSweepOutcome::SkippedSuppressed),
    ] {
        for scored_open in [0u32, 2] {
            for unverified in [0u32, 1] {
                for switch in [
                    ON,
                    SwitchState {
                        enabled: false,
                        ..ON
                    },
                    SwitchState {
                        grant_missing: true,
                        ..ON
                    },
                    SwitchState {
                        not_applicable: true,
                        ..ON
                    },
                ] {
                    let facts = SweepFacts {
                        outcome: sweep,
                        scored_open,
                        unverified,
                        observed_at: sweep.map(|_| 1_000),
                    };
                    let check = check_for(
                        DebtSource::MissingTests,
                        &facts,
                        &switch,
                        Presence::Present,
                        &GRANTED,
                    );
                    assert_eq!(
                        check.unknown_reason.is_some(),
                        check.outcome == CheckOutcome::Unknown,
                        "{:?} carried {:?}",
                        check.outcome,
                        check.unknown_reason
                    );
                    seen += 1;
                }
            }
        }
    }
    eprintln!("outcome/reason pairings exercised: {seen}");
    assert!(seen > 0, "a reason test over no checks proves nothing");
}

/// **A timeout looks exactly like a missing file**, and telling them apart is the whole of this
/// row. A budget exceedance is `notRead` — never `absent`, never `failed`, and never
/// `notObserved`, which would claim there was nothing there.
#[test]
fn ac_p3_30_16_a_budget_exceedance_is_not_read_never_absent_and_never_fail() {
    for outcome in [DebtSweepOutcome::Partial, DebtSweepOutcome::Failed] {
        let facts = swept(outcome, 0, 0);
        assert_eq!(outcome_for(&facts, &ON), CheckOutcome::Unknown);
        assert_eq!(
            reason_for(&facts, Presence::Present, &GRANTED),
            UnknownReason::NotRead,
            "{outcome:?}"
        );
        assert_ne!(
            reason_for(&facts, Presence::Present, &GRANTED),
            UnknownReason::NotObserved
        );
    }
}

/// §30.9 — a check switched back on is `unknown` until it next runs, **not `ok` and not `0`**,
/// and the sentence it is owed is *this has not run yet*, never *there is nothing to observe*.
///
/// The mechanism is that a switch turned off takes the source's sweep row with it (Task 6): while
/// a check is off the app makes no observation claim for it, so turning it back on leaves no row
/// — which is exactly the state this reason describes.
#[test]
fn ac_p3_30_16_a_check_switched_off_and_back_on_is_not_run_yet_never_not_observed() {
    let no_row = SweepFacts {
        outcome: None,
        scored_open: 0,
        unverified: 0,
        observed_at: None,
    };
    assert_eq!(outcome_for(&no_row, &ON), CheckOutcome::Unknown);
    let reason = reason_for(&no_row, Presence::Present, &GRANTED);
    assert_eq!(reason, UnknownReason::NotRunYet);
    assert_ne!(reason, UnknownReason::NotObserved);

    // And its self-resolving neighbour is not the same sentence: a bare repository has nothing to
    // observe and does not resolve on its own.
    assert_eq!(
        reason_for(
            &swept(DebtSweepOutcome::Unobservable, 0, 0),
            Presence::Present,
            &GRANTED
        ),
        UnknownReason::NotObserved
    );
}

/// §30.3 — the evidence exists and the **local store cannot be reached**, including a check with
/// no value inside a `frozen` reading. It outranks every reason a read could have produced,
/// because nothing could be read.
#[test]
fn ac_p3_30_16_an_offline_anchor_location_is_unreachable() {
    for anchor in [Presence::Offline, Presence::Missing] {
        for sweep in [
            None,
            Some(DebtSweepOutcome::Partial),
            Some(DebtSweepOutcome::Unobservable),
        ] {
            let facts = SweepFacts {
                outcome: sweep,
                scored_open: 0,
                unverified: 0,
                observed_at: sweep.map(|_| 1_000),
            };
            assert_eq!(
                reason_for(&facts, anchor, &GRANTED),
                UnknownReason::Unreachable,
                "{anchor:?}/{sweep:?}"
            );
        }
    }

    // The two grant reasons, in their own order, against a reachable anchor.
    let facts = swept(DebtSweepOutcome::Partial, 0, 0);
    assert_eq!(
        reason_for(
            &facts,
            Presence::Present,
            &GrantState {
                account_missing: true,
                awaiting_sync: true,
            }
        ),
        UnknownReason::NeedsAccount,
        "syncing cannot begin without an account"
    );
    assert_eq!(
        reason_for(
            &facts,
            Presence::Present,
            &GrantState {
                account_missing: false,
                awaiting_sync: true,
            }
        ),
        UnknownReason::NotSynced
    );
}

/// **R128/F8.** `Settings.contentScanEnabled` defaults **false** while `todo_marker`'s switch
/// defaults **on**, so on a default install the two disagree about one source. Resolving it as
/// `unknown` with reason `notRunYet` tells every such user to wait for a sweep that never comes
/// and inflates `eligible` with a check that cannot be evaluated.
///
/// `off` already means *the user chose not to have this* and is already outside `eligible`.
/// **No seventh variant, no note row, no widening of the six-variant CHECK.**
#[test]
fn ac_p3_30_16_an_ungranted_content_scan_makes_todo_marker_off_not_unknown() {
    let never_swept = SweepFacts {
        outcome: None,
        scored_open: 0,
        unverified: 0,
        observed_at: None,
    };
    let ungranted = SwitchState {
        enabled: true,
        grant_missing: true,
        not_applicable: false,
    };
    let todo = check_for(
        DebtSource::TodoMarker,
        &never_swept,
        &ungranted,
        Presence::Present,
        &GRANTED,
    );
    assert_eq!(todo.outcome, CheckOutcome::Off);
    assert_eq!(todo.unknown_reason, None);

    // And the figure it would otherwise have inflated: `eligible` counts it in one case and not
    // the other, which is the whole consequence.
    let ungranted_basis = basis_over(&[CheckObservation {
        check: todo,
        observed_at: None,
    }]);
    assert!(
        ungranted_basis.is_none(),
        "an off check is the only check: nothing eligible was observed"
    );
    let as_unknown = basis_over(&[check(CheckOutcome::Unknown, Some(1_000))])
        .expect("an observed unknown carries a basis");
    assert_eq!(as_unknown.eligible, 1, "this is the figure `off` keeps out");
}

/// **R132/F11** — the criterion is about the *value*, not the count, so **no literal appears in
/// the assertion**: it enumerates the variants the generated enum declares, covers each, and
/// prints the count. A floor of *"at least six"* would leave the run green while a seventh
/// variant went untested.
///
/// R31: the schema declares this vocabulary once and both languages are generated from it. A
/// hand-written copy on either side is the defect that has been caught twice.
#[test]
fn ac_p3_30_16_unknown_reason_exists_once() {
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repo root");
    let schema: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(repo.join("protocol/schema/protocol.json")).unwrap(),
    )
    .unwrap();
    let variants: Vec<String> = schema["types"]["UnknownReason"]["variants"]
        .as_array()
        .expect("UnknownReason is a declared enum")
        .iter()
        .map(|v| v.as_str().expect("a variant is a string").to_owned())
        .collect();
    eprintln!(
        "UnknownReason variants declared by the schema: {}",
        variants.len()
    );
    assert!(
        !variants.is_empty(),
        "a vocabulary test over no variants proves nothing"
    );

    // Every declared variant is reachable from the one chooser, and the set it produces is
    // exactly the set the schema declares — neither short of it nor past it.
    let mut produced: Vec<String> = Vec::new();
    let cases: [(Presence, GrantState, Option<DebtSweepOutcome>); 6] = [
        (Presence::Offline, GRANTED, None),
        (
            Presence::Present,
            GrantState {
                account_missing: true,
                awaiting_sync: false,
            },
            None,
        ),
        (
            Presence::Present,
            GrantState {
                account_missing: false,
                awaiting_sync: true,
            },
            None,
        ),
        (Presence::Present, GRANTED, Some(DebtSweepOutcome::Partial)),
        (
            Presence::Present,
            GRANTED,
            Some(DebtSweepOutcome::Unobservable),
        ),
        (Presence::Present, GRANTED, None),
    ];
    for (anchor, grant, sweep) in cases {
        let facts = SweepFacts {
            outcome: sweep,
            scored_open: 0,
            unverified: 0,
            observed_at: sweep.map(|_| 1_000),
        };
        let reason = reason_for(&facts, anchor, &grant);
        let slug = serde_json::to_value(reason).unwrap();
        let slug = slug.as_str().expect("a reason serialises as a string");
        if !produced.iter().any(|s| s == slug) {
            produced.push(slug.to_owned());
        }
    }
    produced.sort();
    let mut declared = variants.clone();
    declared.sort();
    assert_eq!(
        produced, declared,
        "the chooser reaches every declared variant and invents none"
    );
}

/// **R31** — the schema declares this vocabulary once and both languages are generated from it.
/// A hand-written copy on either side is the defect that has already been caught twice, and it is
/// a different claim from the one above: that one is about the *value*, this one about where the
/// value may be written down.
#[test]
fn ac_p3_30_16_unknown_reason_is_declared_once_and_generated_into_both_languages() {
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repo root");
    let mut rust_declarations = Vec::new();
    let mut sources = Vec::new();
    collect_files(&repo.join("core/src"), "rs", &mut sources);
    collect_files(&repo.join("app/src"), "ts", &mut sources);
    collect_files(&repo.join("app/src"), "tsx", &mut sources);
    assert!(
        !sources.is_empty(),
        "scanned no sources: an R31 audit over nothing is a failing audit"
    );
    for path in &sources {
        let text = std::fs::read_to_string(path).unwrap();
        if text.contains("enum UnknownReason") || text.contains("type UnknownReason") {
            rust_declarations.push(path.clone());
        }
    }
    eprintln!(
        "UnknownReason declarations found over {} sources: {:?}",
        sources.len(),
        rust_declarations
    );
    assert_eq!(
        rust_declarations.len(),
        2,
        "exactly two declarations, both generated: core/src/protocol.rs and app/src/generated"
    );
    for path in &rust_declarations {
        let name = path.to_string_lossy().replace('\\', "/");
        assert!(
            name.ends_with("core/src/protocol.rs") || name.contains("/generated/"),
            "{name} declares UnknownReason by hand"
        );
    }
}

fn collect_files(dir: &std::path::Path, ext: &str, out: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_files(&path, ext, out);
        } else if path.extension().is_some_and(|e| e == ext) && !out.contains(&path) {
            out.push(path);
        }
    }
}

// ---------------------------------------------------------------------------------------------
// §30.1 — the reading assembled, and the freeze applied once, upstream (Task 8).
// ---------------------------------------------------------------------------------------------

use codotheca_core::health::read_for_project;
use codotheca_core::index::migrate::{apply_all, MIGRATIONS};
use codotheca_core::index::{open_connection, Index};
use codotheca_core::protocol::ProjectId;

const READ_NOW: i64 = 1_781_179_200;

fn store() -> (tempfile::TempDir, rusqlite::Connection) {
    let dir = tempfile::tempdir().unwrap();
    let mut conn = open_connection(&Index::db_path(dir.path())).unwrap();
    apply_all(&mut conn, MIGRATIONS).unwrap();
    (dir, conn)
}

/// A project that reaches `live`: authorship computed, not Reference, enrolled, one present copy.
fn seed_live(conn: &rusqlite::Connection) -> i64 {
    conn.execute(
        "INSERT INTO project (name, seed_basename, lineage_key, created_at, updated_at,
                              authored_by_user, is_reference, acknowledged_at)
         VALUES ('p', 'p', 'abc123', 1, 1, 1, 0, ?1)",
        [READ_NOW],
    )
    .unwrap();
    let project = conn.last_insert_rowid();
    // `(kind, distro, path_key)` is UNIQUE, so each seeded copy needs a path of its own.
    let path = format!("/p-{project}");
    conn.execute(
        "INSERT INTO location (project_id, kind, path_bytes, path_key, path_display, store_key,
                               presence, repo_kind, refstate_observed_at)
         VALUES (?1, 'linux', ?3, ?3, ?4, 'store-a', 'present', 'worktree', ?2)",
        rusqlite::params![project, READ_NOW, path.as_bytes(), path],
    )
    .unwrap();
    project
}

fn sweep(conn: &rusqlite::Connection, project: i64, source: &str, outcome: &str, at: i64) {
    let item_count = if outcome == "complete" || outcome == "partial" {
        Some(0_i64)
    } else {
        None
    };
    conn.execute(
        "INSERT INTO debt_sweep (project_id, source, outcome, item_count, observed_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![project, source, outcome, item_count, at],
    )
    .unwrap();
}

fn item(conn: &rusqlite::Connection, project: i64, source: &str, fingerprint: &str, state: &str) {
    // Identity is `(subject_key, source, fingerprint)` and `subject_key` is the project's, so a
    // fixture with several projects needs a subject per project rather than one shared literal.
    conn.execute(
        "INSERT INTO debt_item (project_id, subject_key, source, fingerprint, state, scoring,
                                first_seen_at, last_seen_at)
         VALUES (?1, ?6, ?2, ?3, ?4, 'scored', ?5, ?5)",
        rusqlite::params![
            project,
            source,
            fingerprint,
            state,
            READ_NOW,
            format!("lineage:abc{project}|remote:")
        ],
    )
    .unwrap();
}

/// **The invariant's own counterexample.** A reading that says nothing was computed carries no
/// number at all — `scoredOpen` and `basis` are NULL with no default, core-side, on the writer.
#[test]
fn the_writer_never_writes_zero_for_unknown() {
    let (_dir, conn) = store();

    // `absent`: authorship uncomputed, which is the ordinary state of a project mid-scan.
    conn.execute(
        "INSERT INTO project (name, seed_basename, created_at, updated_at, is_reference)
         VALUES ('a', 'a', 1, 1, 0)",
        [],
    )
    .unwrap();
    let absent = conn.last_insert_rowid();
    let (reading, items) = read_for_project(&conn, ProjectId(absent)).unwrap();
    assert_eq!(reading.state, HealthState::Absent);
    assert_eq!(reading.scored_open, None, "a zero was written for unknown");
    assert!(reading.basis.is_none(), "a zeroed basis was written");
    assert!(
        reading.checks.is_empty(),
        "an empty checks array is the state saying nothing was computed"
    );
    assert!(items.is_empty());

    // `suppressed`: enrolled, but archived.
    let suppressed = seed_live(&conn);
    conn.execute(
        "UPDATE project SET is_archived = 1 WHERE id = ?1",
        [suppressed],
    )
    .unwrap();
    let (reading, items) = read_for_project(&conn, ProjectId(suppressed)).unwrap();
    assert_eq!(reading.state, HealthState::Suppressed);
    assert_eq!(reading.scored_open, None);
    assert!(reading.basis.is_none());
    assert!(reading.checks.is_empty());
    assert!(items.is_empty());
}

/// §30.2's closing rule — **the freeze is applied once, upstream.** A `frozen` project is handed
/// its last computed item set, not the current unobservable one, and §33 and §35 re-derive
/// nothing.
#[test]
fn ac_p3_30_11a_a_frozen_projects_item_set_is_the_last_computed_one() {
    let (_dir, conn) = store();
    let project = seed_live(&conn);
    sweep(&conn, project, "missing_readme", "complete", READ_NOW - 600);
    item(&conn, project, "missing_readme", "", "open");

    let (live, before) = read_for_project(&conn, ProjectId(project)).unwrap();
    assert_eq!(live.state, HealthState::Live);
    assert_eq!(before.len(), 1, "seeded nothing to freeze");

    // The store goes away. Nothing recomputes a set it cannot observe, which is what makes the
    // frozen set the last computed one rather than a fresh empty one.
    conn.execute(
        "UPDATE location SET presence = 'offline' WHERE project_id = ?1",
        [project],
    )
    .unwrap();
    let (frozen, after) = read_for_project(&conn, ProjectId(project)).unwrap();
    assert_eq!(frozen.state, HealthState::Frozen);
    eprintln!(
        "item set before the store went offline: {}, after: {}",
        before.len(),
        after.len()
    );
    assert_eq!(
        after.iter().map(|i| &i.fingerprint).collect::<Vec<_>>(),
        before.iter().map(|i| &i.fingerprint).collect::<Vec<_>>(),
        "the frozen set is not the last computed one"
    );

    // **A frozen reading keeps its value AND its age.** Freezing is not clearing.
    let basis = frozen.basis.expect("a frozen reading carries its basis");
    assert_eq!(basis.observed_at, READ_NOW - 600, "the age was dropped");
    assert_eq!(frozen.scored_open, Some(1));
}

/// The reading over a project with one `ok` check and one `unknown` one — the shape Task 10's
/// shelf summary has to agree with.
#[test]
fn ac_p3_30_11a_a_live_reading_names_every_check_and_counts_only_what_ran() {
    let (_dir, conn) = store();
    let project = seed_live(&conn);
    sweep(&conn, project, "missing_readme", "complete", READ_NOW - 90);
    sweep(&conn, project, "missing_license", "partial", READ_NOW - 30);
    item(&conn, project, "missing_license", "", "open");

    let (reading, _items) = read_for_project(&conn, ProjectId(project)).unwrap();
    assert_eq!(reading.state, HealthState::Live);
    let basis = reading.basis.expect("a live reading with observations");
    eprintln!(
        "ran={} eligible={} unknown={} off={} notApplicable={} observedAt={}",
        basis.ran,
        basis.eligible,
        basis.unknown,
        basis.off,
        basis.not_applicable,
        basis.observed_at
    );
    assert_eq!(basis.ran, 2, "one ok and one failed");
    assert_eq!(reading.scored_open, Some(1));

    // Every check the registry declares is named, and every `unknown` carries a reason.
    assert_eq!(reading.checks.len(), DebtSource::ALL.len());
    for check in &reading.checks {
        assert_eq!(
            check.unknown_reason.is_some(),
            check.outcome == CheckOutcome::Unknown,
            "{check:?}"
        );
    }
    // The never-swept sources are `unknown` and owed *this has not run yet* — except
    // `todo_marker`, whose grant is off by default, which makes it `off` (R128/F8).
    let todo = reading
        .checks
        .iter()
        .find(|c| c.id == DebtSource::TodoMarker)
        .expect("todo_marker is a declared source");
    assert_eq!(todo.outcome, CheckOutcome::Off);
}

// ---------------------------------------------------------------------------------------------
// §30.11 — the wire projection: one producer, both surfaces, batched (Task 10).
// ---------------------------------------------------------------------------------------------

use codotheca_core::health::summary::{summaries_for_all, summary_for};
use codotheca_core::protocol::ProjectLifecycle;
use std::sync::atomic::{AtomicUsize, Ordering};

/// The statement counter `rusqlite`'s trace hook feeds. A plain `fn(&str)` cannot capture, so the
/// count is a static; the one test that reads it resets it first.
static STATEMENTS: AtomicUsize = AtomicUsize::new(0);

fn count_statement(_sql: &str) {
    STATEMENTS.fetch_add(1, Ordering::Relaxed);
}

/// **Two producers would drift.** `projects.list`'s row and `projects.get`'s reading report the
/// same state, the same `scoredOpen` and the same `observedAt` because there is one.
#[test]
fn the_shelf_summary_and_the_page_reading_agree_on_one_project() {
    let (_dir, conn) = store();
    let project = seed_live(&conn);
    sweep(&conn, project, "missing_readme", "complete", READ_NOW - 120);
    sweep(&conn, project, "missing_license", "partial", READ_NOW - 40);
    item(&conn, project, "missing_license", "", "open");
    item(&conn, project, "missing_tests", "", "unverified");

    let (reading, _items) = read_for_project(&conn, ProjectId(project)).unwrap();
    let (summary, lifecycle) = summary_for(&conn, ProjectId(project)).unwrap();
    let basis = reading.basis.as_ref().expect("a live reading");

    eprintln!("reading {reading:?}\nsummary {summary:?} lifecycle {lifecycle:?}");
    assert_eq!(summary.state, reading.state);
    assert_eq!(summary.scored_open, reading.scored_open);
    assert_eq!(summary.unknown_checks, Some(basis.unknown));
    assert_eq!(summary.observed_at, Some(basis.observed_at));
    // `unverified` is the one quantity the reading does not carry, and the shelf has no items —
    // which is why R117 puts the count here and nowhere else.
    assert_eq!(summary.unverified, Some(1));
    assert_eq!(lifecycle, ProjectLifecycle::Active);

    // And the batched path agrees with the single one, or the shelf and the page can disagree.
    let all = summaries_for_all(&conn).unwrap();
    assert_eq!(all.get(&project), Some(&(summary, lifecycle)));
}

/// **The projection is batched or it is a per-row query storm.** A per-row query is the shape
/// this catches: the statement count must not scale with the project count.
#[test]
fn the_projection_issues_a_bounded_number_of_statements_for_n_projects() {
    let measure = |projects: usize| -> usize {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = open_connection(&Index::db_path(dir.path())).unwrap();
        apply_all(&mut conn, MIGRATIONS).unwrap();
        for _ in 0..projects {
            let id = seed_live(&conn);
            sweep(&conn, id, "missing_readme", "complete", READ_NOW - 60);
            item(&conn, id, "missing_readme", "", "open");
        }
        STATEMENTS.store(0, Ordering::Relaxed);
        conn.trace(Some(count_statement));
        let all = summaries_for_all(&conn).unwrap();
        conn.trace(None);
        assert_eq!(all.len(), projects, "the projection skipped a project");
        STATEMENTS.load(Ordering::Relaxed)
    };

    let few = measure(2);
    let many = measure(12);
    eprintln!("statements for 2 projects: {few}; for 12: {many}");
    assert!(
        few > 0,
        "the trace hook counted nothing, so this proves nothing"
    );
    assert_eq!(
        few, many,
        "the statement count scales with the project count: this is the query storm"
    );
}

// ---------------------------------------------------------------------------------------------
// §30.3's `notApplicable` — *marked not-applicable for this project through §31's
// archetype-proposed mechanism*, read over §31.9's bijection.
// ---------------------------------------------------------------------------------------------

use codotheca_core::completion::proposal::suppressed_source;
use codotheca_core::completion::{evaluate_and_write, set_check_na};
use codotheca_core::health::switches::write_switches;
use codotheca_core::protocol::{CompletionCheck, HealthCheckSwitch, HealthReading};

/// §31's ten rows exist, so a ruling has a row to land on — which is how the product reaches it.
fn rule_na(conn: &rusqlite::Connection, project: i64, key: CompletionCheck, na: Option<bool>) {
    let tx = conn.unchecked_transaction().unwrap();
    evaluate_and_write(&tx, ProjectId(project), READ_NOW).unwrap();
    set_check_na(&tx, ProjectId(project), key, na, READ_NOW).unwrap();
    tx.commit().unwrap();
    let key_slug = serde_json::to_value(key).unwrap();
    let stored: Option<i64> = conn
        .query_row(
            "SELECT user_na FROM project_check WHERE project_id = ?1 AND check_key = ?2",
            rusqlite::params![project, key_slug.as_str().unwrap()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(stored, na.map(i64::from), "the ruling did not land");
}

fn switch(conn: &rusqlite::Connection, check: DebtSource, enabled: bool) {
    let tx = conn.unchecked_transaction().unwrap();
    write_switches(&tx, &[HealthCheckSwitch { check, enabled }]).unwrap();
    tx.commit().unwrap();
}

fn outcome_of(reading: &HealthReading, source: DebtSource) -> CheckOutcome {
    reading
        .checks
        .iter()
        .find(|c| c.id == source)
        .expect("every declared source is named")
        .outcome
}

/// **A check the user marked N/A reads `notApplicable`, leaves `eligible`, and its open item
/// neither fails it nor counts.** Through the one producer, on both the page and the shelf path.
#[test]
fn a_check_the_user_marked_not_applicable_reads_not_applicable_and_leaves_eligible() {
    let (_dir, conn) = store();
    let project = seed_live(&conn);
    sweep(&conn, project, "missing_readme", "complete", READ_NOW - 60);
    sweep(&conn, project, "missing_tests", "complete", READ_NOW - 60);
    item(&conn, project, "missing_tests", "", "open");

    // The control: with no ruling the open item fails the check and is counted.
    let (before, _) = read_for_project(&conn, ProjectId(project)).unwrap();
    let before_basis = before.basis.clone().expect("a live reading");
    assert_eq!(
        outcome_of(&before, DebtSource::MissingTests),
        CheckOutcome::Failed
    );
    assert_eq!(before.scored_open, Some(1));
    assert_eq!(before_basis.not_applicable, 0);

    rule_na(&conn, project, CompletionCheck::Tests, Some(true));

    let (after, _) = read_for_project(&conn, ProjectId(project)).unwrap();
    let basis = after.basis.clone().expect("a live reading");
    eprintln!(
        "before: eligible={} ran={} notApplicable={} scoredOpen={:?}; after: eligible={} ran={} \
         notApplicable={} scoredOpen={:?}",
        before_basis.eligible,
        before_basis.ran,
        before_basis.not_applicable,
        before.scored_open,
        basis.eligible,
        basis.ran,
        basis.not_applicable,
        after.scored_open
    );
    assert_eq!(after.state, HealthState::Live);
    assert_eq!(
        outcome_of(&after, DebtSource::MissingTests),
        CheckOutcome::NotApplicable,
        "the user's N/A ruling never reached the reading"
    );
    assert_eq!(basis.not_applicable, 1);
    assert_eq!(
        basis.eligible,
        before_basis.eligible - 1,
        "N/A stayed eligible"
    );
    assert_eq!(basis.ran, before_basis.ran - 1);
    assert!(
        after
            .checks
            .iter()
            .all(|c| c.outcome != CheckOutcome::Failed),
        "an N/A check's item failed the reading: {:?}",
        after.checks
    );
    assert_eq!(
        after.scored_open,
        Some(0),
        "an N/A check's item was counted"
    );

    // The shelf's two paths read the same ruling, or the page and the row disagree.
    let (summary, _) = summary_for(&conn, ProjectId(project)).unwrap();
    assert_eq!(summary.scored_open, Some(0));
    assert_eq!(
        summaries_for_all(&conn)
            .unwrap()
            .get(&project)
            .map(|s| &s.0),
        Some(&summary)
    );

    // §30.3's order: the switch is the user's own statement about the check and is tested first.
    switch(&conn, DebtSource::MissingTests, false);
    let (off, _) = read_for_project(&conn, ProjectId(project)).unwrap();
    assert_eq!(
        outcome_of(&off, DebtSource::MissingTests),
        CheckOutcome::Off
    );
}

/// **The archetype proposes; the user rules.** A proposal reaches the reading with no ruling, and
/// the user's `false` overrides it — the same gate §31's evaluator runs, read and not re-derived.
#[test]
fn an_archetype_proposal_reads_not_applicable_until_the_user_overrides_it() {
    let (_dir, conn) = store();
    let project = seed_live(&conn);
    sweep(&conn, project, "missing_readme", "complete", READ_NOW - 60);
    sweep(&conn, project, "missing_tests", "complete", READ_NOW - 60);
    item(&conn, project, "missing_tests", "", "open");
    conn.execute(
        "UPDATE project SET archetype = 'docs' WHERE id = ?1",
        [project],
    )
    .unwrap();

    let (proposed, _) = read_for_project(&conn, ProjectId(project)).unwrap();
    let na: Vec<DebtSource> = proposed
        .checks
        .iter()
        .filter(|c| c.outcome == CheckOutcome::NotApplicable)
        .map(|c| c.id)
        .collect();
    eprintln!("sources N/A on a docs project with no ruling: {na:?}");
    assert!(
        na.contains(&DebtSource::MissingTests),
        "the archetype's proposal never reached the reading"
    );
    // `ci_red` is N/A only through §31.1a's derived `ciGreen` rule, which §30 never reads.
    assert!(!na.contains(&DebtSource::CiRed));
    assert_eq!(proposed.scored_open, Some(0));

    rule_na(&conn, project, CompletionCheck::Tests, Some(false));
    let (overridden, _) = read_for_project(&conn, ProjectId(project)).unwrap();
    assert_eq!(
        outcome_of(&overridden, DebtSource::MissingTests),
        CheckOutcome::Failed,
        "the user's override lost to the proposal"
    );
    assert_eq!(overridden.scored_open, Some(1));
}

/// §30.1 over both causes at once: **`eligible = 0` is `absent`** whether a check left `eligible`
/// by its switch or by N/A.
#[test]
fn every_check_off_or_not_applicable_is_absent() {
    let (_dir, conn) = store();
    let project = seed_live(&conn);
    sweep(&conn, project, "missing_readme", "complete", READ_NOW - 60);

    // Sources no check stands behind cannot be N/A, so their switches go off.
    let covered: Vec<DebtSource> = CompletionCheck::ALL
        .into_iter()
        .filter_map(suppressed_source)
        .collect();
    for source in DebtSource::ALL {
        if !covered.contains(&source) {
            switch(&conn, source, false);
        }
    }
    let (partial, _) = read_for_project(&conn, ProjectId(project)).unwrap();
    assert_eq!(
        partial.state,
        HealthState::Live,
        "the control is already absent"
    );

    for key in CompletionCheck::ALL {
        if suppressed_source(key).is_some() {
            rule_na(&conn, project, key, Some(true));
        }
    }
    let (reading, _) = read_for_project(&conn, ProjectId(project)).unwrap();
    eprintln!(
        "{} source(s) N/A, {} switched off: {:?}",
        covered.len(),
        DebtSource::ALL.len() - covered.len(),
        reading.state
    );
    assert!(!covered.is_empty());
    assert_eq!(reading.state, HealthState::Absent);
    assert_eq!(reading.scored_open, None);
    assert!(reading.basis.is_none());
    assert!(reading.checks.is_empty());
}

// ---------------------------------------------------------------------------------------------
// §30.9 — **off hides; it never closes, and it never pays.**
// ---------------------------------------------------------------------------------------------

fn item_rows(conn: &rusqlite::Connection, project: i64) -> i64 {
    conn.query_row(
        "SELECT count(*) FROM debt_item WHERE project_id = ?1",
        [project],
        |r| r.get(0),
    )
    .unwrap()
}

/// A switched-off check's open item leaves `scoredOpen` on the page and on the shelf, and its row
/// stays where it was.
#[test]
fn a_switched_off_checks_open_items_leave_scored_open() {
    let (_dir, conn) = store();
    let project = seed_live(&conn);
    sweep(&conn, project, "missing_readme", "complete", READ_NOW - 60);
    sweep(&conn, project, "missing_tests", "complete", READ_NOW - 60);
    item(&conn, project, "missing_tests", "", "open");

    // The control: switched on, the item fails its check and is counted.
    let (before, _) = read_for_project(&conn, ProjectId(project)).unwrap();
    assert_eq!(
        outcome_of(&before, DebtSource::MissingTests),
        CheckOutcome::Failed
    );
    assert_eq!(before.scored_open, Some(1));

    switch(&conn, DebtSource::MissingTests, false);

    let (after, _) = read_for_project(&conn, ProjectId(project)).unwrap();
    let (summary, _) = summary_for(&conn, ProjectId(project)).unwrap();
    eprintln!(
        "scoredOpen switched on: {:?}; switched off: page {:?}, shelf {:?}; item rows {}",
        before.scored_open,
        after.scored_open,
        summary.scored_open,
        item_rows(&conn, project)
    );
    assert_eq!(after.state, HealthState::Live);
    assert_eq!(
        outcome_of(&after, DebtSource::MissingTests),
        CheckOutcome::Off
    );
    assert_eq!(
        after.scored_open,
        Some(0),
        "a switched-off check's item was counted"
    );
    assert_eq!(summary.scored_open, Some(0));
    assert_eq!(
        summaries_for_all(&conn)
            .unwrap()
            .get(&project)
            .map(|s| &s.0),
        Some(&summary)
    );
    assert_eq!(
        item_rows(&conn, project),
        1,
        "switching a check off closed its item"
    );
}

/// **An N/A check's items are treated as `off`'s**: §31.9 rules an N/A check never becomes a debt
/// item, so one already open when the check became N/A leaves the list and every count, on the
/// page and on the shelf — never closed — and comes back unchanged when the ruling is reverted.
#[test]
fn a_not_applicable_checks_items_leave_the_list_and_every_count_and_come_back_unchanged() {
    let (_dir, conn) = store();
    let project = seed_live(&conn);
    sweep(&conn, project, "missing_readme", "complete", READ_NOW - 60);
    sweep(&conn, project, "missing_tests", "complete", READ_NOW - 60);
    item(&conn, project, "missing_tests", "", "open");
    item(&conn, project, "missing_tests", "u", "unverified");

    // The control: both items listed, one counted as open and one as unverified.
    let (before, before_items) = read_for_project(&conn, ProjectId(project)).unwrap();
    let (before_summary, _) = summary_for(&conn, ProjectId(project)).unwrap();
    assert_eq!(before_items.len(), 2);
    assert_eq!(before.scored_open, Some(1));
    assert_eq!(before_summary.unverified, Some(1));

    rule_na(&conn, project, CompletionCheck::Tests, Some(true));

    let (after, items) = read_for_project(&conn, ProjectId(project)).unwrap();
    let (summary, _) = summary_for(&conn, ProjectId(project)).unwrap();
    eprintln!(
        "N/A: items listed {} -> {}, scoredOpen {:?} -> {:?}, unverified {:?} -> {:?}",
        before_items.len(),
        items.len(),
        before.scored_open,
        after.scored_open,
        before_summary.unverified,
        summary.unverified
    );
    assert_eq!(
        outcome_of(&after, DebtSource::MissingTests),
        CheckOutcome::NotApplicable
    );
    assert!(
        items.is_empty(),
        "an N/A check's items are still on the list: {items:?}"
    );
    assert_eq!(after.scored_open, Some(0));
    assert_eq!(
        summary.unverified,
        Some(0),
        "an N/A check's unverified item was counted"
    );
    assert_eq!(
        summaries_for_all(&conn)
            .unwrap()
            .get(&project)
            .map(|s| &s.0),
        Some(&summary)
    );
    assert_eq!(item_rows(&conn, project), 2, "an N/A ruling closed an item");

    // Reverted: the same two items, with the identity they had.
    rule_na(&conn, project, CompletionCheck::Tests, None);
    let (_, back) = read_for_project(&conn, ProjectId(project)).unwrap();
    let identity = |items: &[codotheca_core::protocol::DebtItem]| -> Vec<(String, i64)> {
        let mut out: Vec<_> = items
            .iter()
            .map(|i| (i.fingerprint.clone(), i.first_seen_at))
            .collect();
        out.sort();
        out
    };
    assert_eq!(
        identity(&back),
        identity(&before_items),
        "the items came back with a different identity"
    );
}

/// §30.1's ruling — **what freezes is the observation, not the user's preference.** A check
/// switched off while the store is offline reads `off` and its items leave the list and every
/// count, while every other check keeps the outcome and the age it was frozen with.
#[test]
fn a_switch_applies_live_to_a_frozen_reading_and_the_other_checks_stay_frozen() {
    let (_dir, conn) = store();
    let project = seed_live(&conn);
    sweep(&conn, project, "missing_readme", "complete", READ_NOW - 600);
    item(&conn, project, "missing_readme", "", "open");
    sweep(
        &conn,
        project,
        "missing_license",
        "complete",
        READ_NOW - 600,
    );
    item(&conn, project, "missing_license", "", "open");

    // The control: the store goes away and both checks keep their `failed`, their items and
    // their age under glass.
    conn.execute(
        "UPDATE location SET presence = 'offline' WHERE project_id = ?1",
        [project],
    )
    .unwrap();
    let (frozen, frozen_items) = read_for_project(&conn, ProjectId(project)).unwrap();
    assert_eq!(frozen.state, HealthState::Frozen);
    assert_eq!(
        outcome_of(&frozen, DebtSource::MissingLicense),
        CheckOutcome::Failed
    );
    assert_eq!(frozen_items.len(), 2);
    assert_eq!(frozen.scored_open, Some(2));

    switch(&conn, DebtSource::MissingLicense, false);

    let (after, items) = read_for_project(&conn, ProjectId(project)).unwrap();
    let (summary, _) = summary_for(&conn, ProjectId(project)).unwrap();
    let basis = after
        .basis
        .clone()
        .expect("a frozen reading carries its basis");
    eprintln!(
        "frozen, license switched off: state {:?}, license {:?}, readme {:?}, items {:?}, \
         scoredOpen {:?} (shelf {:?}), observedAt {}",
        after.state,
        outcome_of(&after, DebtSource::MissingLicense),
        outcome_of(&after, DebtSource::MissingReadme),
        items.iter().map(|i| i.source).collect::<Vec<_>>(),
        after.scored_open,
        summary.scored_open,
        basis.observed_at
    );
    assert_eq!(
        after.state,
        HealthState::Frozen,
        "the switch unfroze the store"
    );
    assert_eq!(
        outcome_of(&after, DebtSource::MissingLicense),
        CheckOutcome::Off,
        "a switch waited on a disk: the frozen reading kept the check's frozen outcome"
    );
    assert_eq!(
        outcome_of(&after, DebtSource::MissingReadme),
        CheckOutcome::Failed,
        "a check the user did not touch lost its frozen outcome"
    );
    assert_eq!(
        items.iter().map(|i| i.source).collect::<Vec<_>>(),
        vec![DebtSource::MissingReadme],
        "a switched-off check's item stayed on a frozen project's list"
    );
    assert_eq!(after.scored_open, Some(1));
    assert_eq!(summary.scored_open, Some(1));
    assert_eq!(basis.observed_at, READ_NOW - 600, "the frozen age moved");
}
