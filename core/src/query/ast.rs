//! The serde types that serialise byte-identically to `app/src/shared/query/ast.ts`.
//!
//! **R13: this plan owns both language mirrors**, and R24 makes the pair legitimate on one
//! condition — a test reads the other side. `core/tests/query_corpus.rs` is that test: it runs
//! `protocol/query/corpus.json`, the same file the TypeScript suite runs.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Cmp {
    Lt,
    Gt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TextField {
    Lang,
    Owner,
    In,
    Collection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum IsFlag {
    Dirty,
    Unpushed,
    Behind,
    Interrupted,
    Archived,
    Pinned,
    Hidden,
    Reference,
    Bare,
    Fork,
    Empty,
    Shallow,
    Local,
    Wsl,
    New,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HasAttribute {
    Readme,
    License,
    Tests,
    Ci,
    Remote,
    Stash,
    Submodules,
}

/// The three the *parser* can produce. `notAvailable` is renderer-only — only a projection can
/// lack a field the index holds — so it never crosses the wire and has no variant here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum IgnoredReason {
    UnknownField,
    NotComputed,
    MalformedValue,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IgnoredTerm {
    pub text: String,
    pub reason: IgnoredReason,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum QueryTerm {
    Bare {
        negated: bool,
        text: String,
    },
    Text {
        negated: bool,
        field: TextField,
        value: String,
        quoted: bool,
    },
    Flag {
        negated: bool,
        flag: IsFlag,
    },
    Has {
        negated: bool,
        attribute: HasAttribute,
    },
    Size {
        negated: bool,
        op: Cmp,
        bytes: u64,
    },
    TouchedAge {
        negated: bool,
        op: Cmp,
        days: u32,
    },
    TouchedYear {
        negated: bool,
        year: u32,
    },
}

impl QueryTerm {
    /// Whether the term is negated, without matching seven variants at every call site.
    #[must_use]
    pub const fn negated(&self) -> bool {
        match self {
            Self::Bare { negated, .. }
            | Self::Text { negated, .. }
            | Self::Flag { negated, .. }
            | Self::Has { negated, .. }
            | Self::Size { negated, .. }
            | Self::TouchedAge { negated, .. }
            | Self::TouchedYear { negated, .. } => *negated,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryAst {
    pub grammar_version: u32,
    pub terms: Vec<QueryTerm>,
    pub ignored: Vec<IgnoredTerm>,
}
