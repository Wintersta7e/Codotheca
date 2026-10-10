//! The `Clock` seam (§15.2). Nothing in the core reads the OS clock directly: era boundaries
//! (§8.1) and idle segmentation (§9) are only testable if time is injected.

use std::fmt;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// The only source of time in the core.
///
/// `now_unix` is wall time in **epoch seconds** and is what every `*_at` column stores — the
/// spec states no other unit and has no millisecond column, and every `*_at` value originates
/// from git, which reports seconds. `monotonic_ms` never decreases and is what budgets,
/// timeouts and session segmentation measure; it is never stored, because it has no meaning
/// across a process restart, and it is deliberately not an `Instant`, which cannot be faked.
pub trait Clock: Send + Sync + fmt::Debug {
    /// Seconds since the Unix epoch. May jump in either direction.
    fn now_unix(&self) -> i64;
    /// Milliseconds since an arbitrary origin. Never decreases.
    fn monotonic_ms(&self) -> u64;
    /// Block the calling thread for `dur`. Every backoff in the core delays through here, so a
    /// fake can return immediately instead of the suite waiting out a real budget.
    fn sleep(&self, dur: Duration);
}

/// The production clock.
#[derive(Debug)]
pub struct SystemClock {
    origin: Instant,
}

impl SystemClock {
    /// A clock whose `monotonic_ms` counts from this call.
    #[must_use]
    pub fn new() -> Self {
        Self {
            origin: Instant::now(),
        }
    }
}

impl Default for SystemClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for SystemClock {
    fn now_unix(&self) -> i64 {
        // A pre-epoch system clock is possible and must not panic; it reports as negative.
        match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(d) => i64::try_from(d.as_secs()).unwrap_or(i64::MAX),
            Err(e) => -i64::try_from(e.duration().as_secs()).unwrap_or(i64::MAX),
        }
    }

    fn monotonic_ms(&self) -> u64 {
        u64::try_from(self.origin.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    fn sleep(&self, dur: Duration) {
        std::thread::sleep(dur);
    }
}

/// The machine's zone as a **function of the instant** (§39.2 rule 1): the offset in force at
/// `t`, read per instant and never cached.
///
/// Not a `Clock` method: the clock is time a test injects, and the zone is a property of the
/// machine. The composition root builds one and passes it down, so nothing below the root reads
/// a zone of its own. A number captured at start-up is the defect this replaces — a resident core
/// dated every closure an hour off after a DST change.
pub trait LocalZone: Send + Sync + fmt::Debug {
    /// Minutes east of UTC in force at `unix` (epoch seconds).
    fn offset_min_at(&self, unix: i64) -> i32;

    /// `unix` framed in the offset in force at it — the one frame every date-persisting writer
    /// stores, so no date is re-framed on read (§39.2 rules 3 and 5).
    fn at(&self, unix: i64) -> LocalInstant {
        LocalInstant {
            unix,
            offset_min: self.offset_min_at(unix),
        }
    }
}

/// The production zone: the operating system's, through `chrono::Local`, at each instant.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemZone;

impl LocalZone for SystemZone {
    fn offset_min_at(&self, unix: i64) -> i32 {
        use chrono::TimeZone as _;
        // A UTC instant is never ambiguous or skipped in a local zone, so `single` answers for
        // every instant chrono can represent; one hundreds of millennia away is read as UTC.
        // Every real zone is a whole number of minutes, so the division is exact.
        chrono::Local
            .timestamp_opt(unix, 0)
            .single()
            .map_or(0, |t| t.offset().local_minus_utc() / 60)
    }
}

/// An instant and the offset in force at it: the frame a local date is read in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalInstant {
    /// Epoch seconds.
    pub unix: i64,
    /// Minutes east of UTC at `unix` — what a writer stores as `tz_offset_min`.
    pub offset_min: i32,
}

impl LocalInstant {
    /// The local day number: days since 1970-01-01 in this instant's own offset.
    #[must_use]
    pub fn day(self) -> i64 {
        crate::git::local_day(self.unix, self.offset_min)
    }

    /// The local calendar date, `YYYY-MM-DD` — the date a `debt_day` key carries.
    #[must_use]
    pub fn date(self) -> String {
        crate::jobs::j4_history::local_date(self.day())
    }
}

/// The half-open instant range `[start, end)` whose local day is `day` (§39.2 rule 2).
///
/// 23 or 25 hours long across a transition, by construction: each end is found by asking the
/// zone, never by adding 86 400 s. Each end is the first instant whose local day reaches the day,
/// so where local time steps back over midnight the repeated hour belongs to the later day.
#[must_use]
pub fn day_bounds(zone: &dyn LocalZone, day: i64) -> (i64, i64) {
    (
        first_instant_of(zone, day),
        first_instant_of(zone, day.saturating_add(1)),
    )
}

/// The earliest instant whose local day is `day` or later.
fn first_instant_of(zone: &dyn LocalZone, day: i64) -> i64 {
    const SECS_PER_DAY: i64 = 86_400;
    // Every offset is under a day either way, so a day before UTC midnight is still the day
    // before locally and a day after it is already this day: the answer lies between them.
    let midnight = day.saturating_mul(SECS_PER_DAY);
    let mut before = midnight.saturating_sub(SECS_PER_DAY);
    let mut reached = midnight.saturating_add(SECS_PER_DAY);
    while reached - before > 1 {
        let mid = before + (reached - before) / 2;
        if zone.at(mid).day() >= day {
            reached = mid;
        } else {
            before = mid;
        }
    }
    reached
}

/// The machine's local offset from UTC **now**, in minutes east of Greenwich, read once by the
/// composition root — `ProjectsCtx.tz_offset_min` and `CoreHandler.tz_offset_min`.
///
/// **It serves only the readers that are not ring or day boundaries** (§39.2 rule 7): §8.1's era
/// bands, the art pipeline, the Activity windows and the stats. Every writer that persists a
/// local date takes [`LocalZone`] instead and reads the offset at its own instant.
///
/// §8.1's era bands cut on the **local calendar year**, which makes this a correctness input
/// rather than a display detail: an offset that is wrong by a few hours moves a project into the
/// neighbouring era near New Year, and does it silently.
#[must_use]
pub fn local_utc_offset_min() -> i32 {
    // `local_minus_utc` is seconds east of UTC. Every real zone is a whole number of minutes,
    // so the division is exact for every value this can return.
    chrono::Local::now().offset().local_minus_utc() / 60
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
    fn the_offset_is_local_wall_clock_minus_utc() {
        // Derived from the difference between the two wall clocks rather than from
        // `local_minus_utc`, so this does not restate the implementation back to itself.
        let local = chrono::Local::now().naive_local();
        let utc = chrono::Utc::now().naive_utc();
        let observed = (local - utc).num_minutes();
        let reported = i64::from(local_utc_offset_min());
        assert!(
            (observed - reported).abs() <= 1,
            "reported {reported} min, but the two wall clocks differ by {observed} min"
        );
    }

    #[test]
    fn the_offset_is_one_a_real_zone_could_have() {
        // UTC-12 (Baker Island) to UTC+14 (Line Islands) bound every zone in use.
        let offset = local_utc_offset_min();
        assert!(
            (-720..=840).contains(&offset),
            "{offset} min is outside every real UTC offset"
        );
    }
}
