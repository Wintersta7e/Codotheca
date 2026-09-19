//! §30.1's closing paragraph — the four boundaries that write no row, and the fifth.
//!
//! **This criterion is split by the wave order, and the split is reported rather than hidden.**
//! `AC-P3-30-18` asks for a core-side write test that drives an enrolment, a switch toggle, a
//! freeze and an unfreeze and asserts **zero** rows written across all four, then asserts a
//! genuine observed transition writes one. `health_delta` has **no producer until p3-34** and its
//! table is not rebuilt until migration `0016`, so no row can be written or withheld in this
//! worktree: this file lands the **predicate** and drives all five cases through it, printing a
//! verdict per case, and the row-count half lands with p3-34's producer under the same tag.
#![cfg(feature = "testkit")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use codotheca_core::health::delta_gate::{delta_admissible, DeltaBoundary, Observation};
use codotheca_core::protocol::{DebtSource, DecayLayer, HealthState};

fn observed(value: u32) -> Observation {
    Observation {
        state: HealthState::Live,
        enrolled: true,
        layer: DecayLayer::Dust,
        eligible: vec![DebtSource::MissingReadme, DebtSource::MissingLicense],
        value: Some(value),
    }
}

/// The five boundaries, each asserting `Err` with **its own** variant — a single *"not
/// comparable"* would make five different restorations one sentence.
#[test]
fn ac_p3_30_18_no_delta_crosses_an_enrolment_a_freeze_an_unfreeze_a_switch_toggle_or_a_first_observation(
) {
    let cases: [(&str, Observation, Observation, DeltaBoundary); 5] = [
        (
            "enrolment",
            Observation {
                enrolled: false,
                ..observed(9)
            },
            observed(4),
            DeltaBoundary::Enrolment,
        ),
        (
            "freeze",
            observed(9),
            Observation {
                state: HealthState::Frozen,
                ..observed(4)
            },
            DeltaBoundary::Freeze,
        ),
        (
            "unfreeze",
            Observation {
                state: HealthState::Frozen,
                ..observed(9)
            },
            observed(4),
            DeltaBoundary::Unfreeze,
        ),
        (
            "switch toggle",
            observed(9),
            Observation {
                eligible: vec![DebtSource::MissingReadme],
                ..observed(4)
            },
            DeltaBoundary::SwitchToggle,
        ),
        (
            "first observation",
            Observation {
                value: None,
                ..observed(0)
            },
            observed(4),
            DeltaBoundary::FirstObservation,
        ),
    ];

    let mut exercised = 0usize;
    for (name, from, to, expected) in cases {
        let verdict = delta_admissible(&from, &to);
        eprintln!("{name}: {verdict:?}");
        assert_eq!(verdict, Err(expected), "{name} was admitted or misnamed");
        exercised += 1;
    }
    eprintln!("boundaries exercised: {exercised}");
    assert!(
        exercised > 0,
        "a case that exercised nothing proves nothing"
    );

    // The layer half of the same-set rule: two endpoints about different layers count different
    // things, and a delta between them measures the choice of layer rather than the project.
    let verdict = delta_admissible(
        &observed(9),
        &Observation {
            layer: DecayLayer::Cracks,
            ..observed(4)
        },
    );
    eprintln!("different layers: {verdict:?}");
    assert_eq!(verdict, Err(DeltaBoundary::SwitchToggle));

    // And a far end that was never observed is the same refusal from the other side.
    assert_eq!(
        delta_admissible(
            &observed(9),
            &Observation {
                value: None,
                ..observed(0)
            }
        ),
        Err(DeltaBoundary::FirstObservation)
    );
}

/// The case the whole gate exists to let through: two values the app observed, of one layer, over
/// one eligible set, with no boundary between them.
#[test]
fn ac_p3_30_18_a_genuine_observed_transition_is_admissible() {
    let verdict = delta_admissible(&observed(9), &observed(4));
    eprintln!("genuine observed transition: {verdict:?}");
    assert_eq!(verdict, Ok(()));

    // Including one that moved the wrong way — §34 decides what a delta *earns*; the precondition
    // decides only whether the two numbers are comparable.
    assert_eq!(delta_admissible(&observed(4), &observed(9)), Ok(()));
    assert_eq!(delta_admissible(&observed(4), &observed(4)), Ok(()));

    // A frozen pair on both ends is not a freeze crossing: nothing moved, and nothing is claimed.
    let frozen = Observation {
        state: HealthState::Frozen,
        ..observed(4)
    };
    assert_eq!(
        delta_admissible(
            &Observation {
                state: HealthState::Frozen,
                ..observed(9)
            },
            &frozen
        ),
        Ok(())
    );
}

/// **R1 avoided rather than repeated.** A predicate with no caller is a trait that gets its fake
/// and never its real implementation, so the call site §34 must use is named in the module's own
/// documentation — visible to p3-34's author rather than discovered.
#[test]
fn the_precondition_names_the_call_site_section_34_must_use() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/health/delta_gate.rs");
    let text = std::fs::read_to_string(&path).expect("the module is readable");
    assert!(
        !text.is_empty(),
        "an empty file would satisfy nothing below"
    );
    assert!(
        text.contains("§34"),
        "the module does not name §34 as its caller"
    );
    assert!(
        text.contains("writes no row on `Err`"),
        "the module does not state what §34's producer must do with a refusal"
    );
    eprintln!(
        "delta_gate.rs names its §34 call site over {} bytes of documentation",
        text.len()
    );
}
