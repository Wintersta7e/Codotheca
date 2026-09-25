//! §28.1's salient normalisation — **the cap is part of the identity**.
//!
//! This file is §28's and holds exactly the two things §29's producer needs from it (R134). It
//! lives here rather than in J7's module because a reader asking *why do two items share a
//! fingerprint* opens this file: in a job module the cap reads as a scanning detail, and the next
//! author who changes it for scanning reasons silently changes every item's identity — closing
//! items that were never fixed and opening items that never changed.
//!
//! §29.3 owns the other half and does not restate this one: **what J7 extracts** is the bytes
//! from the marker's first byte to the end of its line. What is then done to them is here.
//!
//! **p3-28 adds to this file in wave 2 and creates no second.**
//!
//! **Anyone editing [`SALIENT_CAP_BYTES`] or [`normalise_salient`] is changing every item's
//! identity**, closing items that were never fixed and opening items that never changed. A change
//! to either invalidates every stored `salient_sha256`, so it bumps §29's `J7_SCANNER_VERSION`
//! in the same edit. `core/tests/debt_identity.rs` holds a gate asserting exactly one
//! implementation of each, over the whole of `core/src/`.

use crate::protocol::DebtSource;

/// §28.1's cap, applied **before the hash and not after**, at the largest UTF-8 character
/// boundary at or below it — a byte index that splits a code point panics in Rust.
///
/// Load-bearing rather than cosmetic: a minified or generated file with a programming extension
/// is a single line of arbitrary length.
pub const SALIENT_CAP_BYTES: usize = 200;

/// Trim, collapse internal whitespace runs to one space, then cap.
///
/// Takes **raw bytes**, not a marker and a body: §29.3 rules that *J7 does not parse*, so the
/// producer performs no split and `&[u8]` is what it actually holds. A formatter reflowing a
/// wrapped comment must not change an item's identity; a formatter *rewording* it is a different
/// item and the app is allowed to think so.
#[must_use]
pub fn normalise_salient(raw: &[u8]) -> String {
    let text = String::from_utf8_lossy(raw);
    let mut collapsed = String::with_capacity(text.len());
    for word in text.split_whitespace() {
        if !collapsed.is_empty() {
            collapsed.push(' ');
        }
        collapsed.push_str(word);
    }
    if collapsed.len() <= SALIENT_CAP_BYTES {
        return collapsed;
    }
    let mut cut = SALIENT_CAP_BYTES;
    while cut > 0 && !collapsed.is_char_boundary(cut) {
        cut -= 1;
    }
    collapsed.truncate(cut);
    collapsed
}

/// An item's identity: `(subject_key, source, fingerprint)` and nothing else.
///
/// **`subject_key` is [`ProjectSubject::to_key`] and never `project_id`.** §1.7 records that v1
/// keyed the ledger on `project_id` and it broke on merges; `subject_key` is *total* where
/// `lineage_key` is not. `project_id` is an attribute, repointed by the merge recompute, and is
/// never key material.
///
/// **The path, the line and the column are attributes too.** A rename closes nothing, a line move
/// closes nothing, and two identical marker texts are two items.
///
/// [`ProjectSubject::to_key`]: crate::index::subject::ProjectSubject::to_key
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DebtKey {
    /// The logical subject, shared with the §1.12 sidecar and with `xp_events.subject_key`.
    pub subject_key: String,
    /// The producer that found the item; its registry row fixes the fingerprint's shape.
    pub source: DebtSource,
    /// `''` for a singleton — never NULL, because SQLite treats NULLs as distinct inside a UNIQUE
    /// index and a nullable fingerprint would silently permit duplicate singletons.
    pub fingerprint: String,
}

impl DebtKey {
    /// §28.1's singleton shape: one item per source per subject, fingerprint `''`.
    #[must_use]
    pub fn singleton(subject_key: &str, source: DebtSource) -> Self {
        Self {
            subject_key: subject_key.to_owned(),
            source,
            fingerprint: String::new(),
        }
    }

    /// §28.1's content shape: `<salient_sha256>:<ordinal>`.
    ///
    /// `ordinal` is the **per-project** ordinal (A10), 0-based among the project's occurrences
    /// sharing a `salient_sha256` — never `blob_finding.ordinal_in_blob`, which is a position
    /// inside a content-addressed blob shared library-wide. One blob reachable at two paths
    /// contributes its occurrences twice, and conflating the two collapses them onto one item.
    #[must_use]
    pub fn content(subject_key: &str, salient_sha256: &str, ordinal: i64) -> Self {
        Self {
            subject_key: subject_key.to_owned(),
            source: DebtSource::TodoMarker,
            fingerprint: format!("{salient_sha256}:{ordinal}"),
        }
    }

    /// §28.1's external shape: `<ecosystem>:<package>:<advisory_id>` (**A5**).
    ///
    /// **The version is excluded and the advisory id is the GHSA id, never the CVE id.** A bump
    /// from one vulnerable version to another must not close the item and open a new one — the
    /// user has not finished, and paying them twice for one advisory is rewarding volume.
    #[must_use]
    pub fn external(subject_key: &str, ecosystem: &str, package: &str, advisory_id: &str) -> Self {
        Self {
            subject_key: subject_key.to_owned(),
            source: DebtSource::DependencyAdvisory,
            fingerprint: format!("{ecosystem}:{package}:{advisory_id}"),
        }
    }
}
