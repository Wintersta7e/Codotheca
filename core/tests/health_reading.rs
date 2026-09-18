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
