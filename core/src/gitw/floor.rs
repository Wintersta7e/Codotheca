//! §47.8: the governed-git floor — the lowest git release a governed act (§45.1) runs on.
//!
//! **Separate from [`crate::git::GIT_FLOOR`], which it does not move.** The app floor is what the
//! scanner needs to read a repository; this one is what the verifying read needs to write
//! objects and nothing else. Below it every governed act is unknown and runs no weaker read
//! (PA1): there is no fallback argv without `--stdin` or `--no-write-fetch-head`, because a
//! fallback is the retired fetch by another name. `Clone` is not governed and runs on every git
//! the app runs on.
//!
//! **The value is measured, not looked up.** It is the lowest release on which both
//! `core/tests/git_write_differential.rs` (§47.9 C and the named regressions) and
//! `core/tests/git_analyse.rs` pass with the product's git pointed at the candidate through the
//! test-only `CODOTHECA_TEST_GIT`. Each release was built from its upstream tarball (checked
//! against the published `sha256sums.asc`) with `NO_GETTEXT NO_TCLTK NO_PERL NO_PYTHON`, curl
//! on, and run on WSL (Linux x86-64). Measured 2026-09-25 on `b463724` plus this change's
//! harness:
//!
//! | git | `git_write_differential` | `git_analyse` | result |
//! |---|---|---|---|
//! | 2.55.0 (the newest; the Windows git `config_keys.rs` also lists) | 13 passed, all four routes | 10 passed | pass; the reftable fixture read correctly |
//! | 2.43.0 (the gate's) | 13 passed | 10 passed | pass |
//! | 2.29.0 | 13 passed; not run: both M3 tests (it lists no `fetch.bundleURI` or `transfer.bundleURI`) and the `GIT_CONFIG_COUNT` route inside layer C and M2 (it does not read it) | 10 passed | **pass — the floor** |
//! | 2.28.0 | 9 passed, 4 failed: `fetch --stdin` is an unknown option, so every objects step fails | 10 passed | fail |
//!
//! A test not run is a hazard the release cannot have — it has no bundle URIs and no
//! environment config route — and each prints the release and the missing feature, as the
//! reftable fixture does (R207). 2.29.0 is also `47:338-340`'s lower bound, from the release
//! notes' `--no-write-fetch-head`; the notes do not date `fetch --stdin`, and 2.28.0 above is
//! the measurement that it is absent there.

use crate::git::GitVersion;

/// `(major, minor, patch)` of the lowest git a governed intent runs on.
pub const GOVERNED_GIT_FLOOR: (u32, u32, u32) = (2, 29, 0);

/// True when `v` is at or above [`GOVERNED_GIT_FLOOR`].
#[must_use]
pub fn meets_governed_floor(v: &GitVersion) -> bool {
    (v.major, v.minor, v.patch) >= GOVERNED_GIT_FLOOR
}
