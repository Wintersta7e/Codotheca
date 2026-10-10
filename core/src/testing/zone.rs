//! Zones a test chooses (§39.2 rule 1): one offset for ever, or one switch at a chosen instant.

use crate::clock::LocalZone;

/// A zone whose offset never changes: minutes east of UTC.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FixedZone(pub i32);

impl LocalZone for FixedZone {
    fn offset_min_at(&self, _unix: i64) -> i32 {
        self.0
    }
}

/// A zone with one transition: `before_min` before `at`, `after_min` from `at` on — a DST switch
/// at an instant the test picks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransitionZone {
    /// The first instant (epoch seconds) in the new offset.
    pub at: i64,
    /// Minutes east of UTC before the switch.
    pub before_min: i32,
    /// Minutes east of UTC from the switch on.
    pub after_min: i32,
}

impl LocalZone for TransitionZone {
    fn offset_min_at(&self, unix: i64) -> i32 {
        if unix < self.at {
            self.before_min
        } else {
            self.after_min
        }
    }
}
