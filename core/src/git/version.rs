//! The git version floor.
//!
//! §3.1: the core refuses to run below this, and records the observed version in
//! `app_meta.git_version` at startup.
//!
//! The shell mirrors this value in `app/src/shared/gitFloor.ts`, because §11.2a's git-floor
//! failure window has to print it and the protocol carries no compile-time constant.
//! `gitFloor.test.ts` reads *this file* and asserts the two agree — R24's condition on a
//! cross-language mirror. Change one and that test fails.

/// `(major, minor)`. git 2.22 is the floor because it is the first release whose
/// `--porcelain=v2` status output the scanner relies on is stable.
pub const GIT_FLOOR: (u32, u32) = (2, 22);
