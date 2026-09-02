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

/// A parsed `git --version`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GitVersion {
    /// Major component.
    pub major: u32,
    /// Minor component.
    pub minor: u32,
    /// Patch component; `0` when the vendor string omits it.
    pub patch: u32,
    /// The trimmed line as git printed it, for `app_meta.git_version` and the upgrade hint.
    pub raw: String,
}

/// Parse the output of `git --version`. Returns `None` when no version token is present,
/// which the caller reports as `GIT_MISSING` rather than guessing — an unparsed version is
/// not a version zero.
#[must_use]
pub fn parse_version(output: &[u8]) -> Option<GitVersion> {
    let text = String::from_utf8_lossy(output);
    let raw = text.trim().to_owned();
    let token = raw
        .split_whitespace()
        .find(|t| t.starts_with(|c: char| c.is_ascii_digit()))?;
    let mut parts = token.split('.');
    let major = parts.next()?.parse::<u32>().ok()?;
    let minor = parts
        .next()
        .and_then(|p| p.parse::<u32>().ok())
        .unwrap_or(0);
    let patch = parts
        .next()
        .and_then(|p| p.parse::<u32>().ok())
        .unwrap_or(0);
    Some(GitVersion {
        major,
        minor,
        patch,
        raw,
    })
}

/// True when `v` is at or above [`GIT_FLOOR`].
#[must_use]
pub fn meets_floor(v: &GitVersion) -> bool {
    (v.major, v.minor) >= GIT_FLOOR
}
