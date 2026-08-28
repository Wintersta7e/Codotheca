//! Test doubles for the three injection seams §15.2 requires. Compiled only under the
//! `testkit` feature, so none of it can reach a shipped binary.

mod clock;

pub use clock::FakeClock;
