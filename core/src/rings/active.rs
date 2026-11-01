//! §39.6: the active day — a boolean union of three day sets. Pure: it names no table, so no
//! minute ever becomes a commit-day and no commit-day a minute.

use std::collections::BTreeSet;

/// An inclusive range of local day numbers, as `clock::local_day` produces them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DaySpan {
    /// The span's first day.
    pub first: i64,
    /// The span's last day, inclusive.
    pub last: i64,
}

impl DaySpan {
    /// Every day of the span, first to last.
    #[must_use]
    pub const fn days(self) -> std::ops::RangeInclusive<i64> {
        self.first..=self.last
    }
}

/// §39.6: the dates on which any witness holds — the union of the git (`g`), focus (`s`) and
/// tend (`t`) day sets, and nothing else. A day witnessed twice counts once; no witness is
/// weighted.
#[must_use]
pub fn union_days(g: &BTreeSet<i64>, s: &BTreeSet<i64>, t: &BTreeSet<i64>) -> BTreeSet<i64> {
    g.iter().chain(s).chain(t).copied().collect()
}
