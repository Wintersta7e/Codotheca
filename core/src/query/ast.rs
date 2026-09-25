//! The serde types that serialise byte-identically to `app/src/shared/query/ast.ts`.
//!
//! **R13: this plan owns both language mirrors**, and R24 makes the pair legitimate on one
//! condition — a test reads the other side. `core/tests/query_corpus.rs` is that test: it runs
//! `protocol/query/corpus.json`, the same file the TypeScript suite runs.

use serde::{Deserialize, Serialize};

/// The direction of a comparison value — `size:`, `touched:` and `completion:` take one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Cmp {
    /// `<`: the row's value is strictly below the bound.
    Lt,
    /// `>`: the row's value is strictly above the bound.
    Gt,
}

/// The four fields whose value is matched as text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TextField {
    /// `lang:` — the project's primary language, case-insensitively.
    Lang,
    /// `owner:` — the project's owner, case-insensitively.
    Owner,
    /// `in:` — where the primary working copy lives: `local`, `wsl`, `wsl:<distro>`, or a path
    /// prefix.
    In,
    /// `collection:` — membership in the user's collection of that name.
    Collection,
}

/// The flags an `is:` term can ask about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum IsFlag {
    /// `is:dirty` — the working copy had uncommitted changes when last observed.
    Dirty,
    /// `is:unpushed` — the branch is ahead of its upstream: work only on this disk.
    Unpushed,
    /// `is:behind` — the upstream holds commits the branch does not.
    Behind,
    /// `is:interrupted` — a merge, rebase, cherry-pick, revert, bisect or `am` was left in
    /// progress.
    Interrupted,
    /// `is:archived` — the user archived the project.
    Archived,
    /// `is:pinned` — the user pinned the project.
    Pinned,
    /// `is:hidden` — the user hid the project; asking for it opts hidden rows back into the
    /// base set (§8.0b).
    Hidden,
    /// `is:reference` — no commit in the project's history is the user's (§5.5); asking for it
    /// opts reference rows back into the base set (§8.0b).
    Reference,
    /// `is:bare` — the repository has no working tree.
    Bare,
    /// `is:fork` — the project is a fork of another repository.
    Fork,
    /// `is:empty` — the repository was measured and holds no commits.
    Empty,
    /// `is:shallow` — the clone's history is truncated.
    Shallow,
    /// `is:local` — the primary working copy is on this machine's own filesystem, not in WSL.
    Local,
    /// `is:wsl` — the primary working copy lives inside a WSL distribution.
    Wsl,
    /// `is:new` — created after first run completed and not yet acknowledged; unknown before
    /// first run completes (§10.5a).
    New,
    /// §23.6, **one word**: `rename_all = "camelCase"` emits `"notcloned"`, the same string
    /// `IS_FLAGS` carries and the corpus holds. `NotCloned` would emit `"notCloned"` and put the
    /// two mirrors' ASTs one capital apart in a file both are tested against. This is the query
    /// AST's convention and **not** the generated protocol enum's — R47's hyphenated renditions
    /// are a different mechanism and the two must not be conflated.
    Notcloned,
}

/// The attributes a `has:` term can ask about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HasAttribute {
    /// `has:readme` — a README at `HEAD`, as J7 read it (§29.4).
    Readme,
    /// `has:license` — a license file at `HEAD`, as J7 read it.
    License,
    /// `has:tests` — tests at `HEAD`, as J7 read them.
    Tests,
    /// `has:ci` — CI configuration at `HEAD`, as J7 read it.
    Ci,
    /// `has:remote` — the project carries a forge remote key.
    Remote,
    /// `has:stash` — the repository holds at least one stash entry.
    Stash,
    /// `has:submodules` — the project is the parent of at least one indexed submodule.
    Submodules,
}

/// The three the *parser* can produce. `notAvailable` is renderer-only — only a projection can
/// lack a field the index holds — so it never crosses the wire and has no variant here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum IgnoredReason {
    /// The prefix reads as a field name, and the grammar has no field by that name.
    UnknownField,
    /// Nothing in the index can answer the term.
    NotComputed,
    /// The field is known and its value does not parse.
    MalformedValue,
}

/// A term the query ran without — a soft error the pill renders while the rest still runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IgnoredTerm {
    /// The term as typed, its `-` included.
    pub text: String,
    /// Why the term was left out.
    pub reason: IgnoredReason,
}

/// One term of a query. Terms are AND: a row matches when every term does.
///
/// Every variant's `negated` is the term's leading `-`. A negated term matches a row whose answer
/// is known false; an unknown answer matches neither polarity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum QueryTerm {
    /// A word with no field, fuzzy-matched against name, owner, description, path and commit
    /// subjects.
    Bare {
        /// The term's leading `-`.
        negated: bool,
        /// The word, unquoted and lower-cased.
        text: String,
    },
    /// `lang:`, `owner:`, `in:` or `collection:` with a text value.
    Text {
        /// The term's leading `-`.
        negated: bool,
        /// Which of the four text fields.
        field: TextField,
        /// The value, lower-cased unless it was quoted.
        value: String,
        /// Whether the value was quoted, which keeps an `in:` path prefix case-sensitive where
        /// paths are.
        quoted: bool,
    },
    /// `is:<flag>`.
    Flag {
        /// The term's leading `-`.
        negated: bool,
        /// The flag asked about.
        flag: IsFlag,
    },
    /// `has:<attribute>`.
    Has {
        /// The term's leading `-`.
        negated: bool,
        /// The attribute asked about.
        attribute: HasAttribute,
    },
    /// `size:>10mb` — tracked bytes against a bound.
    Size {
        /// The term's leading `-`.
        negated: bool,
        /// Above or below the bound.
        op: Cmp,
        /// The bound in bytes, the unit already applied (`kb` is 1,024 bytes).
        bytes: u64,
    },
    /// `touched:>6mo` — time since the project was last touched, against a bound.
    TouchedAge {
        /// The term's leading `-`.
        negated: bool,
        /// Older or younger than the bound.
        op: Cmp,
        /// The bound in days: a `w` is 7, a `mo` 30 and a `y` 365.
        days: u32,
    },
    /// `touched:2019` — the local calendar year the project was last touched in.
    TouchedYear {
        /// The term's leading `-`.
        negated: bool,
        /// The four-digit year.
        year: u32,
    },
    /// [p3] §31.1: `completion:>5` and `completion:<5`, over `project.completion_lit`.
    ///
    /// **The NULL half is the invariant**: a row whose projection is NULL matches **neither**
    /// comparison and is never coerced to `0`. That was the one half of the phase-1 soft-error
    /// behaviour worth keeping, and it hardens here rather than expiring with it.
    Completion {
        /// The term's leading `-`.
        negated: bool,
        /// Above or below the bound.
        op: Cmp,
        /// The bound, as a count of lit checks.
        value: u32,
    },
}

impl QueryTerm {
    /// Whether the term is negated, without matching every variant at every call site.
    #[must_use]
    pub const fn negated(&self) -> bool {
        match self {
            Self::Bare { negated, .. }
            | Self::Text { negated, .. }
            | Self::Flag { negated, .. }
            | Self::Has { negated, .. }
            | Self::Size { negated, .. }
            | Self::TouchedAge { negated, .. }
            | Self::TouchedYear { negated, .. }
            | Self::Completion { negated, .. } => *negated,
        }
    }
}

/// A parsed query, the shape both language mirrors serialise byte for byte.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryAst {
    /// The `QUERY_GRAMMAR_VERSION` the text was parsed under.
    pub grammar_version: u32,
    /// The terms that parsed, in the order they were typed.
    pub terms: Vec<QueryTerm>,
    /// The terms that did not, each with the reason.
    pub ignored: Vec<IgnoredTerm>,
}
