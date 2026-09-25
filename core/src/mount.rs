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

/// The production resolver.
#[derive(Debug, Default)]
pub struct SystemMountResolver;

impl SystemMountResolver {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

/// One row of `/proc/self/mountinfo`, reduced to what §4.7 needs.
///
/// `cfg(unix)` because the only caller is the unix resolver below. Ungated, Windows compiles it,
/// never constructs it, and `-D warnings` fails the build — which is how CI's first Windows
/// clippy run found it.
#[cfg(unix)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MountEntry {
    pub(crate) mount_point: std::path::PathBuf,
    pub(crate) major_minor: String,
    pub(crate) fs_type: String,
    pub(crate) source: String,
}

/// Longest mount point covering `path`. Malformed lines are skipped, never fatal — this file
/// is read on every scan and one unexpected row must not blind the scheduler.
#[cfg(unix)]
pub(crate) fn parse_mountinfo(contents: &str, path: &Path) -> Option<MountEntry> {
    let mut best: Option<MountEntry> = None;
    for line in contents.lines() {
        let Some((left, right)) = line.split_once(" - ") else {
            continue;
        };
        let mut left_fields = left.split_whitespace();
        let major_minor = left_fields.nth(2)?.to_owned();
        let mount_point = left_fields.nth(1)?;
        let mut right_fields = right.split_whitespace();
        let Some(fs_type) = right_fields.next() else {
            continue;
        };
        let Some(source) = right_fields.next() else {
            continue;
        };
        let mount_point = std::path::PathBuf::from(mount_point);
        if !path.starts_with(&mount_point) {
            continue;
        }
        let deeper = match best.as_ref() {
            None => true,
            Some(b) => mount_point.components().count() > b.mount_point.components().count(),
        };
        if deeper {
            best = Some(MountEntry {
                mount_point,
                major_minor,
                fs_type: fs_type.to_owned(),
                source: source.to_owned(),
            });
        }
    }
    best
}

/// Class from the filesystem type alone. `Unknown` means "ask the block device next".
#[cfg(unix)]
pub(crate) fn class_for_fs_type(fs_type: &str) -> StoreClass {
    if fs_type.starts_with("fuse") {
        return StoreClass::Fuse;
    }
    let network = ["nfs", "cifs", "smb", "9p", "afs", "sshfs", "davfs"];
    if network.iter().any(|n| fs_type.starts_with(n)) {
        return StoreClass::Network;
    }
    StoreClass::Unknown
}

#[cfg(unix)]
impl MountResolver for SystemMountResolver {
    fn resolve(&self, path: &Path) -> Result<MountFacts, MountError> {
        let contents = std::fs::read_to_string("/proc/self/mountinfo")
            .map_err(|e| MountError::Io(e.to_string()))?;
        let entry = parse_mountinfo(&contents, path)
            .ok_or_else(|| MountError::Unsupported(path.display().to_string()))?;
        let mut class = class_for_fs_type(&entry.fs_type);
        if class == StoreClass::Unknown {
            class = block_device_class(&entry.source);
        }
        Ok(MountFacts {
            store_key: format!("dev:{}", entry.major_minor),
            volume_key: volume_uuid(&entry.source),
            class,
        })
    }

    fn is_volume_mounted(&self, volume_key: &str) -> bool {
        std::fs::read_to_string("/proc/self/mountinfo").is_ok_and(|contents| {
            contents
                .lines()
                .filter_map(|l| l.split_once(" - "))
                .any(|(_, right)| {
                    right
                        .split_whitespace()
                        .nth(1)
                        .and_then(volume_uuid)
                        .is_some_and(|uuid| uuid == volume_key)
                })
        })
    }
}

/// `/dev/disk/by-uuid/<uuid>` is a symlink to the device. Reverse it by scanning, which is a
/// directory of a few entries and needs no privileges.
#[cfg(unix)]
fn volume_uuid(source: &str) -> Option<String> {
    let canonical = std::fs::canonicalize(source).ok()?;
    let dir = std::fs::read_dir("/dev/disk/by-uuid").ok()?;
    for entry in dir.flatten() {
        if std::fs::canonicalize(entry.path()).ok().as_deref() == Some(canonical.as_path()) {
            return entry.file_name().into_string().ok();
        }
    }
    None
}

#[cfg(unix)]
fn block_device_class(source: &str) -> StoreClass {
    let Some(name) = Path::new(source).file_name().and_then(|n| n.to_str()) else {
        return StoreClass::Unknown;
    };
    // /sys/block holds whole disks; strip a trailing partition number to find the parent.
    let base: String = name
        .trim_end_matches(|c: char| c.is_ascii_digit())
        .to_owned();
    let read = |leaf: &str| std::fs::read_to_string(format!("/sys/block/{base}/{leaf}"));
    if read("removable").is_ok_and(|v| v.trim() == "1") {
        return StoreClass::Removable;
    }
    if read("queue/rotational").is_ok_and(|v| v.trim() == "1") {
        return StoreClass::Hdd;
    }
    StoreClass::Local
}

#[cfg(windows)]
impl MountResolver for SystemMountResolver {
    fn resolve(&self, path: &Path) -> Result<MountFacts, MountError> {
        use std::path::{Component, Prefix};
        let absolute = std::fs::canonicalize(path).map_err(|e| MountError::Io(e.to_string()))?;
        let Some(Component::Prefix(prefix)) = absolute.components().next() else {
            return Err(MountError::Unsupported(path.display().to_string()));
        };
        let key = prefix.as_os_str().to_string_lossy().to_string();
        // No volume serial without a Win32 call, and the crate forbids `unsafe`. §4.7 already
        // permits a stable fallback; the root component is that fallback.
        let class = match prefix.kind() {
            Prefix::UNC(..) | Prefix::VerbatimUNC(..) => StoreClass::Network,
            _ => StoreClass::Unknown,
        };
        Ok(MountFacts {
            store_key: key.clone(),
            volume_key: Some(key),
            class,
        })
    }

    fn is_volume_mounted(&self, volume_key: &str) -> bool {
        std::path::Path::new(&format!("{volume_key}\\")).exists()
    }
}

#[cfg(all(test, unix))]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use super::{class_for_fs_type, parse_mountinfo, StoreClass};
    use std::path::Path;

    const SAMPLE: &str = "\
21 27 0:20 / /proc rw,nosuid shared:5 - proc proc rw
25 27 8:2 / / rw,relatime shared:1 - ext4 /dev/sda2 rw
33 25 8:17 / /media/stick rw,nosuid shared:9 - vfat /dev/sdb1 rw
41 25 0:44 / /net/share rw shared:11 - cifs //host/share rw
";

    #[test]
    fn the_longest_mount_point_wins_not_the_first_line() {
        let entry = parse_mountinfo(SAMPLE, Path::new("/media/stick/repo")).unwrap();
        assert_eq!(entry.mount_point, Path::new("/media/stick"));
        assert_eq!(entry.major_minor, "8:17");
        assert_eq!(entry.fs_type, "vfat");
        assert_eq!(entry.source, "/dev/sdb1");
    }

    #[test]
    fn a_path_under_no_special_mount_falls_back_to_the_root_entry() {
        let entry = parse_mountinfo(SAMPLE, Path::new("/srv/work/repo")).unwrap();
        assert_eq!(entry.mount_point, Path::new("/"));
        assert_eq!(entry.fs_type, "ext4");
    }

    #[test]
    fn network_and_fuse_filesystems_are_classified_as_slow() {
        assert_eq!(class_for_fs_type("cifs"), StoreClass::Network);
        assert_eq!(class_for_fs_type("nfs4"), StoreClass::Network);
        assert_eq!(class_for_fs_type("9p"), StoreClass::Network);
        assert_eq!(class_for_fs_type("fuse.sshfs"), StoreClass::Fuse);
        assert_eq!(class_for_fs_type("ext4"), StoreClass::Unknown);
    }

    #[test]
    fn a_malformed_line_is_skipped_rather_than_aborting_the_parse() {
        let broken = format!("nonsense without a separator\n{SAMPLE}");
        let entry = parse_mountinfo(&broken, Path::new("/net/share/x")).unwrap();
        assert_eq!(entry.fs_type, "cifs");
    }
}
