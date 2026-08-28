//! §5's derived values. **This module is plan 09's**; the one type below is landed early because
//! plan 07 needs it and R21 forbids a second copy.
//!
//! R21 settled `LocationKind` on plan 09 after finding it declared identically in two plans. The
//! alternative to landing it here was a local duplicate in `core::scan`, which is the collision
//! the ruling exists to prevent — the same shape as plan 02 needing `GIT_FLOOR` before plan 05
//! ran, which was resolved the same way.

/// Which world a `location` row's path belongs to — `location.kind` (§1.3).
///
/// **R31: declared in `protocol/schema/protocol.json` and generated into `crate::protocol`.**
/// Re-exported so this module's path still names it, and declared nowhere else — a second
/// hand-written copy compiles and then drifts from the wire form. The generated variant renames
/// are `win`/`linux`/`wsl`, matching `as_str` below character for character, which is what the
/// column's `CHECK (kind IN ('win', 'linux', 'wsl'))` requires; a mismatch would fail at insert
/// time rather than in review. Same treatment as `Outcome` in `core/src/proto/wire.rs`.
pub use crate::protocol::LocationKind;

impl LocationKind {
    /// Every kind, so a test can walk the vocabulary without restating it.
    pub const ALL: [Self; 3] = [Self::Win, Self::Linux, Self::Wsl];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Win => "win",
            Self::Linux => "linux",
            Self::Wsl => "wsl",
        }
    }

    /// `as_str`'s inverse. `None` for anything else: a `kind` this build does not know is a row
    /// from a newer schema, and guessing would file a Windows path under Linux path rules.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "win" => Some(Self::Win),
            "linux" => Some(Self::Linux),
            "wsl" => Some(Self::Wsl),
            _ => None,
        }
    }
}
