//! The Rust half of §8.3's one production, held to `protocol/query/corpus.json`.
//!
//! The corpus, not the version number, is what stops the two engines diverging: both parsers
//! read the same file and must emit the same JSON for every case in it.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use codotheca_core::query::parse_query;
use serde_json::Value;

fn corpus() -> Value {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../protocol/query/corpus.json");
    let text = std::fs::read_to_string(path).expect("corpus.json is readable");
    serde_json::from_str(&text).expect("corpus.json is JSON")
}

#[test]
fn rust_parser_matches_every_corpus_case() {
    let doc = corpus();
    let cases = doc["cases"].as_array().expect("cases is an array");
    assert!(cases.len() >= 20, "the corpus must exercise every branch");

    let mut failures: Vec<String> = Vec::new();
    for case in cases {
        let name = case["name"].as_str().expect("name");
        let query = case["query"].as_str().expect("query");
        let produced = serde_json::to_value(parse_query(query)).expect("serialisable");
        if produced != case["ast"] {
            failures.push(format!(
                "{name}\n  expected {}\n  produced {}",
                case["ast"], produced
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "corpus divergence:\n{}",
        failures.join("\n")
    );
}

#[test]
fn rust_tables_transcribe_the_production() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../protocol/query/grammar.json"
    );
    let text = std::fs::read_to_string(path).expect("grammar.json is readable");
    let doc: Value = serde_json::from_str(&text).expect("grammar.json is JSON");

    assert_eq!(
        doc["queryGrammarVersion"].as_u64().expect("version"),
        u64::from(codotheca_core::query::QUERY_GRAMMAR_VERSION)
    );
    let flags: Vec<String> = doc["is"]
        .as_array()
        .expect("is")
        .iter()
        .map(|v| v.as_str().expect("string").to_owned())
        .collect();
    // Every value in the production parses; nothing outside it does.
    for flag in &flags {
        let ast = parse_query(&format!("is:{flag}"));
        assert_eq!(ast.terms.len(), 1, "is:{flag} must parse");
        assert!(ast.ignored.is_empty(), "is:{flag} must not soft-error");
    }
    assert_eq!(parse_query("is:mine").terms.len(), 0);
}
