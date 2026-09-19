//! §30.8 — `active` / `done` / `archived`, computed once and expressed nowhere else.
#![cfg(feature = "testkit")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use codotheca_core::health::lifecycle::lifecycle_of;
use codotheca_core::protocol::{
    CheckOutcome, ConditionSignal, DebtSource, HealthBasis, HealthCheck, HealthReading,
    HealthState, ProjectLifecycle,
};

const NOW: i64 = 1_781_179_200;

fn basis(ran: u32, unknown: u32) -> HealthBasis {
    HealthBasis {
        ran,
        eligible: ran + unknown,
        unknown,
        off: 0,
        not_applicable: 0,
        observed_at: NOW,
    }
}

fn reading(state: HealthState, ran: u32, unknown: u32, scored_open: u32) -> HealthReading {
    HealthReading {
        state,
        scored_open: Some(scored_open),
        basis: Some(basis(ran, unknown)),
        checks: vec![HealthCheck {
            id: DebtSource::MissingReadme,
            outcome: CheckOutcome::Ok,
            unknown_reason: None,
        }],
    }
}

/// **`done` requires positive evidence**: every eligible check `ok`, from a `live` reading, with
/// nothing scored open — and one unknown check is enough to withhold it.
#[test]
fn ac_p3_30_13_done_only_where_every_eligible_check_is_ok_from_a_live_reading() {
    assert_eq!(
        lifecycle_of(&reading(HealthState::Live, 3, 0, 0), false, None),
        ProjectLifecycle::Done
    );
    assert_eq!(
        lifecycle_of(&reading(HealthState::Live, 3, 1, 0), false, None),
        ProjectLifecycle::Active,
        "an unknown check is not a passing one"
    );
    assert_eq!(
        lifecycle_of(&reading(HealthState::Live, 3, 0, 2), false, None),
        ProjectLifecycle::Active,
        "an open item is not done"
    );
    // `ran = 0` on a live reading is a project between enrolment and its first sweep. Nothing has
    // passed, so nothing is done — a vacuous *every eligible check is ok* must not read as done.
    assert_eq!(
        lifecycle_of(&reading(HealthState::Live, 0, 0, 0), false, None),
        ProjectLifecycle::Active,
        "a project that has run no check is not finished"
    );
    // A reading with no basis has no eligible set to be complete over.
    let no_basis = HealthReading {
        state: HealthState::Live,
        scored_open: None,
        basis: None,
        checks: Vec::new(),
    };
    assert_eq!(
        lifecycle_of(&no_basis, false, None),
        ProjectLifecycle::Active
    );
}

/// **`frozen` included**: a freeze must never manufacture a verdict any more than it manufactures
/// a zero.
#[test]
fn ac_p3_30_13_absent_suppressed_and_frozen_are_never_done() {
    let mut checked = 0usize;
    for state in [
        HealthState::Absent,
        HealthState::Suppressed,
        HealthState::Frozen,
    ] {
        let verdict = lifecycle_of(&reading(state, 3, 0, 0), false, None);
        assert_eq!(verdict, ProjectLifecycle::Active, "{state:?} produced done");
        assert_ne!(verdict, ProjectLifecycle::Done);
        checked += 1;
    }
    eprintln!("non-live states checked separately: {checked}");
    assert_eq!(checked, 3, "one of the three states was not exercised");
}

/// **Declared, never derived.** `archived` outranks every other input, including a reading that
/// would otherwise be `done`.
#[test]
fn ac_p3_30_13_archived_tracks_is_archived_and_is_derived_from_nothing_else() {
    assert_eq!(
        lifecycle_of(&reading(HealthState::Live, 3, 0, 0), true, None),
        ProjectLifecycle::Archived
    );
    assert_eq!(
        lifecycle_of(
            &reading(HealthState::Absent, 0, 0, 0),
            true,
            Some(ConditionSignal::Live)
        ),
        ProjectLifecycle::Archived
    );
    // And nothing else produces it: every non-archived combination below is `active` or `done`.
    for state in [HealthState::Live, HealthState::Frozen, HealthState::Absent] {
        for signal in [
            None,
            Some(ConditionSignal::Abandoned),
            Some(ConditionSignal::Empty),
        ] {
            assert_ne!(
                lifecycle_of(&reading(state, 3, 0, 0), false, signal),
                ProjectLifecycle::Archived,
                "{state:?}/{signal:?} derived archived"
            );
        }
    }
}

/// §30.8's **hold**: a project being worked on this month is not finished, whatever its tree says.
/// It opens no item, enters no basis term and moves no count — it only withholds `done`.
#[test]
fn ac_p3_30_13_a_project_at_condition_signal_live_or_idle_is_active_whatever_its_tree_says() {
    let spotless = reading(HealthState::Live, 4, 0, 0);
    assert_eq!(
        lifecycle_of(&spotless, false, None),
        ProjectLifecycle::Done,
        "the same reading without the hold is done"
    );
    for signal in [ConditionSignal::Live, ConditionSignal::Idle] {
        assert_eq!(
            lifecycle_of(&spotless, false, Some(signal)),
            ProjectLifecycle::Active,
            "{signal:?} did not hold"
        );
    }
    // And the bands that do not hold.
    for signal in [
        ConditionSignal::Dormant,
        ConditionSignal::Neglected,
        ConditionSignal::Abandoned,
        ConditionSignal::Offline,
        ConditionSignal::Empty,
    ] {
        assert_eq!(
            lifecycle_of(&spotless, false, Some(signal)),
            ProjectLifecycle::Done,
            "{signal:?} held and should not have"
        );
    }
}

/// **`done` is a `ProjectLifecycle`, not a `ConditionSignal` variant.** The stored column
/// `condition_material` is `Option<ConditionSignal>` over seven **activity** bands; a lifecycle
/// verdict in that vocabulary would add a meaningless variant to `condition_signal`, which shares
/// the enum.
#[test]
fn ac_p3_30_13_condition_signal_gains_no_variant() {
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repo root");
    let schema: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(repo.join("protocol/schema/protocol.json")).unwrap(),
    )
    .unwrap();
    let declared: Vec<String> = schema["types"]["ConditionSignal"]["variants"]
        .as_array()
        .expect("ConditionSignal is a declared enum")
        .iter()
        .map(|v| v.as_str().expect("a variant is a string").to_owned())
        .collect();
    eprintln!(
        "ConditionSignal variants declared by the schema: {} {declared:?}",
        declared.len()
    );
    assert!(
        !declared.is_empty(),
        "a vocabulary test over none proves none"
    );

    // Against the generated enum's own arity, so the two cannot drift apart silently.
    assert_eq!(ConditionSignal::ALL.len(), declared.len());
    for banned in ["done", "stable"] {
        assert!(
            !declared.iter().any(|v| v == banned),
            "ConditionSignal gained {banned}, which is a lifecycle verdict"
        );
    }
    // And the lifecycle's own vocabulary is disjoint from the activity bands.
    let lifecycle: Vec<String> = schema["types"]["ProjectLifecycle"]["variants"]
        .as_array()
        .expect("ProjectLifecycle is a declared enum")
        .iter()
        .map(|v| v.as_str().expect("a variant is a string").to_owned())
        .collect();
    for value in &lifecycle {
        assert!(
            !declared.iter().any(|v| v == value),
            "{value} is in both vocabularies"
        );
    }
}
