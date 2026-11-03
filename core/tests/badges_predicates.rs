//! §40.2–§40.3's populations and gates, each driven through the J4 settle that writes the
//! commit-days they read.
//!
//! Every test prints the rows it read, so a fixture that wrote no commit-day cannot pass by
//! awarding nothing over nothing.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use codotheca_core::clock::local_date;
use codotheca_core::ledger::price::REVIVAL_GAP_DAYS;
use support::badges_rig::{BadgesRig, Flags};

/// A local day number in early 2024: the first commit-day of every gap below.
const D1: i64 = 19_724;

/// `project`'s commit-day rows.
fn commit_days(rig: &BadgesRig, project: i64) -> i64 {
    let guard = rig.index.lock().unwrap();
    let n = guard
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM xp_events WHERE kind = 'commit_day' AND project_id = ?1",
            [project],
            |r| r.get(0),
        )
        .unwrap();
    drop(guard);
    n
}

/// With `gap = d2 − d1`, Necromancer is not awarded one below §38's dormancy constant and is
/// awarded at it — the constant read from §38's module.
#[test]
fn ac_p4_40_10_necromancer_is_the_gap_rule_at_the_constant() {
    let mut rig = BadgesRig::new();
    let below = rig.project("below", &[D1, D1 + REVIVAL_GAP_DAYS - 1], Flags::default());
    rig.settle_j4(below);
    eprintln!(
        "gap {}: {} commit-days, award {:?}",
        REVIVAL_GAP_DAYS - 1,
        commit_days(&rig, below.0),
        rig.award("necromancer")
    );
    assert_eq!(commit_days(&rig, below.0), 2);
    assert_eq!(rig.award("necromancer"), None);

    let at = rig.project("at", &[D1, D1 + REVIVAL_GAP_DAYS], Flags::default());
    rig.settle_j4(at);
    let award = rig.award("necromancer");
    eprintln!("gap {REVIVAL_GAP_DAYS}: award {award:?}");
    let (earned_on, project, evidence) = award.expect("no award at the constant");
    assert_eq!(earned_on, local_date(D1 + REVIVAL_GAP_DAYS));
    assert_eq!(project, Some(at.0));
    assert_eq!(evidence["gapDays"], REVIVAL_GAP_DAYS);
}

/// §40.2's **H**: a qualifying gap in a shallow project, and the same gap in a Reference
/// project, yield no Necromancer.
#[test]
fn ac_p4_40_9_shallow_and_reference_yield_no_necromancer() {
    let mut rig = BadgesRig::new();
    let gap = [D1, D1 + REVIVAL_GAP_DAYS];
    let shallow = rig.project(
        "shallow",
        &gap,
        Flags {
            shallow: true,
            reference: false,
        },
    );
    rig.settle_j4(shallow);
    // J4 still writes a shallow project's days; only the population keeps them out.
    eprintln!(
        "shallow: {} commit-days, award {:?}",
        commit_days(&rig, shallow.0),
        rig.award("necromancer")
    );
    assert_eq!(commit_days(&rig, shallow.0), 2);
    assert_eq!(rig.award("necromancer"), None);

    let reference = rig.project(
        "reference",
        &gap,
        Flags {
            shallow: false,
            reference: true,
        },
    );
    rig.settle_j4(reference);
    eprintln!(
        "reference: {} commit-days, award {:?}",
        commit_days(&rig, reference.0),
        rig.award("necromancer")
    );
    assert_eq!(rig.award("necromancer"), None);
}
