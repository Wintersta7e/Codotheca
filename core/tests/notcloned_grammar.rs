//! AC-P2-23-7: `is:notcloned` and **the domain rule**, in the Rust engine.
//!
//! > A `has:` attribute, the working-copy `is:` flags, and `touched:` are predicates **about a
//! > working copy**. For a project with zero `location` rows every one of them evaluates to
//! > **Unknown**.
//!
//! `TermTruth::Unknown` matches neither polarity, so `-has:remote` keeps meaning *a local copy
//! with no remote configured* and never silently acquires the whole not-cloned tail. Without the
//! rule, the day zero-location rows enter the index a real triage query would return every
//! not-cloned project in the library — **parsing identically while answering differently**, which
//! is the failure this rule exists to prevent.
#![cfg(feature = "testkit")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::collections::BTreeMap;

use codotheca_core::projects::rows::{LoadedRow, RowFacts};
use codotheca_core::protocol::ProjectRow;
use codotheca_core::query::execute::{evaluate_query, term_truth, ExecContext, TermTruth};
use codotheca_core::query::parse_query;

const NOW: i64 = 1_781_000_000;

/// Id 1 — no working copy, and a `remote_key`, which is the case that makes `has:remote` the
/// inversion §23.6 names: in phase 2 every not-cloned project carries one by construction.
fn not_cloned() -> LoadedRow {
    let mut row = ProjectRow::for_test_not_cloned(1);
    row.primary_language = Some("Rust".to_owned());
    row.last_touched_at = NOW - 10 * 86_400;
    LoadedRow {
        row,
        facts: RowFacts {
            has_remote: true,
            ..RowFacts::default()
        },
    }
}

/// Id 2 — a working copy **with** a remote.
fn located_with_remote() -> LoadedRow {
    let mut row = ProjectRow::for_test(2);
    row.last_touched_at = NOW - 10 * 86_400;
    LoadedRow {
        row,
        facts: RowFacts {
            has_remote: true,
            ..RowFacts::default()
        },
    }
}

/// Id 3 — a working copy and **no** remote: the row `-has:remote` is a triage query about.
fn located_without_remote() -> LoadedRow {
    let mut row = ProjectRow::for_test(3);
    row.last_touched_at = NOW - 10 * 86_400;
    LoadedRow {
        row,
        facts: RowFacts::default(),
    }
}

const fn ctx(names: &BTreeMap<String, i64>) -> ExecContext<'_> {
    ExecContext {
        now: NOW,
        tz_offset_min: 0,
        first_run_completed_at: None,
        collection_ids_by_name: names,
        paths_are_case_sensitive: false,
        commit_subject_hits: None,
    }
}

fn ids(rows: &[LoadedRow], query: &str, ctx: &ExecContext<'_>) -> Vec<i64> {
    let ast = parse_query(query);
    evaluate_query(rows, &ast, ctx)
        .rows
        .iter()
        .map(|r| r.row.id.0)
        .collect()
}

#[test]
fn the_domain_rule_and_is_notcloned_over_one_fixture_set() {
    let names = BTreeMap::new();
    let ctx = ctx(&names);
    let rows = vec![
        not_cloned(),
        located_with_remote(),
        located_without_remote(),
    ];
    let bare = &rows[0];

    let mut terms_evaluated = 0_usize;

    // The five terms whose answer §23.6 changes, plus the family they belong to. Each must match
    // the zero-location row under **neither** polarity.
    let outside_domain = [
        "has:remote",
        "has:submodules",
        "has:readme",
        "has:stash",
        "is:bare",
        "is:shallow",
        "is:dirty",
        "is:unpushed",
        "is:behind",
        "is:interrupted",
        "is:empty",
        "is:local",
        "is:wsl",
        "touched:>1d",
        "touched:<1d",
    ];
    for query in outside_domain {
        let ast = parse_query(query);
        assert_eq!(ast.terms.len(), 1, "{query} did not parse to one term");
        assert_eq!(
            term_truth(bare, &ast.terms[0], &ctx),
            TermTruth::Unknown,
            "{query} is a predicate about a working copy and there is none"
        );
        terms_evaluated += 1;
        assert!(
            !ids(&rows, query, &ctx).contains(&1),
            "{query} matched the zero-location row"
        );
        assert!(
            !ids(&rows, &format!("-{query}"), &ctx).contains(&1),
            "-{query} matched the zero-location row"
        );
        terms_evaluated += 1;
    }

    // The project-row facts stay known — the rule is about a working copy, not about the row.
    for query in [
        "is:archived",
        "is:pinned",
        "is:hidden",
        "is:reference",
        "is:fork",
        "is:notcloned",
    ] {
        let ast = parse_query(query);
        assert_eq!(ast.terms.len(), 1, "{query} did not parse to one term");
        assert_ne!(
            term_truth(bare, &ast.terms[0], &ctx),
            TermTruth::Unknown,
            "{query} is a project-row fact and stays known"
        );
        terms_evaluated += 1;
    }

    // `is:notcloned` returns exactly the zero-location rows and `-is:notcloned` exactly the rest.
    assert_eq!(ids(&rows, "is:notcloned", &ctx), vec![1]);
    assert_eq!(ids(&rows, "-is:notcloned", &ctx), vec![2, 3]);

    // The inversion this rule exists to prevent: `-has:remote` means *a local copy with no
    // remote configured*, and must never silently acquire the whole not-cloned tail.
    assert_eq!(ids(&rows, "-has:remote", &ctx), vec![3]);
    assert_eq!(ids(&rows, "has:remote", &ctx), vec![2]);

    // Not-cloned rows are in §8.0b's base set, so a bare query returns them.
    assert_eq!(ids(&rows, "", &ctx), vec![1, 2, 3]);

    // `lang:rust` correctly starts matching blueprint tiles.
    assert_eq!(ids(&rows, "lang:rust", &ctx), vec![1]);

    eprintln!(
        "notcloned_grammar: {} rows, {terms_evaluated} term evaluations",
        rows.len()
    );
    assert!(!rows.is_empty(), "a run over no rows proves nothing");
    assert!(
        terms_evaluated > 0,
        "a run that evaluated no term proves nothing"
    );
}

/// `is_bare` and `is_shallow` keep storing `0` — both are `NOT NULL DEFAULT 0`, and making them
/// nullable is the `project` rebuild §23.1's shape exists to avoid. **The domain rule is
/// evaluated before the column is read**, so the stored `0` is never observable through the
/// grammar and no rebuild is paid. This asserts exactly that: the column still says `false`, and
/// the grammar still says Unknown.
#[test]
fn the_stored_zero_is_unobservable_rather_than_removed() {
    let names = BTreeMap::new();
    let ctx = ctx(&names);
    let bare = not_cloned();
    assert!(!bare.row.is_bare, "the column still stores 0");
    assert!(!bare.row.is_shallow);
    for query in ["is:bare", "is:shallow"] {
        let ast = parse_query(query);
        assert_eq!(term_truth(&bare, &ast.terms[0], &ctx), TermTruth::Unknown);
    }
}

/// `last_touched_at` keeps its `created_at` fallback: it stays a non-null `Timestamp` on the wire
/// and it orders rows **within** the not-cloned tail. `touched:` simply does not read it for
/// those rows. Filling it from a remote push time would be a third clock merged into the first.
#[test]
fn last_touched_at_still_orders_the_tail_and_touched_still_refuses_to_read_it() {
    let names = BTreeMap::new();
    let ctx = ctx(&names);
    let mut bare = not_cloned();
    bare.row.last_touched_at = NOW - 900 * 86_400;
    assert!(bare.row.last_touched_at > 0, "the fallback is not epoch 0");
    for query in ["touched:>1d", "touched:<1d", "touched:2024"] {
        let ast = parse_query(query);
        assert_eq!(
            term_truth(&bare, &ast.terms[0], &ctx),
            TermTruth::Unknown,
            "{query} read a clock that dates no interaction"
        );
    }
}
