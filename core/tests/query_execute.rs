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

fn ctx(names: &BTreeMap<String, i64>) -> ExecContext<'_> {
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

#[test]
fn completion_never_reaches_the_executor_because_the_parser_already_ignored_it() {
    let ast = parse_query("completion:>5");
    assert!(ast.terms.is_empty());
    assert_eq!(ast.ignored.len(), 1);
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
