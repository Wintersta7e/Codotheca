//! The executor R37 found missing: a `QueryAst` plus loaded rows becomes a matched set, and
//! unknown is matched by no polarity.
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
use codotheca_core::query::execute::{
    evaluate_query, in_base_set, term_truth, ExecContext, TermTruth,
};
use codotheca_core::query::parse_query;

const NOW: i64 = 1_781_000_000;

/// A row with every observable field unset — the never-indexed repository §11.1 describes.
fn blank(id: i64) -> LoadedRow {
    LoadedRow {
        row: ProjectRow::for_test(id),
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

#[test]
fn an_unobserved_worktree_is_unknown_and_is_not_matched_by_either_polarity() {
    let names = BTreeMap::new();
    let ctx = ctx(&names);
    let row = blank(1);
    let dirty = parse_query("is:dirty");
    let clean = parse_query("-is:dirty");
    assert_eq!(term_truth(&row, &dirty.terms[0], &ctx), TermTruth::Unknown);
    // This is the whole ruling: SQL would put the row in exactly one of these two answers.
    assert!(evaluate_query(std::slice::from_ref(&row), &dirty, &ctx)
        .rows
        .is_empty());
    assert!(evaluate_query(std::slice::from_ref(&row), &clean, &ctx)
        .rows
        .is_empty());
}

/// **[p3] §29.4 landed the producer, so the term stopped being ignored** — `answerable()` was
/// amended in the same change, which is §23.6's own rule. What an unscanned project answers is
/// `Unknown`, not a false: it is excluded from `has:ci` and from `-has:ci` alike.
#[test]
fn a_term_whose_producer_has_not_run_is_unknown_rather_than_ignored() {
    let names = BTreeMap::new();
    let ctx = ctx(&names);
    let rows = vec![blank(1)];
    for query in [
        "has:ci",
        "-has:ci",
        "has:license",
        "has:tests",
        "has:readme",
    ] {
        let out = evaluate_query(&rows, &parse_query(query), &ctx);
        assert!(out.ignored.is_empty(), "{query} is still an ignored term");
        assert!(
            out.rows.is_empty(),
            "{query} answered a project J7 has never observed"
        );
    }
    // `has:remote` and `has:submodules` read different columns and are unaffected.
    let answered = evaluate_query(&rows, &parse_query("has:remote"), &ctx);
    assert!(answered.ignored.is_empty());
    assert!(answered.rows.is_empty());
}

/// **[p3] §31.1 landed the producer, so the term stopped being ignored** — the same shape §29.4
/// already applied to the four `has:` file terms, and the same rule: a term leaves the ignored
/// set in the change that gives it a value to read.
#[test]
fn completion_reaches_the_executor_now_that_something_computes_it() {
    let ast = parse_query("completion:>5");
    assert_eq!(ast.terms.len(), 1);
    assert!(ast.ignored.is_empty());
}

#[test]
fn the_base_predicate_hides_reference_and_hidden_until_the_query_asks_for_them() {
    let names = BTreeMap::new();
    let ctx = ctx(&names);
    let mut reference = blank(1);
    reference.row.is_reference = true;
    let mut hidden = blank(2);
    hidden.row.is_hidden = true;
    let rows = vec![reference, hidden, blank(3)];

    let bare = parse_query("");
    assert_eq!(evaluate_query(&rows, &bare, &ctx).rows.len(), 1);
    assert!(!in_base_set(&rows[0].row, &bare));
    assert!(in_base_set(&rows[0].row, &parse_query("is:reference")));
    // A negated term does not opt them in — `-is:hidden` is not a request for hidden rows.
    assert!(!in_base_set(&rows[1].row, &parse_query("-is:hidden")));
}

#[test]
fn is_new_is_unknown_until_first_run_has_a_stamp() {
    let names = BTreeMap::new();
    let ctx = ctx(&names);
    let row = blank(1);
    assert_eq!(
        term_truth(&row, &parse_query("is:new").terms[0], &ctx),
        TermTruth::Unknown
    );
}

#[test]
fn touched_age_cuts_where_the_typescript_engine_cuts() {
    // The renderer divides by a day and compares floats; this compares seconds. The two must
    // agree at the boundary, which is the only place the two forms could differ.
    let names = BTreeMap::new();
    let ctx = ctx(&names);
    let day = 86_400_i64;
    let mut exact = blank(1);
    exact.row.last_touched_at = NOW - 365 * day;
    assert_eq!(
        term_truth(&exact, &parse_query("touched:>365d").terms[0], &ctx),
        TermTruth::False,
        "exactly 365 days old is not older than 365 days"
    );
    let mut older = blank(2);
    older.row.last_touched_at = NOW - 365 * day - 1;
    assert_eq!(
        term_truth(&older, &parse_query("touched:>365d").terms[0], &ctx),
        TermTruth::True
    );
}

#[test]
fn a_collection_name_nothing_maps_matches_nothing_and_is_not_unknown_state() {
    let mut names = BTreeMap::new();
    names.insert("side projects".to_owned(), 4_i64);
    let ctx = ctx(&names);
    let mut member = blank(1);
    member.row.collection_ids = vec![codotheca_core::protocol::CollectionId(4)];
    assert_eq!(
        term_truth(
            &member,
            &parse_query("collection:\"side projects\"").terms[0],
            &ctx
        ),
        TermTruth::True
    );
    assert_eq!(
        term_truth(&member, &parse_query("collection:nosuch").terms[0], &ctx),
        TermTruth::False
    );
}

// ---------------------------------------------------------------------------------------------
// [p3] §31.1 — `completion:` filters, and the NULL half hardens
// ---------------------------------------------------------------------------------------------

/// **The invariant, asserted where it lives.**
///
/// §31.10 and this plan both describe it as *the generated SQL contains no `COALESCE`*. There is
/// no generated SQL: `core/src/query/execute.rs` evaluates term by term over loaded rows, so the
/// coercion this forbids would be a `unwrap_or(0)` in that evaluator rather than a cast in a
/// statement. Asserting the **answer** is stronger than asserting the text that would have
/// produced it — a NULL row matches neither comparison, whatever the implementation.
#[test]
fn a_null_projection_matches_neither_completion_comparison() {
    let names = BTreeMap::new();
    let ctx = ctx(&names);
    let row = blank(1);
    assert_eq!(row.row.completion_lit, None, "the fixture is the NULL case");

    for query in [
        "completion:>5",
        "completion:<5",
        "-completion:>5",
        "-completion:<5",
    ] {
        let ast = parse_query(query);
        assert_eq!(ast.terms.len(), 1, "{query} did not parse as a term");
        assert_eq!(
            term_truth(&row, &ast.terms[0], &ctx),
            TermTruth::Unknown,
            "{query} coerced a NULL projection to a number"
        );
        assert!(
            evaluate_query(std::slice::from_ref(&row), &ast, &ctx)
                .rows
                .is_empty(),
            "{query} matched a row whose completion was never computed"
        );
    }
}

/// A computed row answers both comparisons, so the NULL case above is not passing because the
/// term never matches anything.
#[test]
fn a_computed_projection_filters_on_both_sides_of_the_bound() {
    let names = BTreeMap::new();
    let ctx = ctx(&names);
    let mut row = blank(1);
    row.row.completion_lit = Some(8);
    row.row.completion_applicable = Some(10);

    let above = parse_query("completion:>5");
    let below = parse_query("completion:<5");
    assert_eq!(term_truth(&row, &above.terms[0], &ctx), TermTruth::True);
    assert_eq!(term_truth(&row, &below.terms[0], &ctx), TermTruth::False);
    assert_eq!(
        evaluate_query(std::slice::from_ref(&row), &above, &ctx)
            .rows
            .len(),
        1
    );
    assert!(evaluate_query(std::slice::from_ref(&row), &below, &ctx)
        .rows
        .is_empty());

    // And the negation, which a NULL row is still matched by neither of.
    let negated = parse_query("-completion:>5");
    assert!(evaluate_query(std::slice::from_ref(&row), &negated, &ctx)
        .rows
        .is_empty());
}

/// **No `COALESCE`, no `IFNULL`, no `, 0`, and no `unwrap_or` on the column** — asserted over the
/// evaluator's own source, because a coercion is invisible in the result set of a fixture that
/// happens to hold no NULL row.
#[test]
fn the_completion_evaluator_coerces_no_null() {
    let source = include_str!("../src/query/execute.rs");
    let lines = source.lines().count();
    eprintln!("query_execute: {lines} lines of the evaluator read");
    assert!(lines > 0, "a run that read nothing is a failing run");

    let arm = source
        .split_once("QueryTerm::Completion { op, value, .. }")
        .map(|(_, rest)| &rest[..rest.len().min(400)])
        .expect("the completion arm");
    for banned in ["COALESCE", "IFNULL", "unwrap_or(0)", "unwrap_or_default"] {
        assert!(
            !arm.contains(banned),
            "the completion arm names {banned}, which renders unknown as zero"
        );
    }
    assert!(arm.contains("TermTruth::Unknown"), "NULL must stay Unknown");
}
