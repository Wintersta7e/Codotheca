//! §4.5 — never traverse the WSL bridge.
//!
//! The bridge is a 9p share. Walking it means every `stat` is a round trip through a virtual
//! machine, which is the slow path the WSL worker (§13) exists to replace. So the walk stops at
//! the boundary, names the distro, and lets plan 18 dispatch in-distro work over the protocol.

use std::path::Path;

/// A distro named by a bridge path the walk refused to enter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WslBridgeRef {
    /// The distro name as the path spells it, case kept.
    pub distro: String,
}

const WSL_DOLLAR: &str = r"\\wsl$\";
const WSL_LOCALHOST: &str = r"\\wsl.localhost\";

/// `Some` when `path` is inside the WSL bridge. The walk must return without descending.
#[must_use]
pub fn wsl_boundary(path: &Path) -> Option<WslBridgeRef> {
    let raw = path.to_string_lossy().replace('/', "\\");
    let lower = raw.to_lowercase();
    // Both prefixes are ASCII, so a byte offset taken from the lowercased copy is valid on the
    // original — which is what lets the distro keep its case while the host is folded.
    let rest_at = if lower.starts_with(WSL_DOLLAR) {
        WSL_DOLLAR.len()
    } else if lower.starts_with(WSL_LOCALHOST) {
        WSL_LOCALHOST.len()
    } else {
        return None;
    };
    let distro = raw.get(rest_at..)?.split('\\').next().unwrap_or_default();
    if distro.is_empty() {
        return None;
    }
    Some(WslBridgeRef {
        distro: distro.to_owned(),
    })
}

/// True when a Linux filesystem type is a Windows drive surfaced inside a distro.
///
/// §4.5: an in-distro scan classifies by **type**, never by matching `/mnt/[a-z]` by name — a
/// user may mount anything anywhere, and `/mnt/data` on ext4 is not a bridge.
#[must_use]
pub fn is_drvfs_fstype(fstype: &str) -> bool {
    matches!(
        fstype.to_ascii_lowercase().as_str(),
        "9p" | "v9fs" | "drvfs"
    )
}
