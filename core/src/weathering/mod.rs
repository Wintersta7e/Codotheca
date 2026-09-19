//! §33's anchor resolution, beside the debt list rather than inside `core/src/art/`.
//!
//! **Boundary 2 (§27.4): decay never enters the bitmap.** `scene_hash` takes no debt input, and
//! `core/src/art/` gains no decay knowledge at all — this module reads the art module's `Scene`
//! and the art module names no symbol of this one. `core/tests/acceptance_weathering.rs` audits
//! both halves: a byte-identity criterion over two rendition files, and a comment-stripped
//! identifier scan over `core/src/art/`.
//!
//! What it does **not** read: any debt table, `health_delta`, `location`, or a clock. The reply
//! is a derivation over a stored document, so it carries no `computedAt` — the reading's age is
//! §30's `HealthState.observedAt` and is restated nowhere.

pub mod anchors;
