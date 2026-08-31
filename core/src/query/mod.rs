//! The second consumer of §8.3's one production. The corpus in `protocol/query/corpus.json` is
//! what keeps this parser and `app/src/shared/query/parse.ts` from diverging.

pub mod ast;
pub mod execute;
pub mod parse;

pub use ast::{
    Cmp, HasAttribute, IgnoredReason, IgnoredTerm, IsFlag, QueryAst, QueryTerm, TextField,
};
pub use parse::parse_query;

/// Mirrors `queryGrammarVersion` in `protocol/query/grammar.json`; the tests assert the equality.
pub const QUERY_GRAMMAR_VERSION: u32 = 1;
