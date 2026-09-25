//! §46.7: **why the recycle bin cannot take a copy, said before the click.**
//!
//! [`BinSettings`] reads what a platform's bin will do with a path; [`trash_refusal_for`] is the
//! one classifier over what it read, platform-free, so every reason has one producer. **Any input
//! that cannot be read is `capacity_unknown`, never a default** (D-8): a bin whose limits are
//! guessed is a silent permanent delete waiting for a large folder.
//!
//! The Windows key set is the starting reading of the shell's documented settings. **It is not
//! yet measured**: §46.7's probe on a disposable Windows VM replaces it, and until then
//! `oversized_folder` is a first-tag blocker rather than a measured one.

use std::path::Path;

use crate::protocol::TrashRefusalKind;

/// What a platform's bin will do with one path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BinFacts {
    /// There is no bin to send to.
    NoTrash,
    /// A bin with no limit the platform enforces: the freedesktop trash renames and never
    /// deletes.
    NoLimits,
    /// A volume's bin, as its settings read.
    Volume {
        /// The path is on a network share, which Windows does not recycle.
        network: bool,
        /// The volume's bin deletes outright instead of keeping.
        nuke_on_delete: bool,
        /// Bytes the bin will still take on this volume.
        capacity_left_bytes: u64,
    },
    /// Some setting the answer depends on could not be read.
    Unreadable,
}

/// A platform's bin settings, read without writing anything.
pub trait BinSettings: Send + Sync + std::fmt::Debug {
    /// What the bin will do with `path`.
    fn for_path(&self, path: &Path) -> BinFacts;
}

/// The one classifier (§46.7): the refusal to name, or `None` when the bin will take the copy.
/// `tree_bytes` is asked only when a remaining capacity must be compared.
#[must_use]
pub fn trash_refusal_for(
    facts: &BinFacts,
    tree_bytes: impl FnOnce() -> Option<u64>,
) -> Option<TrashRefusalKind> {
    match facts {
        BinFacts::NoTrash => Some(TrashRefusalKind::Unsupported),
        BinFacts::NoLimits => None,
        BinFacts::Unreadable => Some(TrashRefusalKind::CapacityUnknown),
        BinFacts::Volume { network: true, .. } => Some(TrashRefusalKind::NetworkDrive),
        BinFacts::Volume {
            nuke_on_delete: true,
            ..
        } => Some(TrashRefusalKind::DisabledOnVolume),
        BinFacts::Volume {
            capacity_left_bytes,
            ..
        } => match tree_bytes() {
            None => Some(TrashRefusalKind::CapacityUnknown),
            Some(bytes) if bytes > *capacity_left_bytes => Some(TrashRefusalKind::OversizedFolder),
            Some(_) => None,
        },
    }
}

/// The bytes under `path`, never following a symlink. `None` when any part cannot be read.
#[must_use]
pub fn tree_bytes(path: &Path) -> Option<u64> {
    let meta = std::fs::symlink_metadata(path).ok()?;
    if !meta.is_dir() {
        return Some(meta.len());
    }
    let mut total = 0_u64;
    for child in std::fs::read_dir(path).ok()? {
        total = total.saturating_add(tree_bytes(&child.ok()?.path())?);
    }
    Some(total)
}

/// The platform's own settings.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemBinSettings;

impl BinSettings for SystemBinSettings {
    /// The freedesktop trash is a directory under the user's data home. With no home there is
    /// nowhere to put anything; with one, the crate renames into it and never deletes.
    #[cfg(not(windows))]
    fn for_path(&self, _path: &Path) -> BinFacts {
        if std::env::var_os("XDG_DATA_HOME").is_none() && std::env::var_os("HOME").is_none() {
            BinFacts::NoTrash
        } else {
            BinFacts::NoLimits
        }
    }

    /// The Recycle Bin, read-only through the registry: a UNC path is a network share; otherwise
    /// the volume's own `NukeOnDelete` and `MaxCapacity`, the Explorer policy, and the bytes the
    /// bin already holds on that volume.
    #[cfg(windows)]
    fn for_path(&self, path: &Path) -> BinFacts {
        windows::facts(path)
    }
}

#[cfg(windows)]
mod windows {
    use std::path::{Component, Path, Prefix};

    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    use winreg::RegKey;

    use super::BinFacts;

    const BIT_BUCKET: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\BitBucket\Volume";
    const POLICIES: &str = r"Software\Microsoft\Windows\CurrentVersion\Policies\Explorer";
    const MOUNTED_DEVICES: &str = r"SYSTEM\MountedDevices";
    const MIB: u64 = 1024 * 1024;

    /// Where a path lives: a drive letter, a network share, or neither.
    enum Place {
        Drive(char),
        Network,
        Unknown,
    }

    fn place_of(path: &Path) -> Place {
        match path.components().next() {
            Some(Component::Prefix(prefix)) => match prefix.kind() {
                Prefix::Disk(letter) | Prefix::VerbatimDisk(letter) => {
                    Place::Drive(char::from(letter).to_ascii_uppercase())
                }
                Prefix::UNC(..) | Prefix::VerbatimUNC(..) => Place::Network,
                _ => Place::Unknown,
            },
            _ => Place::Unknown,
        }
    }

    pub(super) fn facts(path: &Path) -> BinFacts {
        let drive = match place_of(path) {
            Place::Drive(letter) => letter,
            Place::Network => {
                return BinFacts::Volume {
                    network: true,
                    nuke_on_delete: false,
                    capacity_left_bytes: 0,
                }
            }
            Place::Unknown => return BinFacts::Unreadable,
        };
        let Some(guid) = volume_guid(drive) else {
            return BinFacts::Unreadable;
        };
        let Ok(volume) =
            RegKey::predef(HKEY_CURRENT_USER).open_subkey(format!(r"{BIT_BUCKET}\{guid}"))
        else {
            return BinFacts::Unreadable;
        };
        let (Ok(nuke), Ok(max_mib)) = (
            volume.get_value::<u32, _>("NukeOnDelete"),
            volume.get_value::<u32, _>("MaxCapacity"),
        ) else {
            return BinFacts::Unreadable;
        };
        let Some(policy) = no_recycle_policy() else {
            return BinFacts::Unreadable;
        };
        let Some(held) = bytes_held(drive) else {
            return BinFacts::Unreadable;
        };
        BinFacts::Volume {
            network: false,
            nuke_on_delete: nuke != 0 || policy,
            capacity_left_bytes: u64::from(max_mib).saturating_mul(MIB).saturating_sub(held),
        }
    }

    /// `\DosDevices\<drive>:`'s device bytes, matched to the `\??\Volume{GUID}` value holding the
    /// same bytes. `None` when either cannot be read.
    fn volume_guid(drive: char) -> Option<String> {
        let devices = RegKey::predef(HKEY_LOCAL_MACHINE)
            .open_subkey(MOUNTED_DEVICES)
            .ok()?;
        let wanted = format!(r"\DosDevices\{drive}:");
        let mut target = None;
        let mut volumes = Vec::new();
        for value in devices.enum_values() {
            let (name, data) = value.ok()?;
            if name.eq_ignore_ascii_case(&wanted) {
                target = Some(data.bytes.into_owned());
            } else if let Some(guid) = name.strip_prefix(r"\??\Volume") {
                volumes.push((guid.to_owned(), data.bytes.into_owned()));
            }
        }
        let target = target?;
        volumes
            .into_iter()
            .find(|(_, bytes)| *bytes == target)
            .map(|(guid, _)| guid)
    }

    /// `NoRecycleFiles` under the Explorer policies, either hive. A policy key that is absent
    /// sets nothing; one that exists and cannot be read is `None`.
    fn no_recycle_policy() -> Option<bool> {
        let mut set = false;
        for hive in [HKEY_LOCAL_MACHINE, HKEY_CURRENT_USER] {
            match RegKey::predef(hive).open_subkey(POLICIES) {
                Ok(key) => match key.get_value::<u32, _>("NoRecycleFiles") {
                    Ok(value) => set |= value != 0,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(_) => return None,
                },
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return None,
            }
        }
        Some(set)
    }

    /// The bytes the bin already holds from `drive`: every item whose original folder is on it.
    /// A folder's bytes are walked from the bin's own copy. `None` when any cannot be read.
    fn bytes_held(drive: char) -> Option<u64> {
        let mut held = 0_u64;
        for item in trash::os_limited::list().ok()? {
            let on_drive =
                matches!(place_of(&item.original_parent), Place::Drive(letter) if letter == drive);
            if !on_drive {
                continue;
            }
            let bytes = match trash::os_limited::metadata(&item).ok()?.size {
                trash::TrashItemSize::Bytes(bytes) => bytes,
                trash::TrashItemSize::Entries(_) => super::tree_bytes(Path::new(&item.id))?,
            };
            held = held.saturating_add(bytes);
        }
        Some(held)
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

    use super::{trash_refusal_for, BinFacts};
    use crate::protocol::TrashRefusalKind;

    const fn volume(network: bool, nuke: bool, left: u64) -> BinFacts {
        BinFacts::Volume {
            network,
            nuke_on_delete: nuke,
            capacity_left_bytes: left,
        }
    }

    #[test]
    fn every_fact_classifies_to_its_one_reason() {
        let cases = [
            (
                BinFacts::NoTrash,
                Some(10),
                Some(TrashRefusalKind::Unsupported),
            ),
            (BinFacts::NoLimits, Some(10), None),
            (
                BinFacts::Unreadable,
                Some(10),
                Some(TrashRefusalKind::CapacityUnknown),
            ),
            (
                volume(true, false, 100),
                Some(10),
                Some(TrashRefusalKind::NetworkDrive),
            ),
            (
                volume(false, true, 100),
                Some(10),
                Some(TrashRefusalKind::DisabledOnVolume),
            ),
            (
                volume(false, false, 100),
                None,
                Some(TrashRefusalKind::CapacityUnknown),
            ),
            (
                volume(false, false, 100),
                Some(101),
                Some(TrashRefusalKind::OversizedFolder),
            ),
            (volume(false, false, 100), Some(100), None),
        ];
        for (facts, bytes, expected) in cases {
            assert_eq!(trash_refusal_for(&facts, || bytes), expected, "{facts:?}");
        }
    }
}
