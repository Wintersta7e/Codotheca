//! §24.6's single warranted primitive: the one place in the core that removes a user-visible
//! tree, and the evidence it demands first.
//!
//! **R63: `remove_warranted` takes no path.** The authorised path is a field of the [`Warrant`],
//! so *"remove the directory one level up"* — the real working copy beside the staging root — is
//! not a sentence this API can express. That is a type-level guarantee, not a runtime check, and
//! `core/tests/removal_audit.rs` records that it is.
//!
//! Everything else in `core/src` that removes bytes removes **app-owned** bytes — a lock file, a
//! temporary raster, a fixture corpus, a database backup, a credential socket. Those are a
//! measured baseline pinned per line in `acceptance/callsites.json`, not a wildcard, so an
//! eighteenth cannot appear silently inside an already-listed file.

pub mod trash;
pub mod warrant;

use std::path::{Component, Path};

pub use trash::{HardDelete, SystemTrash, Trash, TrashAvailability, TrashRefusal};
pub use warrant::{SessionNonce, Warrant, WarrantKind, WarrantVariant};

use crate::analyser::identity::LiveIdentity;
use crate::install::staging::STAGING_DIR_NAME;

/// What happened to the bytes, said by whoever removed them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemovalOutcome {
    /// Recoverable from the Recycle Bin / XDG trash.
    Trashed,
    /// Gone. Only ever the answer for bytes this process wrote in this session.
    HardDeleted,
}

/// Why a removal did not happen.
///
/// p2-24b adds `IdentityChanged` for §24.7E's identity check, which cannot apply to a partial
/// clone and is therefore not claimed here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemovalRefusal {
    /// The warrant's own evidence did not hold. The string names which clause.
    WarrantFailed(&'static str),
    /// The target is a symlink. It was **not** followed.
    SymlinkedPath,
    /// The authorised path is not strictly inside the warranted root.
    OutsideWarrantedRoot,
    /// The removal was attempted and the platform refused.
    Io(String),
    /// [p2-24b] §24.7E: the directory is no longer the repository its row describes.
    IdentityChanged,
}

/// Remove the one path this warrant authorises.
///
/// `identity_now` is the directory's identity re-derived at this moment; a staging warrant
/// carries no identity guard and its callers pass [`LiveIdentity::Underivable`].
///
/// # Errors
/// Refuses when the warrant's evidence does not hold, when the target is a symlink, when the
/// path is not strictly inside the warranted root, or when the removal itself fails.
pub fn remove_warranted(
    warrant: &Warrant,
    destination: &dyn Trash,
    identity_now: &LiveIdentity,
) -> Result<RemovalOutcome, RemovalRefusal> {
    let path = warrant.path();
    match warrant.kind() {
        WarrantKind::Staging {
            staging_root,
            created_in_session,
            ..
        } => {
            // "In this session" is checked, not assumed: a staging directory left by a previous
            // run of this core carries a different nonce and is the start-up sweep's problem to
            // report, never this function's to delete.
            if *created_in_session != SessionNonce::current() {
                return Err(RemovalRefusal::WarrantFailed(
                    "the warrant was not minted by this run of the core",
                ));
            }
            // A forged `staging_root` is the way round the containment check, so the root has to
            // prove it is one: its final component is the single literal §4.3's skip list also
            // reads, stated once in `install::staging`.
            if staging_root.file_name() != Some(std::ffi::OsStr::new(STAGING_DIR_NAME)) {
                return Err(RemovalRefusal::WarrantFailed(
                    "the warranted root is not a staging directory",
                ));
            }
            if !is_strictly_inside(path, staging_root) {
                return Err(RemovalRefusal::OutsideWarrantedRoot);
            }
        }
        // [p2-24b] §24.7E: identity at the moment of removal, **re-derived, never remembered**.
        //
        // The lineage and not `head_oid`: a tip moves with every commit, so a guard over it
        // would refuse a working copy the user had merely committed to, and admit one rewound
        // onto the same tip. The root set is what makes this repository *this* repository.
        //
        // The re-derivation is the caller's — it needs a git seam this module deliberately does
        // not hold — so what is checked here is that it agrees with the **row's** lineage the
        // warrant carries (§45.6 step 1). An underivable identity is a refusal, never a match.
        WarrantKind::Uninstall {
            expected_lineage, ..
        } => match identity_now {
            LiveIdentity::Derived(live) if live == expected_lineage => {}
            LiveIdentity::Derived(_) => return Err(RemovalRefusal::IdentityChanged),
            LiveIdentity::Underivable => {
                return Err(RemovalRefusal::WarrantFailed(
                    "the directory's identity could not be re-derived at removal time",
                ))
            }
        },
    }

    // `symlink_metadata` does not follow, which is the whole point: following one would remove
    // whatever it aims at, and a partial clone has no business containing one at its own root.
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => return Err(RemovalRefusal::SymlinkedPath),
        Ok(_) => {}
        Err(error) => return Err(RemovalRefusal::Io(error.to_string())),
    }

    destination.send(path).map_err(|refusal| match refusal {
        TrashRefusal::Io(message) => RemovalRefusal::Io(message),
        other => RemovalRefusal::Io(format!("{other:?}")),
    })
}

/// Is `path` exactly one ordinary component below `root`?
///
/// **`..` is rejected outright rather than normalised.** `Path::parent` is lexical, so
/// `<root>/..` has `<root>` as its parent and would satisfy a naive parent comparison while
/// naming the directory *above* the staging root — which is where the user's real working copies
/// live. Nothing upstream is trusted to have stripped it.
fn is_strictly_inside(path: &Path, root: &Path) -> bool {
    if path.components().any(|c| c == Component::ParentDir) {
        return false;
    }
    let Some(parent) = path.parent() else {
        return false;
    };
    parent == root
        && path
            .file_name()
            .is_some_and(|name| !name.is_empty() && name != std::ffi::OsStr::new(".."))
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use super::is_strictly_inside;
    use std::path::Path;

    #[test]
    fn one_ordinary_component_below_the_root_is_inside() {
        assert!(is_strictly_inside(
            Path::new("/r/.codotheca-installing/widget"),
            Path::new("/r/.codotheca-installing")
        ));
    }

    /// The whole reason this is not a `parent()` comparison.
    #[test]
    fn a_parent_component_is_refused_even_though_its_lexical_parent_matches() {
        let root = Path::new("/r/.codotheca-installing");
        let escape = Path::new("/r/.codotheca-installing/..");
        assert_eq!(
            escape.parent(),
            Some(root),
            "the lexical parent really does match — that is the hazard"
        );
        assert!(!is_strictly_inside(escape, root));
        assert!(!is_strictly_inside(
            Path::new("/r/.codotheca-installing/../widget"),
            root
        ));
    }

    #[test]
    fn the_root_itself_and_anything_deeper_are_refused() {
        let root = Path::new("/r/.codotheca-installing");
        assert!(!is_strictly_inside(root, root));
        assert!(!is_strictly_inside(
            Path::new("/r/.codotheca-installing/widget/nested"),
            root
        ));
        assert!(!is_strictly_inside(Path::new("/r/widget"), root));
    }
}
