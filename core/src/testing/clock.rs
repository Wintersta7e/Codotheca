use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use crate::clock::Clock;

/// A clock the test drives.
///
/// Wall time and the monotonic counter move independently on purpose: criterion 62 advances
/// the system clock by an arbitrary amount and asserts nothing content-addressed changed.
#[derive(Debug)]
pub struct FakeClock {
    unix_secs: AtomicI64,
    mono_ms: AtomicU64,
    slept: Mutex<Vec<Duration>>,
}

impl FakeClock {
    /// A clock at `unix_secs` wall time and monotonic zero, with no sleep recorded.
    #[must_use]
    pub const fn new(unix_secs: i64) -> Self {
        Self {
            unix_secs: AtomicI64::new(unix_secs),
            mono_ms: AtomicU64::new(0),
            slept: Mutex::new(Vec::new()),
        }
    }

    /// Move both clocks forward by whole seconds. This is what a test does to let time pass.
    pub fn advance(&self, secs: i64) {
        self.mono_ms.fetch_add(
            u64::try_from(secs.saturating_mul(1_000)).unwrap_or(0),
            Ordering::SeqCst,
        );
        self.unix_secs.fetch_add(secs, Ordering::SeqCst);
    }

    /// Move both clocks forward at millisecond resolution, for budgets and timeouts stated in
    /// milliseconds. Wall time is seconds, so a sub-second remainder is not visible there.
    pub fn advance_ms(&self, ms: u64) {
        self.mono_ms.fetch_add(ms, Ordering::SeqCst);
        self.unix_secs.fetch_add(
            i64::try_from(ms / 1_000).unwrap_or(i64::MAX),
            Ordering::SeqCst,
        );
    }

    /// Move wall time only — a clock jump, an NTP correction, a user changing the date.
    pub fn set_unix(&self, unix_secs: i64) {
        self.unix_secs.store(unix_secs, Ordering::SeqCst);
    }

    /// Every duration passed to `sleep`, in order. A backoff test asserts the schedule it
    /// asked for rather than how long the suite actually took.
    #[must_use]
    pub fn sleeps(&self) -> Vec<Duration> {
        self.slept.lock().map_or_else(|_| Vec::new(), |v| v.clone())
    }
}

impl Clock for FakeClock {
    fn now_unix(&self) -> i64 {
        self.unix_secs.load(Ordering::SeqCst)
    }

    fn monotonic_ms(&self) -> u64 {
        self.mono_ms.load(Ordering::SeqCst)
    }

    /// Returns at once. The request is recorded and time moves forward by it, so a retry loop
    /// that measures its own elapsed time makes progress instead of spinning.
    fn sleep(&self, dur: Duration) {
        if let Ok(mut v) = self.slept.lock() {
            v.push(dur);
        }
        self.advance_ms(u64::try_from(dur.as_millis()).unwrap_or(u64::MAX));
    }
}
