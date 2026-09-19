//! §7.3a owns `fade`, and §33.2 makes the pin permanent.
//!
//! **`fade = (is_reference || is_archived) ? 0.25 : 0`. For ever.** Two flat cases and a zero,
//! and **never a function of elapsed time** — not at paint time, not anywhere.
//!
//! **This was a phase gate and is now an invariant** (§27.6, `AC-P3-33-12`). §7.3a scheduled a
//! phase-3 input driven by the commit clock; §33.2 **deletes that row rather than amending it**.
//! Phase 3 renders material decay in the **DOM**, from the open debt list, over a bitmap decay
//! never enters — so the last route by which a clock could re-address a card is closed rather
//! than scheduled. Neither reason below weakens with the phase number, and either is sufficient:
//!
//! 1. A time-driven `fade` is staleness-driven decay wearing a palette. §33.2 lights the five
//!    material layers from **items**, which a user can close; a clock only ever accuses, and it
//!    would accuse through a channel that carries no item list to answer it.
//! 2. `fade` is a serialised scene field and `scene_hash` is content-addressed over the scene
//!    document, so a clock-driven `fade` makes the address a function of wall-clock time: every
//!    card re-hashes and re-rasterizes as the days advance, the disk cache grows without bound,
//!    and the recognition §7.4 spends a section defending is lost to arithmetic.
//!
//! **The gate is this signature.** `fade_for` takes no timestamp and no `Clock`, and nothing else
//! in this file carries a decay switch — which is why the invariant is one line rather than a
//! fork in the renderer, and why the two producers §7.1a requires to agree cannot drift apart on
//! it. A2b.

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
