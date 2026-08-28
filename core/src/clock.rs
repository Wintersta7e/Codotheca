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
