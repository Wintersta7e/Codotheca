//! The `MountResolver` seam (§4.7, §15.2).
//!
//! Two device identities, for two different jobs. `store_key` is runtime scheduling: the
//! device or share a path currently lives on, cheap and not persisted. `volume_key` is
//! persistent and best-effort, so a location can be recognised when a drive comes back.
//! Read straight from the OS, the removable-drive criterion is untestable by construction.

use std::fmt;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// How fast the backing store is, in the only granularity §3.4 acts on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StoreClass {
    Local,
    Removable,
    Network,
    Fuse,
    Hdd,
    Unknown,
}

impl StoreClass {
    /// Concurrent git processes allowed against one store (§3.4). One for the slow classes,
    /// four for everything else, fixed — a storage-speed measurement subsystem to choose
    /// between four and eight is machinery for an unobservable difference at this scale.
    #[must_use]
    pub fn per_store_cap(self) -> u32 {
        match self {
            Self::Removable | Self::Network | Self::Fuse | Self::Hdd => 1,
            Self::Local | Self::Unknown => 4,
        }
    }
}

/// What a resolver knows about the store under a path.
///
/// Serde is not decoration (R7): all three fields become `location` columns, and the WSL
/// worker (plan 18) resolves a mount in the distro and sends these facts back to the host.
/// The field names are the column names, so no `rename_all` — a rename here would be one
/// value spelled two ways.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MountFacts {
    pub store_key: String,
    /// `None` where no stable identifier exists — a bind mount, overlayfs, tmpfs. Absent is
    /// not the same as unknown-and-therefore-zero: a location with no volume key can never be
    /// recognised across a remount, and callers must handle that rather than invent one.
    pub volume_key: Option<String>,
    pub class: StoreClass,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MountError {
    /// The store this path belongs to is not mounted right now (§4.6: the location is
    /// `offline`, and nothing is deleted).
    NotMounted,
    /// No mapping exists. The caller must not substitute a default.
    Unsupported(String),
    Io(String),
}

impl fmt::Display for MountError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotMounted => write!(f, "store not mounted"),
            Self::Unsupported(m) => write!(f, "unsupported mount: {m}"),
            Self::Io(m) => write!(f, "mount io: {m}"),
        }
    }
}

impl std::error::Error for MountError {}

pub trait MountResolver: Send + Sync + fmt::Debug {
    /// Facts about the store under `path`. Returns `NotMounted` when the store is absent.
    fn resolve(&self, path: &Path) -> Result<MountFacts, MountError>;
    /// Whether a previously recorded `volume_key` is mounted now. This is what turns an
    /// `offline` location back into a `present` one without a full walk.
    fn is_volume_mounted(&self, volume_key: &str) -> bool;
}
