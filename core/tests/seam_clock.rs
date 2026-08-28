//! Compiled only under `testkit`: these link `codotheca_core::testing`, which the feature
//! gates. Without the gate a bare `cargo test` fails to compile rather than skipping, and the
//! real gate — `npm test`, and CI — passes `--features testkit`. `app/test/toolchain.test.ts`
//! asserts that flag is still there, so it cannot be dropped silently.
#![cfg(feature = "testkit")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use codotheca_core::clock::{Clock, SystemClock};
use codotheca_core::testing::FakeClock;

#[test]
fn fake_clock_advances_wall_and_monotonic_together() {
    let clock = FakeClock::new(1_700_000_000);
    assert_eq!(clock.now_unix(), 1_700_000_000);
    let mono = clock.monotonic_ms();
    clock.advance(5);
    assert_eq!(clock.now_unix(), 1_700_000_005);
    assert_eq!(clock.monotonic_ms(), mono + 5_000);
}

#[test]
fn advance_ms_moves_the_monotonic_counter_at_millisecond_resolution() {
    let clock = FakeClock::new(1_700_000_000);
    clock.advance_ms(1_500);
    assert_eq!(clock.monotonic_ms(), 1_500);
    // Wall time is seconds, so the half second is not representable there and is dropped.
    assert_eq!(clock.now_unix(), 1_700_000_001);
}

#[test]
fn setting_wall_time_leaves_the_monotonic_counter_alone() {
    let clock = FakeClock::new(1_700_000_000);
    let mono = clock.monotonic_ms();
    clock.set_unix(4_102_444_800);
    assert_eq!(clock.now_unix(), 4_102_444_800);
    assert_eq!(clock.monotonic_ms(), mono);
}

#[test]
fn wall_time_may_go_backwards_and_monotonic_may_not() {
    let clock = FakeClock::new(1_700_000_000);
    clock.advance(1);
    let mono = clock.monotonic_ms();
    clock.set_unix(1_600_000_000);
    assert_eq!(clock.now_unix(), 1_600_000_000);
    assert_eq!(clock.monotonic_ms(), mono);
}

#[test]
fn sleeping_on_the_fake_returns_at_once_and_is_recorded() {
    let clock = FakeClock::new(1_700_000_000);
    let started = std::time::Instant::now();
    clock.sleep(std::time::Duration::from_millis(2_500));
    assert!(
        started.elapsed().as_millis() < 200,
        "the fake must not really wait"
    );
    assert_eq!(
        clock.sleeps(),
        vec![std::time::Duration::from_millis(2_500)]
    );
    // A backoff loop measures its own progress, so time has to move.
    assert_eq!(clock.monotonic_ms(), 2_500);
    assert_eq!(clock.now_unix(), 1_700_000_002);
}

#[test]
fn a_fake_clock_is_usable_through_the_trait_object() {
    let clock: std::sync::Arc<dyn Clock> = std::sync::Arc::new(FakeClock::new(0));
    assert_eq!(clock.now_unix(), 0);
}

#[test]
fn system_clock_monotonic_never_decreases() {
    let clock = SystemClock::new();
    let first = clock.monotonic_ms();
    let second = clock.monotonic_ms();
    assert!(second >= first);
    assert!(clock.now_unix() > 1_600_000_000);
    assert!(
        clock.now_unix() < 4_102_444_800,
        "seconds, not milliseconds"
    );
}
