//! §7.3a owns `fade` and its phase gate.
//!
//! **Phase 1:** `fade = (is_reference || is_archived) ? 0.25 : 0`. Two flat cases and a zero,
//! and **never a function of elapsed time** — not at paint time, not anywhere.
//!
//! **Phase 3:** the *input* becomes `clamp((days_since_last_commit − 60) / 900, 0, 1)`, arriving
//! with `condition_material` and the five material layers §0 puts Out. Every `fade` term in the
//! derivation stands verbatim through that change: phase 3 changes an input, not a derivation,
//! which is why the pin is one line and not a fork in the renderer, and why the two producers
//! §7.1a requires to agree cannot drift apart on it.
//!
//! **The gate is this signature.** `fade_for` takes no timestamp and no `Clock`. Nothing else in
//! this file carries a decay switch, so a phase-3 author changes this function and nothing below
//! it. Two independent reasons make that necessary, either one sufficient:
//!
//! 1. Decay rendering is out of phase 1 — §0 names the material layers and this clock as Out,
//!    §1.2 stores `condition_material` unrendered, and §11.3a cuts the decay group from
//!    settings. A time-driven `fade` ships the cut feature through the palette.
//! 2. `fade` is a serialised scene field and `scene_hash` is content-addressed over the scene
//!    document, so a clock-driven `fade` makes the address a function of wall-clock time.

/// The one non-zero value phase 1 emits.
pub const FADE_FLAT: f64 = 0.25;
/// The value every other project takes.
pub const FADE_NONE: f64 = 0.0;

#[must_use]
pub fn fade_for(is_reference: bool, is_archived: bool) -> f64 {
    if is_reference || is_archived {
        FADE_FLAT
    } else {
        FADE_NONE
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
mod tests {
    use super::*;

    #[test]
    fn phase_one_fade_takes_exactly_two_values() {
        // §7.3a: fade = (is_reference || is_archived) ? 0.25 : 0
        assert!((fade_for(false, false) - 0.0).abs() < f64::EPSILON);
        assert!((fade_for(true, false) - 0.25).abs() < f64::EPSILON);
        assert!((fade_for(false, true) - 0.25).abs() < f64::EPSILON);
        assert!((fade_for(true, true) - 0.25).abs() < f64::EPSILON);
    }

    #[test]
    fn no_third_value_is_reachable() {
        // Criterion 62: "no other value appears in phase 1".
        let mut seen = std::collections::BTreeSet::new();
        for reference in [false, true] {
            for archived in [false, true] {
                seen.insert(format!("{:.3}", fade_for(reference, archived)));
            }
        }
        assert_eq!(seen.len(), 2);
        assert!(seen.contains("0.000"));
        assert!(seen.contains("0.250"));
    }

    #[test]
    fn the_full_fade_state_the_ring_contrast_work_reasons_about_is_unreachable() {
        // §7.8: ring-contrast fixtures "never construct fade = 1". Nothing here can.
        for reference in [false, true] {
            for archived in [false, true] {
                assert!(fade_for(reference, archived) <= FADE_FLAT);
            }
        }
    }
}
