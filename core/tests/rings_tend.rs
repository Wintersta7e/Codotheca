//! §39.5's Tend ring: closed on an earned debt day, *nothing to tend* only of what it could look
//! at, over a declared domain `E` that a removed project leaves and an offline copy does not.

#![cfg(feature = "testkit")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use codotheca_core::art::testsupport::CollectingSink;
use codotheca_core::health::switches::switch_key;
use codotheca_core::jobs::NullJobSink;
use codotheca_core::projects::{dispatch_projects_command, ProjectsCtx};
use codotheca_core::protocol::{DebtSource, ProjectId, RingStatus, TendRing};
use codotheca_core::rings::active::{union_days, DaySpan};
use codotheca_core::rings::commit_days::git_days;
use codotheca_core::rings::focus::focus_days;
use codotheca_core::rings::tend::{tend_days, tend_domain, tend_ring};
use codotheca_core::surfaces::settings::KEY_CONTENT_SCAN_ENABLED;
use codotheca_core::testing::FakeMountResolver;
use serde_json::json;
use support::rings_rig::{Flags, RingsRig, UTC};

/// Wednesday 2026-09-16, as an epoch day.
const TODAY: i64 = 20_712;
const DAY: i64 = 86_400;
const NOON: i64 = TODAY * DAY + 12 * 3_600;

fn slug(source: DebtSource) -> String {
    serde_json::to_value(source)
        .unwrap()
        .as_str()
        .unwrap()
        .to_owned()
}

/// An enrolled project on one present copy, every source but `todo_marker` swept `complete` —
/// all a default configuration permits, because the content scan is not granted.
fn clean(rig: &RingsRig) -> ProjectId {
    let project = rig.project(Flags::default());
    rig.location(project, "present", None);
    rig.enrol(project);
    for source in DebtSource::ALL {
        if source != DebtSource::TodoMarker {
            rig.sweep(project, &slug(source), "complete");
        }
    }
    project
}

fn drop_sweep(rig: &RingsRig, project: ProjectId, source: &str) {
    rig.conn()
        .execute(
            "DELETE FROM debt_sweep WHERE project_id = ?1 AND source = ?2",
            rusqlite::params![project.0, source],
        )
        .unwrap();
}

fn switch_off(rig: &RingsRig, source: DebtSource) {
    rig.meta(&switch_key(source).unwrap(), "0");
}

/// `projects.setFlags {isArchived: true}`, through the production command.
fn seal(rig: &RingsRig, project: ProjectId) {
    let sink = CollectingSink::default();
    let mounts = FakeMountResolver::default();
    let ctx = ProjectsCtx {
        index: rig.tmp.index(),
        events: &sink,
        jobs: &NullJobSink,
        mounts: &mounts,
        sync: &codotheca_core::sync::runner::NullSyncSink,
        now: NOON + 60,
        tz_offset_min: 0,
    };
    dispatch_projects_command(
        &ctx,
        "projects.setFlags",
        json!({ "id": project.0, "isArchived": true }),
    )
    .expect("owned")
    .expect("ok");
}

/// A debt day earned today, then a Seal.
fn sealed_after_tending(rig: &RingsRig) {
    let project = clean(rig);
    rig.debt_day(project, NOON, 0, &["missing_readme"]);
    seal(rig, project);
}

/// One row of the truth table: a fresh library, the size of `E` it must have, and the ring.
struct Case {
    name: &'static str,
    setup: fn(&RingsRig),
    domain: usize,
    want: TendRing,
}

fn want(status: RingStatus, excluded: &[DebtSource], unobservable: u32) -> TendRing {
    TendRing {
        status,
        excluded: excluded.to_vec(),
        unobservable_projects: unobservable,
    }
}

const NO_CONSENT: &[DebtSource] = &[DebtSource::TodoMarker];

/// The rows about what blocks *nothing to tend* and what is excluded.
// A table of cases, one short row each, which rustfmt spreads over many lines.
#[allow(clippy::too_many_lines)]
fn sweep_cases() -> Vec<Case> {
    let nothing = want(RingStatus::NothingToTend, NO_CONSENT, 0);
    let open = want(RingStatus::Open, NO_CONSENT, 0);
    vec![
        Case {
            name: "consent off: todo_marker excluded, not blocking",
            setup: |r| {
                clean(r);
            },
            domain: 1,
            want: nothing.clone(),
        },
        Case {
            name: "consent on: todo_marker permitted, unswept, blocks",
            setup: |r| {
                clean(r);
                r.meta(KEY_CONTENT_SCAN_ENABLED, "1");
            },
            domain: 1,
            want: want(RingStatus::Open, &[], 0),
        },
        Case {
            name: "a switch off: excluded, its missing row not blocking",
            setup: |r| {
                let p = clean(r);
                drop_sweep(r, p, "ci_red");
                switch_off(r, DebtSource::CiRed);
            },
            domain: 1,
            want: want(
                RingStatus::NothingToTend,
                &[DebtSource::TodoMarker, DebtSource::CiRed],
                0,
            ),
        },
        Case {
            name: "unobservable: counted, not blocking",
            setup: |r| {
                let p = clean(r);
                r.sweep(p, "ci_red", "unobservable");
            },
            domain: 1,
            want: want(RingStatus::NothingToTend, NO_CONSENT, 1),
        },
        Case {
            name: "unobservable on an excluded source: not counted",
            setup: |r| {
                let p = clean(r);
                r.sweep(p, "todo_marker", "unobservable");
            },
            domain: 1,
            want: nothing.clone(),
        },
        Case {
            name: "partial blocks",
            setup: |r| {
                let p = clean(r);
                r.sweep(p, "ci_red", "partial");
            },
            domain: 1,
            want: open.clone(),
        },
        Case {
            name: "failed blocks",
            setup: |r| {
                let p = clean(r);
                r.sweep(p, "ci_red", "failed");
            },
            domain: 1,
            want: open.clone(),
        },
        Case {
            name: "a missing row blocks",
            setup: |r| {
                let p = clean(r);
                drop_sweep(r, p, "ci_red");
            },
            domain: 1,
            want: open.clone(),
        },
        Case {
            name: "a stale skipped_suppressed on an enrolled project blocks",
            setup: |r| {
                let p = clean(r);
                r.sweep(p, "ci_red", "skipped_suppressed");
            },
            domain: 1,
            want: open.clone(),
        },
        Case {
            name: "a stale skipped_reference on a project in E blocks",
            setup: |r| {
                let p = clean(r);
                r.sweep(p, "no_release", "skipped_reference");
            },
            domain: 1,
            want: open.clone(),
        },
        Case {
            name: "an unverified item blocks",
            setup: |r| {
                let p = clean(r);
                r.item(p, "missing_readme", "unverified", "scored");
            },
            domain: 1,
            want: open.clone(),
        },
        Case {
            name: "an open scored item blocks",
            setup: |r| {
                let p = clean(r);
                r.item(p, "missing_readme", "open", "scored");
            },
            domain: 1,
            want: open,
        },
        Case {
            name: "a switched-off source's open and unverified items: excluded, not blocking",
            setup: |r| {
                let p = clean(r);
                r.item(p, "missing_readme", "open", "scored");
                r.item(p, "missing_readme", "unverified", "scored");
                switch_off(r, DebtSource::MissingReadme);
            },
            domain: 1,
            want: want(
                RingStatus::NothingToTend,
                &[DebtSource::TodoMarker, DebtSource::MissingReadme],
                0,
            ),
        },
        Case {
            name: "the same items with their switch on: open",
            setup: |r| {
                let p = clean(r);
                r.item(p, "missing_readme", "open", "scored");
                r.item(p, "missing_readme", "unverified", "scored");
            },
            domain: 1,
            want: want(RingStatus::Open, NO_CONSENT, 0),
        },
        Case {
            name: "an open shown_only item does not block",
            setup: |r| {
                let p = clean(r);
                r.item(p, "abandoned_with_debt", "open", "shown_only");
            },
            domain: 1,
            want: nothing,
        },
        Case {
            name: "every switch off: nothing to tend, all excluded",
            setup: |r| {
                let p = r.project(Flags::default());
                r.location(p, "present", None);
                r.enrol(p);
                for s in DebtSource::ALL {
                    switch_off(r, s);
                }
            },
            domain: 1,
            want: want(RingStatus::NothingToTend, &DebtSource::ALL, 0),
        },
    ]
}

/// The rows about the domain `E` and the earned close.
// A table of cases, one short row each, which rustfmt spreads over many lines.
#[allow(clippy::too_many_lines)]
fn domain_cases() -> Vec<Case> {
    let nothing = want(RingStatus::NothingToTend, NO_CONSENT, 0);
    let closed = want(RingStatus::Closed, NO_CONSENT, 0);
    vec![
        Case {
            name: "|E| = 0: not computed",
            setup: |r| {
                let p = r.project(Flags::default());
                r.location(p, "present", None);
            },
            domain: 0,
            want: want(RingStatus::NotComputed, NO_CONSENT, 0),
        },
        Case {
            name: "earned beats nothing to tend",
            setup: |r| {
                let p = clean(r);
                r.debt_day(p, NOON, 0, &["missing_readme"]);
            },
            domain: 1,
            want: closed.clone(),
        },
        Case {
            name: "a removed project's unverified items do not block",
            setup: |r| {
                clean(r);
                let gone = r.project(Flags {
                    removed_at: Some(1),
                    ..Flags::default()
                });
                r.location(gone, "present", None);
                r.enrol(gone);
                r.item(gone, "missing_readme", "unverified", "scored");
            },
            domain: 1,
            want: nothing.clone(),
        },
        Case {
            name: "a project whose only copy is removed leaves E",
            setup: |r| {
                clean(r);
                let gone = r.project(Flags::default());
                r.location(gone, "present", Some(1));
                r.enrol(gone);
                r.item(gone, "missing_readme", "unverified", "scored");
            },
            domain: 1,
            want: nothing.clone(),
        },
        Case {
            name: "an offline copy stays in E",
            setup: |r| {
                let p = clean(r);
                r.conn()
                    .execute(
                        "UPDATE location SET presence = 'offline' WHERE project_id = ?1",
                        [p.0],
                    )
                    .unwrap();
            },
            domain: 1,
            want: nothing.clone(),
        },
        Case {
            name: "a Reference project is outside E",
            setup: |r| {
                clean(r);
                let p = r.project(Flags {
                    reference: true,
                    ..Flags::default()
                });
                r.location(p, "present", None);
                r.enrol(p);
            },
            domain: 1,
            want: nothing.clone(),
        },
        Case {
            name: "an archived project is outside E",
            setup: |r| {
                clean(r);
                let p = r.project(Flags {
                    archived: true,
                    ..Flags::default()
                });
                r.location(p, "present", None);
                r.enrol(p);
            },
            domain: 1,
            want: nothing.clone(),
        },
        Case {
            name: "sealed after a debt day today: still closed",
            setup: sealed_after_tending,
            domain: 0,
            want: closed,
        },
        Case {
            name: "a legacy debt day priced at zero earns nothing",
            setup: |r| {
                let p = clean(r);
                r.debt_day(p, NOON, 0, &["abandoned_with_debt"]);
            },
            domain: 1,
            want: nothing,
        },
    ]
}

#[test]
fn ac_p4_39_6_the_tend_truth_table() {
    let today = DaySpan {
        first: TODAY,
        last: TODAY,
    };
    let mut mismatches = Vec::new();
    let cases: Vec<Case> = sweep_cases().into_iter().chain(domain_cases()).collect();
    eprintln!("case | E | T today | expected | got");
    for case in &cases {
        let rig = RingsRig::new();
        (case.setup)(&rig);
        let domain = tend_domain(rig.conn()).unwrap().len();
        let earned = !tend_days(rig.conn(), today).unwrap().is_empty();
        let got = tend_ring(rig.conn(), TODAY).unwrap();
        eprintln!(
            "{} | {domain} | {earned} | {:?} | {got:?}",
            case.name, case.want
        );
        if got != case.want || domain != case.domain {
            mismatches.push((case.name, case.domain, domain));
        }
    }
    eprintln!("tend cases: {}", cases.len());
    assert!(cases.len() >= 20, "the table lost rows");
    assert!(
        mismatches.is_empty(),
        "mismatched (case, E wanted, E got): {mismatches:?}"
    );

    // The Seal withdraws no witness: today stays active through the union over the same day.
    let rig = RingsRig::new();
    sealed_after_tending(&rig);
    let active = union_days(
        &git_days(rig.conn(), today).unwrap(),
        &focus_days(rig.conn(), today, 45 * 60, &UTC, NOON + 120).unwrap(),
        &tend_days(rig.conn(), today).unwrap(),
    );
    assert!(active.contains(&TODAY), "a Seal withdrew today's witness");
}
