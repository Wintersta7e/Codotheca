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

/// The machine's local offset from UTC, in minutes east of Greenwich.
///
/// Not a `Clock` method: the trait is time a test injects, and this is a property of the machine
/// the composition root reads **once** and passes down as data — `ProjectsCtx.tz_offset_min`.
/// Every consumer takes it as an argument, so nothing below the root reads a zone.
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
