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
}
