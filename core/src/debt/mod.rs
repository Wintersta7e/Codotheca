//! §28 — the debt item: **one per-project list of concrete, derived, closable items.**
//!
//! An item has exactly two stored states, `open` and `unverified`. ***Closed* is an event, never
//! a state**: the row is deleted and the `xp_events` ledger is the permanent record. There is no
//! third stored state and **no user-facing dismissal** — completion has one, debt does not.
//!
//! Identity is `(subject_key, source, fingerprint)` ([`identity::DebtKey`]) and everything else
//! on the row is an attribute, so a rename, a line move and a re-clone close nothing.
//!
//! **§28 adds no command, no event and no topic**, so this module has no dispatcher.

pub mod identity;
pub mod store;
pub mod sweep;
pub mod xp;

use crate::protocol::{DebtScoring, DebtSource, DecayLayer, ObservationBasis};

/// Render a **generated** enum as the TEXT its column stores, through serde.
///
/// There is deliberately no second vocabulary here: R31's failure mode is a hand-written table
/// beside the schema's, and R26's is a stored slug that has drifted from the emitted one. Going
/// through serde means the only vocabulary in this module is `protocol/schema/protocol.json`'s,
/// and the DDL CHECKs in `0013_debt.sql` mirror it character for character.
///
/// `core/src/accounts/store.rs:457-464` is the precedent this follows.
pub(crate) fn enum_text<T: serde::Serialize>(value: &T) -> Result<String, DebtError> {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(raw)) => Ok(raw),
        other => Err(DebtError::Codec(format!(
            "a generated enum did not serialise as a string: {other:?}"
        ))),
    }
}

/// Read a stored TEXT enum back through the same renames.
///
/// `None` is a word this build does not know — a row from a newer schema — which the caller
/// turns into an error rather than guessing at.
pub(crate) fn enum_from_text<T: serde::de::DeserializeOwned>(raw: &str) -> Option<T> {
    serde_json::from_value(serde_json::Value::String(raw.to_owned())).ok()
}

/// Why a debt operation could not complete.
///
/// It wraps [`IndexError`] rather than restating it: every write in this module goes through a
/// caller's transaction and the database is the only thing that can refuse one.
///
/// [`IndexError`]: crate::index::IndexError
#[derive(Debug, thiserror::Error)]
pub enum DebtError {
    /// The database refused.
    #[error("index: {0}")]
    Index(#[from] crate::index::IndexError),
    /// A stored value is not one this build's schema declares — a row written by a newer build,
    /// never a value to guess at.
    #[error("debt: {0}")]
    Codec(String),
}

impl From<rusqlite::Error> for DebtError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Index(crate::index::IndexError::from(error))
    }
}

/// How an item of this source is keyed.
///
/// **Not a wire type** (§28.9): the shape is a property of `DebtSource`, so putting it on the
/// wire would be one value in two places.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentityShape {
    /// Fingerprint `''` — one item per source per subject.
    Singleton,
    /// `<salient_sha256>:<ordinal>`.
    Content,
    /// `<ecosystem>:<package>:<advisory_id>`.
    External,
}

/// §28.2's registry row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceRow {
    pub source: DebtSource,
    pub shape: IdentityShape,
    /// §33 owns every rule over the layer; this is the join, and the only place a source's layer
    /// is stated. It is deliberately **not** a `debt_item` column — a stored copy could disagree
    /// with the source it describes.
    pub layer: DecayLayer,
    /// The source's default. **Overridable per item by the producer**: §32 sets an advisory
    /// `scored` when a fix is available and `shown_only` when one is not.
    pub default_scoring: DebtScoring,
    /// `None` has exactly one meaning and exactly one source: `abandoned_with_debt` is derived
    /// from other stored observations and observes nothing itself, so it has no basis to carry
    /// and inventing one would be a lie.
    pub basis: Option<ObservationBasis>,
}

const TODO_MARKER: SourceRow = SourceRow {
    source: DebtSource::TodoMarker,
    shape: IdentityShape::Content,
    layer: DecayLayer::Overgrowth,
    default_scoring: DebtScoring::Scored,
    basis: Some(ObservationBasis::Head),
};
const MISSING_README: SourceRow = SourceRow {
    source: DebtSource::MissingReadme,
    shape: IdentityShape::Singleton,
    layer: DecayLayer::Dust,
    default_scoring: DebtScoring::Scored,
    basis: Some(ObservationBasis::Head),
};
const MISSING_LICENSE: SourceRow = SourceRow {
    source: DebtSource::MissingLicense,
    shape: IdentityShape::Singleton,
    layer: DecayLayer::Dust,
    default_scoring: DebtScoring::Scored,
    basis: Some(ObservationBasis::Head),
};
const MISSING_TESTS: SourceRow = SourceRow {
    source: DebtSource::MissingTests,
    shape: IdentityShape::Singleton,
    layer: DecayLayer::Overgrowth,
    default_scoring: DebtScoring::Scored,
    basis: Some(ObservationBasis::Head),
};
const NO_RELEASE: SourceRow = SourceRow {
    source: DebtSource::NoRelease,
    shape: IdentityShape::Singleton,
    layer: DecayLayer::Dust,
    default_scoring: DebtScoring::Scored,
    basis: Some(ObservationBasis::Refs),
};
const UNPUSHED_COMMITS: SourceRow = SourceRow {
    source: DebtSource::UnpushedCommits,
    shape: IdentityShape::Singleton,
    layer: DecayLayer::Overgrowth,
    default_scoring: DebtScoring::Scored,
    basis: Some(ObservationBasis::Refs),
};
const CI_RED: SourceRow = SourceRow {
    source: DebtSource::CiRed,
    shape: IdentityShape::Singleton,
    layer: DecayLayer::Cracks,
    default_scoring: DebtScoring::Scored,
    basis: Some(ObservationBasis::Remote),
};
const DEPENDENCY_ADVISORY: SourceRow = SourceRow {
    source: DebtSource::DependencyAdvisory,
    shape: IdentityShape::External,
    layer: DecayLayer::Rust,
    // Per item, set by §32: `scored` when a fix is available, `shown_only` when one is not. The
    // default is what a producer that does not know starts from.
    default_scoring: DebtScoring::Scored,
    basis: Some(ObservationBasis::Worktree),
};
const ABANDONED_WITH_DEBT: SourceRow = SourceRow {
    source: DebtSource::AbandonedWithDebt,
    shape: IdentityShape::Singleton,
    layer: DecayLayer::Cobwebs,
    // Its item closes when the project stops being abandoned, and paying XP for that would be
    // paying for **activity**. The layer still lights, because lighting is what `shown_only`
    // keeps.
    default_scoring: DebtScoring::ShownOnly,
    basis: None,
};

/// **One table with one row per source**, so the layer, the default scoring, the identity shape
/// and the basis are read from one place and cannot drift apart.
pub const SOURCE_REGISTRY: [SourceRow; 9] = [
    TODO_MARKER,
    MISSING_README,
    MISSING_LICENSE,
    MISSING_TESTS,
    NO_RELEASE,
    UNPUSHED_COMMITS,
    CI_RED,
    DEPENDENCY_ADVISORY,
    ABANDONED_WITH_DEBT,
];

/// Total over the closed enum, with **no `_ =>` arm**: a tenth variant added to the schema fails
/// to compile here rather than falling through to a wrong row at run time.
#[must_use]
pub fn registry_for(source: DebtSource) -> &'static SourceRow {
    match source {
        DebtSource::TodoMarker => &TODO_MARKER,
        DebtSource::MissingReadme => &MISSING_README,
        DebtSource::MissingLicense => &MISSING_LICENSE,
        DebtSource::MissingTests => &MISSING_TESTS,
        DebtSource::NoRelease => &NO_RELEASE,
        DebtSource::UnpushedCommits => &UNPUSHED_COMMITS,
        DebtSource::CiRed => &CI_RED,
        DebtSource::DependencyAdvisory => &DEPENDENCY_ADVISORY,
        DebtSource::AbandonedWithDebt => &ABANDONED_WITH_DEBT,
    }
}
