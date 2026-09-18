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
