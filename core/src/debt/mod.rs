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
