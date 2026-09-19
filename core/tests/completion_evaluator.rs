//! §31.1a's ten predicates as a **truth table**, and the mechanical half of *never both*.
//!
//! The evaluator is pure, so every branch is reachable without a fixture repository. What that
//! buys is the second test in this file: a source-level walk proving the six predicates R124 gave
//! §28 are not re-derived here. A reviewer cannot see a duplicated predicate; a walk can.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::{Path, PathBuf};

use codotheca_core::completion::evaluate::{
    evaluate, CheckRow, CompletionInputs, Counts, DepsReading, SingletonReading,
};
use codotheca_core::jobs::presence::PresenceState;
use codotheca_core::protocol::{
    CheckState, CompletionCheck, DebtSource, DebtSweepOutcome, DependencyVerdict, RemoteFactsState,
    UnknownReason,
};

const NOW: i64 = 1_700_000_000;

/// A project every check can be scored on: swept, observed, remote-bound and read.
fn baseline() -> CompletionInputs {
    let complete = |source| SingletonReading {
        source,
        outcome: Some(DebtSweepOutcome::Complete),
        open_items: 0,
    };
    CompletionInputs {
        readme: complete(DebtSource::MissingReadme),
        license: complete(DebtSource::MissingLicense),
        tests: complete(DebtSource::MissingTests),
        ci_red: complete(DebtSource::CiRed),
        pushed: complete(DebtSource::UnpushedCommits),
        release: complete(DebtSource::NoRelease),
        has_ci: Some(PresenceState::Present),
        remote_configured: Some(true),
        account_connected: true,
        facts_state: RemoteFactsState::Observed,
        has_remote: true,
        forge_description: Some("a shaped description".to_owned()),
        topic_count: 2,
        deps: DepsReading {
            verdict: DependencyVerdict::Clean,
            scored_open: 0,
            lockfile_not_read: false,
        },
        archetype: Some("library".to_owned()),
        user_na: [None; 10],
    }
}

fn row(rows: &[CheckRow; 10], key: CompletionCheck) -> CheckRow {
    *rows.iter().find(|r| r.key == key).expect("ten rows")
}

fn state(rows: &[CheckRow; 10], key: CompletionCheck) -> CheckState {
    row(rows, key).state
}

/// Every case asserts §31.1b's partition, so a state outside the four cannot slip through.
fn checked(inputs: &CompletionInputs) -> [CheckRow; 10] {
    let rows = evaluate(inputs, NOW);
    let counts = Counts::of(&rows);
    assert_eq!(counts.evaluable + counts.unknown + counts.na, 10);
    for r in &rows {
        assert_eq!(
            r.unknown_reason.is_some(),
            r.state == CheckState::Unknown,
            "{:?}: a reason is present exactly when the state is unknown",
            r.key
        );
        assert_eq!(r.observed_at, NOW);
    }
    rows
}

#[test]
fn the_rows_are_ten_in_declaration_order() {
    let rows = checked(&baseline());
    let keys: Vec<CompletionCheck> = rows.iter().map(|r| r.key).collect();
    assert_eq!(keys, CompletionCheck::ALL.to_vec());
}

#[test]
fn a_fully_observed_project_scores_ten_of_ten() {
    let rows = checked(&baseline());
    let counts = Counts::of(&rows);
    assert_eq!(
        counts,
        Counts {
            lit: 10,
            evaluable: 10,
            unknown: 0,
            na: 0
        }
    );
}

/// **Group A, one case per `DebtSweepOutcome` variant, enumerated from the generated enum**
/// rather than from a literal six.
#[test]
fn every_sweep_outcome_maps_for_every_group_a_key() {
    let group_a = [
        (CompletionCheck::Readme, DebtSource::MissingReadme),
        (CompletionCheck::License, DebtSource::MissingLicense),
        (CompletionCheck::Tests, DebtSource::MissingTests),
        (CompletionCheck::Pushed, DebtSource::UnpushedCommits),
        (CompletionCheck::CiGreen, DebtSource::CiRed),
        (CompletionCheck::Release, DebtSource::NoRelease),
    ];
    let mut cases = 0_u32;

    for (key, source) in group_a {
        // Row absent is *never observed*, and it is neither of the outcomes.
        let mut inputs = baseline();
        put(&mut inputs, key, SingletonReading::never_observed(source));
        let rows = checked(&inputs);
        assert_eq!(state(&rows, key), CheckState::Unknown, "{key:?} row-absent");
        assert_eq!(
            row(&rows, key).unknown_reason,
            Some(UnknownReason::NotRunYet),
            "{key:?} row-absent"
        );
        cases += 1;

        for outcome in DebtSweepOutcome::ALL {
            for open in [0_u32, 1] {
                let mut inputs = baseline();
                put(
                    &mut inputs,
                    key,
                    SingletonReading {
                        source,
                        outcome: Some(outcome),
                        open_items: open,
                    },
                );
                let rows = checked(&inputs);
                let got = row(&rows, key);
                match outcome {
                    DebtSweepOutcome::Complete if open >= 1 => {
                        assert_eq!(got.state, CheckState::Fail, "{key:?} {outcome:?}");
                    }
                    DebtSweepOutcome::Complete => {
                        assert_eq!(got.state, CheckState::Pass, "{key:?} {outcome:?}");
                    }
                    _ => {
                        assert_eq!(
                            got.state,
                            CheckState::Unknown,
                            "{key:?} {outcome:?} is never a fail"
                        );
                        assert!(got.unknown_reason.is_some(), "{key:?} {outcome:?}");
                    }
                }
                cases += 1;
            }
        }
    }

    eprintln!("group-A cases run: {cases}");
    assert!(cases > 0, "a run with no case is a failing run");
}

fn put(inputs: &mut CompletionInputs, key: CompletionCheck, reading: SingletonReading) {
    match key {
        CompletionCheck::Readme => inputs.readme = reading,
        CompletionCheck::License => inputs.license = reading,
        CompletionCheck::Tests => inputs.tests = reading,
        CompletionCheck::Pushed => inputs.pushed = reading,
        CompletionCheck::CiGreen => inputs.ci_red = reading,
        CompletionCheck::Release => inputs.release = reading,
        other => panic!("{other:?} is not a Group-A key"),
    }
}

/// §31.7a's per-key reason, which is §31's alone because `debt_sweep` carries none.
#[test]
fn an_unobservable_group_a_source_gets_its_own_reason() {
    let expected = [
        (CompletionCheck::Readme, UnknownReason::NotRead),
        (CompletionCheck::License, UnknownReason::NotRead),
        (CompletionCheck::Tests, UnknownReason::NotRead),
        (CompletionCheck::Pushed, UnknownReason::NotObserved),
        (CompletionCheck::Release, UnknownReason::NotObserved),
    ];
    for (key, reason) in expected {
        let mut inputs = baseline();
        let source = match key {
            CompletionCheck::Readme => DebtSource::MissingReadme,
            CompletionCheck::License => DebtSource::MissingLicense,
            CompletionCheck::Tests => DebtSource::MissingTests,
            CompletionCheck::Pushed => DebtSource::UnpushedCommits,
            _ => DebtSource::NoRelease,
        };
        put(
            &mut inputs,
            key,
            SingletonReading {
                source,
                outcome: Some(DebtSweepOutcome::Unobservable),
                open_items: 0,
            },
        );
        let rows = checked(&inputs);
        assert_eq!(row(&rows, key).unknown_reason, Some(reason), "{key:?}");
    }

    // `ciGreen`'s reason depends on whether an account is connected at all: a user owed an
    // account is owed a different sentence from a user owed a sync.
    for (connected, reason) in [
        (false, UnknownReason::NeedsAccount),
        (true, UnknownReason::NotSynced),
    ] {
        let mut inputs = baseline();
        inputs.account_connected = connected;
        inputs.ci_red = SingletonReading {
            source: DebtSource::CiRed,
            outcome: Some(DebtSweepOutcome::Unobservable),
            open_items: 0,
        };
        let rows = checked(&inputs);
        assert_eq!(
            row(&rows, CompletionCheck::CiGreen).unknown_reason,
            Some(reason)
        );
    }
}

/// `ci` is Group B and is the one content check with no `DebtSource`, so §31 maps §29's
/// tri-state itself.
#[test]
fn ci_maps_the_content_tri_state_and_row_absence_separately() {
    let cases = [
        (Some(PresenceState::Present), CheckState::Pass, None),
        (Some(PresenceState::Absent), CheckState::Fail, None),
        (
            Some(PresenceState::NotRead),
            CheckState::Unknown,
            Some(UnknownReason::NotRead),
        ),
        (None, CheckState::Unknown, Some(UnknownReason::NotRunYet)),
    ];
    let mut run = 0_u32;
    for (presence, want, reason) in cases {
        let mut inputs = baseline();
        inputs.has_ci = presence;
        // `ciGreen` is `na` when `ci` is `na` or `fail`, so a fixture that only looked at `ci`
        // would miss the coupling.
        let rows = checked(&inputs);
        assert_eq!(state(&rows, CompletionCheck::Ci), want, "{presence:?}");
        assert_eq!(
            row(&rows, CompletionCheck::Ci).unknown_reason,
            reason,
            "{presence:?}"
        );
        if want == CheckState::Fail {
            assert_eq!(
                state(&rows, CompletionCheck::CiGreen),
                CheckState::Na,
                "a green run on a project with no CI config is not a claim this check may make"
            );
        }
        run += 1;
    }
    eprintln!("ci tri-state cases run: {run}");
    assert_eq!(run, 4);
}

/// §31.1a: `remote` is `unknown`/`notObserved` with no working copy, never `fail`.
#[test]
fn remote_is_never_a_failure_without_an_observation() {
    for (configured, want) in [
        (Some(true), CheckState::Pass),
        (Some(false), CheckState::Fail),
        (None, CheckState::Unknown),
    ] {
        let mut inputs = baseline();
        inputs.remote_configured = configured;
        let rows = checked(&inputs);
        assert_eq!(state(&rows, CompletionCheck::Remote), want);
        if want == CheckState::Unknown {
            assert_eq!(
                row(&rows, CompletionCheck::Remote).unknown_reason,
                Some(UnknownReason::NotObserved)
            );
        }
    }
}

/// `description` reads the **forge's own row** and is `na` when there is no remote at all.
#[test]
fn description_needs_a_description_and_a_topic_and_is_na_without_a_remote() {
    let mut no_remote = baseline();
    no_remote.has_remote = false;
    assert_eq!(
        state(&checked(&no_remote), CompletionCheck::Description),
        CheckState::Na
    );

    for (description, topics, want) in [
        (Some("shaped"), 1_u32, CheckState::Pass),
        (Some("shaped"), 0, CheckState::Fail),
        (None, 3, CheckState::Fail),
        // A description of only whitespace is not a description.
        (Some("   "), 3, CheckState::Fail),
    ] {
        let mut inputs = baseline();
        inputs.forge_description = description.map(ToOwned::to_owned);
        inputs.topic_count = topics;
        assert_eq!(
            state(&checked(&inputs), CompletionCheck::Description),
            want,
            "{description:?} / {topics}"
        );
    }

    for (facts, reason) in [
        (RemoteFactsState::NoAccount, UnknownReason::NeedsAccount),
        (RemoteFactsState::NotObserved, UnknownReason::NotSynced),
        (RemoteFactsState::NotPermitted, UnknownReason::NotSynced),
    ] {
        let mut inputs = baseline();
        inputs.facts_state = facts;
        let rows = checked(&inputs);
        assert_eq!(
            state(&rows, CompletionCheck::Description),
            CheckState::Unknown
        );
        assert_eq!(
            row(&rows, CompletionCheck::Description).unknown_reason,
            Some(reason),
            "{facts:?}"
        );
    }
}

/// **R131/F7's second half**: a lockfile the read could not take is `notRead`, never `notSynced`.
#[test]
fn deps_separates_an_unread_lockfile_from_an_unreachable_source() {
    let mut vulnerable = baseline();
    vulnerable.deps = DepsReading {
        verdict: DependencyVerdict::Vulnerable,
        scored_open: 2,
        lockfile_not_read: false,
    };
    assert_eq!(
        state(&checked(&vulnerable), CompletionCheck::Deps),
        CheckState::Fail
    );

    for (lockfile, reason) in [
        (true, UnknownReason::NotRead),
        (false, UnknownReason::NotSynced),
    ] {
        let mut inputs = baseline();
        inputs.deps = DepsReading {
            verdict: DependencyVerdict::Unknown,
            scored_open: 0,
            lockfile_not_read: lockfile,
        };
        let rows = checked(&inputs);
        assert_eq!(state(&rows, CompletionCheck::Deps), CheckState::Unknown);
        assert_eq!(
            row(&rows, CompletionCheck::Deps).unknown_reason,
            Some(reason)
        );
    }
}

/// §31.4: two stored facts. `user_na = 0` overrides a proposal and the check is evaluated.
#[test]
fn the_na_gate_runs_before_every_read() {
    let ci_index = CompletionCheck::ALL
        .iter()
        .position(|k| *k == CompletionCheck::Ci)
        .unwrap();

    // A documentation project proposes four keys N/A.
    let mut docs = baseline();
    docs.archetype = Some("docs".to_owned());
    let rows = checked(&docs);
    let na: Vec<CompletionCheck> = rows
        .iter()
        .filter(|r| r.state == CheckState::Na)
        .map(|r| r.key)
        .collect();
    assert_eq!(
        na,
        vec![
            CompletionCheck::Tests,
            CompletionCheck::Ci,
            // `ciGreen` follows `ci` being N/A, which is §31's own rule and not a proposal.
            CompletionCheck::CiGreen,
            CompletionCheck::Deps,
            CompletionCheck::Release,
        ]
    );

    // The user overrides the proposal for `ci`, and the check is evaluated again — as is
    // `ciGreen`, which was only N/A derivatively.
    let mut overridden = docs.clone();
    overridden.user_na[ci_index] = Some(false);
    let rows = checked(&overridden);
    assert_eq!(state(&rows, CompletionCheck::Ci), CheckState::Pass);
    assert_eq!(state(&rows, CompletionCheck::CiGreen), CheckState::Pass);
    assert_eq!(row(&rows, CompletionCheck::Ci).user_na, Some(false));

    // And a user ruling on a project whose archetype proposes nothing.
    let mut ruled = baseline();
    let readme_index = CompletionCheck::ALL
        .iter()
        .position(|k| *k == CompletionCheck::Readme)
        .unwrap();
    ruled.user_na[readme_index] = Some(true);
    let rows = checked(&ruled);
    assert_eq!(state(&rows, CompletionCheck::Readme), CheckState::Na);
    assert_eq!(row(&rows, CompletionCheck::Readme).user_na, Some(true));
}

// ---------------------------------------------------------------------------------------------
// The no-second-predicate gate
// ---------------------------------------------------------------------------------------------

fn completion_src() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src/completion")
}

fn rust_sources(root: &Path) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                // A file that vanished between the walk and the read is skipped BEFORE it is
                // counted, so the "scanned nothing" guard keeps meaning what it says.
                if let Ok(text) = std::fs::read_to_string(&path) {
                    out.push((path.display().to_string(), text));
                }
            }
        }
    }
    out
}

/// **The mechanical half of *never both*** (R124).
///
/// Six of the ten checks read §28's stored answer, so the inputs to those predicates must not
/// appear in this module at all. A reviewer cannot see a duplicated predicate; a walk can.
#[test]
fn the_completion_module_re_derives_no_predicate_of_section_28() {
    let banned = [
        "has_readme",
        "has_license",
        "has_tests",
        "tag_count",
        "ahead",
        "remote_ci_run",
    ];
    let sources = rust_sources(&completion_src());
    eprintln!(
        "completion_evaluator: walked {} file(s) under core/src/completion",
        sources.len()
    );
    assert!(
        !sources.is_empty(),
        "the walk read no file, so it proved nothing"
    );

    let mut offences = Vec::new();
    for (path, text) in &sources {
        for line in text.lines() {
            // The doc comments explain which predicates are §28's; naming them in prose is why
            // the rule survives, so the scan is over code and not over the whole file.
            let trimmed = line.trim_start();
            if trimmed.starts_with("//") {
                continue;
            }
            for word in banned {
                if line.contains(word) {
                    offences.push(format!("{path}: {word}"));
                }
            }
        }
    }
    assert!(
        offences.is_empty(),
        "§31 re-derives a predicate R124 gave §28: {offences:?}"
    );
}
