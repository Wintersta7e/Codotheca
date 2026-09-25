//! §24.6's warrant: the authority to remove one path, and the evidence behind it.
//!
//! **R63: the authorised path is a FIELD, not a parameter.** `remove_warranted(&Warrant, &dyn
//! Trash)` takes no path of its own, so a caller holding a warrant for one directory has no way
//! to express *"remove that other one"*. Passing the two separately would admit exactly that,
//! and the directory one level up from a staging path is the user's real working copy.
//!
//! The warrant is also **unforgeable from a path**: there is no constructor that accepts one.
//! `install::staging::staging_warrant_for` builds the warrant and its path together out of a
//! durable `install_run` row, which is what makes *"a path this process created, in this
//! session, recorded before the first byte was written"* checkable rather than asserted.

use std::path::{Path, PathBuf};

use crate::analyser::verdict::VerdictSeal;
use crate::protocol::{InstallRunId, LocationId};

/// A per-process random value, minted once at core start.
///
/// It is what makes *"created in this session"* a check rather than an assumption: a staging
/// directory left by a previous run of this core carries a different nonce, so the start-up
/// sweep cannot mistake it for one this process is still writing into. Two cores running over
/// one library would likewise not claim each other's work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionNonce([u8; 16]);

impl SessionNonce {
    /// This process's nonce, minted on first use and constant thereafter.
    #[must_use]
    pub fn current() -> Self {
        static CURRENT: std::sync::OnceLock<SessionNonce> = std::sync::OnceLock::new();
        *CURRENT.get_or_init(Self::mint)
    }

    /// A fresh nonce. `pub` only under `testkit`, so no production caller can mint a second one
    /// and then claim a directory this process did not create.
    #[cfg(feature = "testkit")]
    #[must_use]
    pub fn mint_for_test() -> Self {
        Self::mint()
    }

    /// Sixteen bytes from the clock and the process id.
    ///
    /// Not a CSPRNG and not required to be: this is a same-process identity check, not a secret.
    /// Its only job is that a nonce minted by a *different run of this core* does not collide
    /// with this one. The clock gives that on its own; the process id closes the remaining case,
    /// two cores starting within the same nanosecond, which the clock cannot separate and which
    /// is exactly the two-cores-over-one-library situation the nonce exists for.
    ///
    /// No `as` cast: the low eight bytes of the nanosecond count are taken from its own
    /// little-endian bytes, so the truncation is the code rather than a lint to be silenced.
    fn mint() -> Self {
        let mut bytes = [0_u8; 16];
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0_u128, |d| d.as_nanos())
            .to_le_bytes();
        bytes[..8].copy_from_slice(&nanos[..8]);
        bytes[8..12].copy_from_slice(&std::process::id().to_le_bytes());
        bytes[12..].copy_from_slice(&nanos[8..12]);
        Self(bytes)
    }
}

/// The discriminant list's element — `IntentKind`'s role for warrants.
///
/// **Deviation from the plan's table, which writes `Warrant::ALL: [WarrantKind; 1]`.**
/// `WarrantKind` carries per-variant evidence, so it cannot be the element of a `const` array;
/// `core/src/gitw/intent.rs:164` already solves this with a fieldless companion
/// (`Intent::ALL: [IntentKind; 2]`), and this is that shape. p2-24b raises the length to 2 when
/// it adds `Uninstall`, exactly as the plan intends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WarrantVariant {
    /// A partial clone under `<root>/.codotheca-installing/`.
    Staging,
    /// [p2-24b] A user's working copy, under §24.7E's re-derived identity.
    Uninstall,
}

/// What a warrant authorises, and the evidence that earned it.
///
/// The variants are **discriminated rather than merged** because their identity guards differ:
/// §24.7E's root-commit-SHA check cannot apply to a partial clone, which has no valid root
/// commit to match. Merging them would mean one of the two guarantees is claimed without being
/// checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WarrantKind {
    /// §24.7E: a working copy, identified at the **moment of removal** against the **row**.
    ///
    /// **The lineage, not `head_oid`.** A tip moves with every commit; the root set is what
    /// makes this repository *this* repository. A guard over the tip would refuse a working copy
    /// the user had merely committed to, and admit one that had been rewound onto the same tip —
    /// it is a position, not an identity.
    ///
    /// [p4] **Expected from the row, never from the copy** (§45.6 step 1). Phase 2 re-derived
    /// the expected identity from the directory it was about to compare, so a replaced directory
    /// matched itself (§37.8).
    Uninstall {
        /// The row this warrant is for.
        location_id: LocationId,
        /// `project.lineage_key` as read from the row in this call. `None` matches only a live
        /// repository with no commits that is not shallow.
        expected_lineage: Option<String>,
        /// The verdict this removal was authorised by. **In-core only**: it never crossed a call
        /// boundary to get here, which is what stops a renderer replaying one.
        verdict: VerdictSeal,
    },
    /// §24.3c: bytes this process wrote, in this session, into its own staging directory.
    Staging {
        /// The durable row written before the first byte.
        run_id: InstallRunId,
        /// `<root>/.codotheca-installing`. The authorised path must be strictly inside it.
        staging_root: PathBuf,
        /// Which run of this core created it.
        created_in_session: SessionNonce,
    },
}

impl WarrantKind {
    /// This kind's fieldless discriminant.
    #[must_use]
    pub const fn variant(&self) -> WarrantVariant {
        match self {
            Self::Staging { .. } => WarrantVariant::Staging,
            Self::Uninstall { .. } => WarrantVariant::Uninstall,
        }
    }
}

/// Authority to remove exactly one path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Warrant {
    kind: WarrantKind,
    path: PathBuf,
}

impl Warrant {
    /// Every warrant variant, for the audit to iterate.
    ///
    /// Its length is a deliberate tripwire on its own growth, in `Intent::ALL`'s shape
    /// (`core/src/gitw/intent.rs:164`). **p2-24b raises this to 2** in the change that adds
    /// `WarrantKind::Uninstall`, and raises the audit's floor with it.
    pub const ALL: [WarrantVariant; 2] = [WarrantVariant::Staging, WarrantVariant::Uninstall];

    /// Build a staging warrant from evidence. **There is no constructor taking a bare path**
    /// (R63): the authorised path is composed here from the staging root and the one basename
    /// the run is for, so no caller can name a directory outside it — including the real
    /// destination one level up, which is the removal this type exists to make unwritable.
    #[must_use]
    pub(crate) fn for_staging(
        run_id: InstallRunId,
        staging_root: PathBuf,
        seed_basename: &str,
        created_in_session: SessionNonce,
    ) -> Self {
        let path = staging_root.join(seed_basename);
        Self {
            kind: WarrantKind::Staging {
                run_id,
                staging_root,
                created_in_session,
            },
            path,
        }
    }

    /// Build an uninstall warrant. **There is no constructor taking a bare path** (R63): the
    /// path comes from the `location` row this warrant is for, and the caller cannot substitute
    /// another. `expected_lineage` is the same row's `project.lineage_key`.
    #[must_use]
    pub(crate) const fn for_uninstall(
        location_id: LocationId,
        path: PathBuf,
        expected_lineage: Option<String>,
        verdict: VerdictSeal,
    ) -> Self {
        Self {
            kind: WarrantKind::Uninstall {
                location_id,
                expected_lineage,
                verdict,
            },
            path,
        }
    }

    /// `for_uninstall` for the audit, which lives outside this crate. Testkit-only.
    #[cfg(feature = "testkit")]
    #[must_use]
    pub const fn for_uninstall_in_test(
        location_id: LocationId,
        path: PathBuf,
        expected_lineage: Option<String>,
        verdict: VerdictSeal,
    ) -> Self {
        Self::for_uninstall(location_id, path, expected_lineage, verdict)
    }

    /// `for_staging` for the audit, which lives outside this crate.
    ///
    /// Testkit-only, so no shipped binary contains it. It still **composes** the path from a root
    /// and one basename rather than accepting one, so even here R63 holds: a test cannot name the
    /// working copy either.
    #[cfg(feature = "testkit")]
    #[must_use]
    pub fn for_staging_in_test(
        staging_root: PathBuf,
        seed_basename: &str,
        created_in_session: SessionNonce,
    ) -> Self {
        Self::for_staging(
            InstallRunId(i64::MIN),
            staging_root,
            seed_basename,
            created_in_session,
        )
    }

    /// The one path this warrant authorises.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The evidence behind it.
    #[must_use]
    pub const fn kind(&self) -> &WarrantKind {
        &self.kind
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use super::{SessionNonce, Warrant, WarrantVariant};

    #[test]
    fn the_variant_list_is_two_and_the_two_are_discriminated() {
        assert_eq!(
            Warrant::ALL.len(),
            2,
            "[p2-24b] raised from 1 with WarrantKind::Uninstall — the tripwire working"
        );
        assert_eq!(Warrant::ALL[0], WarrantVariant::Staging);
        assert_eq!(Warrant::ALL[1], WarrantVariant::Uninstall);
        assert_ne!(
            Warrant::ALL[0],
            Warrant::ALL[1],
            "their identity guards differ and neither is claimable for the other"
        );
    }

    #[test]
    fn the_session_nonce_is_stable_within_a_process() {
        assert_eq!(SessionNonce::current(), SessionNonce::current());
    }

    #[test]
    fn a_freshly_minted_nonce_is_not_this_session() {
        assert_ne!(SessionNonce::mint_for_test(), SessionNonce::current());
    }
}
