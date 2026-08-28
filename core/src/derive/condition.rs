//! §5.4 — the condition ladder.
//!
//! **This vocabulary is normative and the design's is not**: `warm`, `cooling` and `blueprint`
//! are stored nowhere, queried nowhere and shipped nowhere, and criterion 58 bans the strings.

/// One day, in seconds. Named because every ladder edge is stated in days.
pub const DAY: i64 = 86_400;

/// §5.4's bands.
///
/// **R31: declared in `protocol/schema/protocol.json` and generated into `crate::protocol`.**
/// Re-exported so this module's path names it. The value is stored in
/// `project.condition_signal`, whose CHECK lists the same seven words; a hand-written copy would
/// compile and then drift from the column and the wire at once.
pub use crate::protocol::ConditionSignal;

impl ConditionSignal {
    /// Every band, so a test can walk the vocabulary without restating it.
    pub const ALL: [ConditionSignal; 7] = [
        ConditionSignal::Live,
        ConditionSignal::Idle,
        ConditionSignal::Dormant,
        ConditionSignal::Neglected,
        ConditionSignal::Abandoned,
        ConditionSignal::Offline,
        ConditionSignal::Empty,
    ];

    /// The stored form, matching `condition_signal`'s CHECK character for character.
    #[must_use]
    pub const fn slug(self) -> &'static str {
        match self {
            ConditionSignal::Live => "live",
            ConditionSignal::Idle => "idle",
            ConditionSignal::Dormant => "dormant",
            ConditionSignal::Neglected => "neglected",
            ConditionSignal::Abandoned => "abandoned",
            ConditionSignal::Offline => "offline",
            ConditionSignal::Empty => "empty",
        }
    }

    /// `slug`'s inverse. `None` for anything the CHECK would reject — including the design's
    /// three band words, which is criterion 58 held mechanically.
    #[must_use]
    pub fn from_slug(s: &str) -> Option<ConditionSignal> {
        ConditionSignal::ALL.into_iter().find(|c| c.slug() == s)
    }
}

/// What the ladder is computed from.
#[derive(Debug, Clone, Copy)]
pub struct Inputs {
    /// `last_interaction_at` for the signal band; `last_commit_at` for the material band.
    pub clock_at: Option<i64>,
    /// Now, epoch seconds.
    pub now: i64,
    /// `None` = J4 has not run. `Some(false)` is a measured zero-commit repository.
    pub has_commits: Option<bool>,
    /// Every copy is on a store that is not mounted.
    pub all_locations_offline: bool,
    /// §1.2: the band is NULL until a scan job has produced one.
    pub any_job_succeeded: bool,
}

/// Compute one band, or `None` for "not computed".
///
/// Overrides are applied before the ladder, and `empty` outranks `offline`: a zero-commit
/// repository was never alive, so there is no band to freeze. `is_reference` and `is_archived`
/// are **not** band values (§5.4a) and are applied at render time.
#[must_use]
pub fn signal(inputs: &Inputs) -> Option<ConditionSignal> {
    if !inputs.any_job_succeeded {
        return None;
    }
    if inputs.has_commits == Some(false) {
        return Some(ConditionSignal::Empty);
    }
    if inputs.all_locations_offline {
        return Some(ConditionSignal::Offline);
    }
    let at = inputs.clock_at?;
    let days = (inputs.now - at).max(0) / DAY;
    Some(match days {
        0..=7 => ConditionSignal::Live,
        8..=30 => ConditionSignal::Idle,
        31..=90 => ConditionSignal::Dormant,
        91..=365 => ConditionSignal::Neglected,
        _ => ConditionSignal::Abandoned,
    })
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

    const NOW: i64 = 1_800_000_000;

    fn at(days_ago: i64) -> Inputs {
        Inputs {
            clock_at: Some(NOW - days_ago * DAY),
            now: NOW,
            has_commits: Some(true),
            all_locations_offline: false,
            any_job_succeeded: true,
        }
    }

    #[test]
    fn the_ladder_edges_are_seven_thirty_ninety_and_three_sixty_five() {
        assert_eq!(signal(&at(0)), Some(ConditionSignal::Live));
        assert_eq!(signal(&at(7)), Some(ConditionSignal::Live));
        assert_eq!(signal(&at(8)), Some(ConditionSignal::Idle));
        assert_eq!(signal(&at(30)), Some(ConditionSignal::Idle));
        assert_eq!(signal(&at(31)), Some(ConditionSignal::Dormant));
        assert_eq!(signal(&at(90)), Some(ConditionSignal::Dormant));
        assert_eq!(signal(&at(91)), Some(ConditionSignal::Neglected));
        assert_eq!(signal(&at(365)), Some(ConditionSignal::Neglected));
        assert_eq!(signal(&at(366)), Some(ConditionSignal::Abandoned));
    }

    #[test]
    fn the_designs_hundred_and_twenty_day_edge_is_not_one_of_ours() {
        // Criterion 58: the design's `cooling` boundary at 120 days appears in no edge here.
        assert_eq!(signal(&at(119)), signal(&at(121)));
    }

    #[test]
    fn a_never_indexed_project_has_no_band_at_all() {
        // §1.2 / §5.4a: NULL until a scan job has produced one. Not `empty`, not `offline`.
        let mut i = at(3);
        i.any_job_succeeded = false;
        assert_eq!(signal(&i), None);

        let mut j = at(3);
        j.clock_at = None;
        assert_eq!(signal(&j), None);
    }

    #[test]
    fn empty_outranks_offline_and_never_inherits_an_interaction_band() {
        // §5.4a: no commit, no band to freeze. A zero-commit repository was never alive.
        let mut i = at(3);
        i.has_commits = Some(false);
        i.all_locations_offline = true;
        assert_eq!(signal(&i), Some(ConditionSignal::Empty));
    }

    #[test]
    fn offline_freezes_the_band() {
        let mut i = at(3);
        i.all_locations_offline = true;
        assert_eq!(signal(&i), Some(ConditionSignal::Offline));
    }

    #[test]
    fn unknown_commit_state_is_not_empty() {
        // has_commits: None means J4 has not run. Rendering that as `empty` asserts a
        // measured zero-commit repository, which is a different fact.
        let mut i = at(3);
        i.has_commits = None;
        assert_eq!(signal(&i), Some(ConditionSignal::Live));
    }

    #[test]
    fn the_designs_band_words_are_not_in_this_enum() {
        // Criterion 58's string ban: `warm`, `cooling` and `blueprint` appear nowhere.
        for s in ["warm", "cooling", "blueprint"] {
            assert_eq!(ConditionSignal::from_slug(s), None);
        }
        assert_eq!(
            ConditionSignal::from_slug("dormant"),
            Some(ConditionSignal::Dormant)
        );
    }

    /// The slug is stored in `project.condition_signal` and carried on the wire.
    /// `core/tests/derive_persist.rs` inserts every one of them against the real column.
    #[test]
    fn every_band_round_trips_and_matches_its_wire_form() {
        for band in ConditionSignal::ALL {
            assert_eq!(ConditionSignal::from_slug(band.slug()), Some(band));
            assert_eq!(
                serde_json::to_string(&band).unwrap(),
                format!("\"{}\"", band.slug()),
                "the stored slug and the wire form are one value"
            );
        }
    }

    /// A clock in the future is not a negative age. Clock skew across a network share is
    /// ordinary, and a negative division would land in `Abandoned`.
    #[test]
    fn a_clock_from_the_future_is_live_not_abandoned() {
        let mut i = at(0);
        i.clock_at = Some(NOW + 100 * DAY);
        assert_eq!(signal(&i), Some(ConditionSignal::Live));
    }
}
