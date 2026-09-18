//! §32 — dependency health: **what this library installs, and what the forge says about it.**
//!
//! One process-wide, unauthenticated, scheduled sync task reads the forge's global advisories
//! endpoint; a bounded worktree read finds each project's lockfiles; and the verdict is the join
//! of the two, derived at read time and stored nowhere.
//!
//! **Three things this module does not own.** The debt item's DDL, its states and its writer are
//! §28's — this module produces `dependency_advisory` items *through* that writer and declares no
//! second one. The `deps` completion check is §31's, and is an aggregate consumer of the item set
//! rather than one item keyed `deps`. The health arithmetic an `unknown` verdict is excluded from
//! is §30's; what this module owns is the input state.

pub mod lockfiles;
pub mod parse;
pub mod store;

use crate::protocol::{DependencyReadState, Ecosystem};

/// Why an advisory operation could not complete.
///
/// It wraps [`IndexError`] rather than restating it: every write in this module goes through a
/// caller's transaction, and the database is the only thing that can refuse one. `Parse` carries
/// the reader's own words for a file it could not make sense of — which is a **`not_read` row**,
/// not an error, in every path but the one where the row itself cannot be written.
///
/// [`IndexError`]: crate::index::IndexError
#[derive(Debug, thiserror::Error)]
pub enum AdvisoryError {
    /// The database refused.
    #[error("index: {0}")]
    Index(#[from] crate::index::IndexError),
    /// A file could not be read from disk at all.
    #[error("advisory read: {0}")]
    Parse(String),
}

/// Render a **generated** enum as the TEXT its column stores, through serde.
///
/// There is deliberately no second vocabulary here: R31's failure mode is a hand-written table
/// beside the schema's, and R26's is a stored slug that has drifted from the emitted one. Going
/// through serde means the only vocabulary in this module is `protocol/schema/protocol.json`'s,
/// and `0014_advisories.sql`'s CHECKs mirror it character for character.
pub(crate) fn enum_text<T: serde::Serialize>(value: &T) -> Result<String, AdvisoryError> {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(raw)) => Ok(raw),
        other => Err(AdvisoryError::Parse(format!(
            "a generated enum did not serialise as a string: {other:?}"
        ))),
    }
}

/// How many `(name, version)` pairs one request's `affects` list may carry.
///
/// **Measured, not documented.** `stack.md` recorded *"up to 1000 packages per request"* from the
/// endpoint's own parameter documentation; against the live endpoint on **2026-09-19** a request
/// is refused with **HTTP 414** long before that, because the binding limit is the **URL length**
/// and not the pair count. See [`ADVISORY_AFFECTS_BYTE_CAP`], which is the bound that actually
/// fires; this one is the second bound, so a sweep over very short package names still issues a
/// bounded number of pairs per request rather than an unbounded one.
///
/// 256 sits well under the **440 seven-character names that were accepted** in the same
/// measurement.
pub const ADVISORY_BATCH_CAP: usize = 256;

/// How many **percent-encoded bytes** of `affects` one request may carry.
///
/// **The measurement, with the date and the tree it was taken on** (2026-09-19, this lane's base):
/// 7,917 encoded bytes were accepted with HTTP 200; 8,277 were refused with **HTTP 414 `We
/// received a Request-URL that is too long from your client`**. The ceiling is the classic 8 KiB
/// request line, and it is a **byte** ceiling — so a cap expressed only as a pair count would be
/// right for `pkg0001` and wrong the first time a library resolves a scoped package name.
///
/// 6,144 leaves roughly 22% headroom under the lower measured pass, and room for the rest of the
/// query string beside `affects`.
pub const ADVISORY_AFFECTS_BYTE_CAP: usize = 6 * 1024;

/// An inherent const on a generated enum: legal because both are in this crate.
///
/// **The set is a property of this app's parser coverage**, not of the source's vocabulary — an
/// ecosystem is listed once a lockfile of its shape can be read. Every slug is character-identical
/// to the endpoint's own `ecosystem` parameter because it is sent as one.
impl Ecosystem {
    pub const ALL: [Ecosystem; 3] = [Ecosystem::Npm, Ecosystem::Rust, Ecosystem::Pip];
}

/// Two variants and deliberately not three: *the scan has not run* is the **absence** of a
/// `project_dependency_scan` row, because a file that was not read produces no `(package,
/// version)` key for a third variant to sit on.
impl DependencyReadState {
    pub const ALL: [DependencyReadState; 2] =
        [DependencyReadState::Parsed, DependencyReadState::NotRead];
}
