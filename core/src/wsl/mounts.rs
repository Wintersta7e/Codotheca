//! §4.5 and §4.7 — what a path inside a distro is stored on.
//!
//! A distro is not a storage device. Its VHD, the Windows drives it surfaces, and anything it
//! mounts over the network have different backing stores and different cost profiles, and the
//! only honest way to tell them apart is the filesystem type reported by the kernel. Matching
//! `/mnt/[a-z]` by name gets this wrong in both directions: `/mnt/data` may be ext4 on the VHD,
//! and a 9p mount may sit anywhere at all.
//!
//! `crate::mount` has a parser and a classifier of the same shape and they are **not** duplicates
//! of these. That one answers for the *host*: it reads one entry covering one path, returns
//! `Unknown` for a local filesystem so the caller can go on to read `/sys/block`, and keys the
//! store by the device's major:minor. This one answers for a *distro*, where the device node is
//! reassigned on every restart, the whole table is needed at once to decide what to walk, and
//! §13 says the class comes from the type alone.

use crate::mount::{MountError, MountFacts, MountResolver, StoreClass};
use crate::scan::wsl::is_drvfs_fstype;
use crate::wsl::path::{parse_drvfs_source, DrvfsMount};

pub const MOUNTINFO_PATH: &str = "/proc/self/mountinfo";

/// One line of `/proc/self/mountinfo`, reduced to the four fields that matter here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MountEntry {
    pub mount_id: i64,
    pub mount_point: String,
    pub fstype: String,
    pub source: String,
}

/// `mountinfo` escapes space, tab, newline and backslash as three-digit octal. Anything that is
/// not a complete escape is left exactly as it was found.
///
/// Decoding happens on bytes, not on `char`s: a mount point may hold any non-separator byte, and
/// widening each byte to a `char` would turn one UTF-8 sequence into two Latin-1 characters.
#[must_use]
pub fn unescape_mountinfo(field: &str) -> String {
    let bytes = field.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0usize;
    while i < bytes.len() {
        let Some(&b) = bytes.get(i) else { break };
        if b == b'\\' && i + 3 < bytes.len() {
            let digits = field.get(i + 1..i + 4).unwrap_or_default();
            if digits.len() == 3 && digits.bytes().all(|d| matches!(d, b'0'..=b'7')) {
                if let Ok(v) = u8::from_str_radix(digits, 8) {
                    out.push(v);
                    i += 4;
                    continue;
                }
            }
        }
        out.push(b);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Every mount the kernel reports, in the order it reports them. A malformed line is skipped
/// rather than failing the whole read: an unreadable mount must not cost the distro its scan.
#[must_use]
pub fn parse_mountinfo(text: &str) -> Vec<MountEntry> {
    let mut out = Vec::new();
    for line in text.lines() {
        let fields: Vec<&str> = line.split(' ').filter(|f| !f.is_empty()).collect();
        let Some(sep) = fields.iter().position(|f| *f == "-") else {
            continue;
        };
        let (Some(id), Some(point), Some(fstype), Some(source)) = (
            fields.first(),
            fields.get(4),
            fields.get(sep + 1),
            fields.get(sep + 2),
        ) else {
            continue;
        };
        let Ok(mount_id) = id.parse::<i64>() else {
            continue;
        };
        out.push(MountEntry {
            mount_id,
            mount_point: unescape_mountinfo(point),
            fstype: unescape_mountinfo(fstype),
            source: unescape_mountinfo(source),
        });
    }
    out
}

/// Whether the in-distro walk may descend into a path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MountVerdict {
    Walk,
    /// The bytes live on a Windows volume. The native walk already covers them, and reaching
    /// them from inside the distro is §4.5's slow path taken in the other direction.
    SkipWindowsBacked {
        mount_point: String,
        fstype: String,
    },
}

#[derive(Debug, Clone, Default)]
pub struct MountTable {
    entries: Vec<MountEntry>,
}

impl MountTable {
    #[must_use]
    pub fn from_mountinfo(text: &str) -> Self {
        Self {
            entries: parse_mountinfo(text),
        }
    }

    /// The live mount table. An unreadable `mountinfo` yields an empty table, whose facts are
    /// `Unknown` — never a fabricated `Local`.
    #[must_use]
    pub fn read() -> Self {
        match std::fs::read_to_string(MOUNTINFO_PATH) {
            Ok(text) => Self::from_mountinfo(&text),
            Err(_) => Self::default(),
        }
    }

    #[must_use]
    pub fn entries(&self) -> &[MountEntry] {
        &self.entries
    }

    /// The mount that covers `path`: the longest mount point that is a prefix of it at a
    /// separator, and among equals the one mounted last, because that is the one on top.
    #[must_use]
    pub fn resolve(&self, path: &str) -> Option<&MountEntry> {
        let mut best: Option<(usize, &MountEntry)> = None;
        for entry in &self.entries {
            if !covers(&entry.mount_point, path) {
                continue;
            }
            let len = entry.mount_point.len();
            match best {
                Some((best_len, _)) if best_len > len => {}
                _ => best = Some((len, entry)),
            }
        }
        best.map(|(_, e)| e)
    }

    #[must_use]
    pub fn facts_for(&self, distro: &str, path: &str) -> MountFacts {
        match self.resolve(path) {
            Some(entry) => MountFacts {
                store_key: store_key_for(distro, entry),
                volume_key: volume_key_for(distro, entry),
                class: class_for_fstype(&entry.fstype),
            },
            None => MountFacts {
                store_key: format!("wsl:{distro}:?"),
                volume_key: None,
                class: StoreClass::Unknown,
            },
        }
    }

    #[must_use]
    pub fn drvfs_at(&self, path: &str) -> Option<DrvfsMount> {
        let entry = self.resolve(path)?;
        if !is_drvfs_fstype(&entry.fstype) {
            return None;
        }
        Some(DrvfsMount {
            mount_point: entry.mount_point.clone(),
            windows_root: parse_drvfs_source(&entry.source)?,
        })
    }

    #[must_use]
    pub fn verdict_for(&self, path: &str) -> MountVerdict {
        match self.resolve(path) {
            Some(entry) if is_drvfs_fstype(&entry.fstype) => MountVerdict::SkipWindowsBacked {
                mount_point: entry.mount_point.clone(),
                fstype: entry.fstype.clone(),
            },
            _ => MountVerdict::Walk,
        }
    }
}

fn covers(mount_point: &str, path: &str) -> bool {
    if mount_point == "/" {
        return path.starts_with('/');
    }
    let base = mount_point.trim_end_matches('/');
    match path.strip_prefix(base) {
        Some("") => true,
        Some(rest) => rest.starts_with('/'),
        None => false,
    }
}

/// §4.7's runtime identity, scoped by distro because two distros' roots are two devices.
#[must_use]
pub fn store_key_for(distro: &str, entry: &MountEntry) -> String {
    format!("wsl:{distro}:{}", entry.mount_point)
}

/// §4.7's persistent identity, best effort. A device node is reassigned on every restart, so the
/// mount point is the stable half and the distro name in front of it is what actually persists.
/// A filesystem with no persistent identity at all returns `None`, not a fabricated key.
#[must_use]
pub fn volume_key_for(distro: &str, entry: &MountEntry) -> Option<String> {
    if is_drvfs_fstype(&entry.fstype) || !entry.source.starts_with("/dev/") {
        return None;
    }
    Some(format!("wsl-distro:{distro}:{}", entry.mount_point))
}

/// §13: the class comes from the filesystem type, never from the path.
#[must_use]
pub fn class_for_fstype(fstype: &str) -> StoreClass {
    let f = fstype.to_ascii_lowercase();
    if is_drvfs_fstype(&f) {
        // Not a wire, but the same cost shape: the bytes are reached across a virtual-machine
        // transport, one round trip per syscall. That is what the per-store cap of 1 is for.
        return StoreClass::Network;
    }
    if f.starts_with("fuse") {
        return StoreClass::Fuse;
    }
    match f.as_str() {
        "ext2" | "ext3" | "ext4" | "btrfs" | "xfs" | "f2fs" | "zfs" | "overlay" | "tmpfs"
        | "ramfs" | "devtmpfs" => StoreClass::Local,
        "nfs" | "nfs4" | "cifs" | "smb3" | "afs" | "ceph" | "virtiofs" => StoreClass::Network,
        // Removable's cap of 1 is the conservative reading, and these are the filesystems a
        // detachable volume actually arrives with.
        "vfat" | "exfat" | "ntfs" | "ntfs3" | "iso9660" | "udf" | "squashfs" | "erofs" => {
            StoreClass::Removable
        }
        _ => StoreClass::Unknown,
    }
}

/// §4.7's seam, answered from inside the distro. The Windows implementation cannot answer for a
/// distro at all: it sees one 9p share where the distro sees a VHD, several Windows drives and
/// whatever else was mounted.
#[derive(Debug)]
pub struct DistroMountResolver {
    distro: String,
    table: MountTable,
}

impl DistroMountResolver {
    #[must_use]
    pub fn new(distro: String, table: MountTable) -> Self {
        Self { distro, table }
    }
}

impl MountResolver for DistroMountResolver {
    fn resolve(&self, path: &std::path::Path) -> Result<MountFacts, MountError> {
        Ok(self.table.facts_for(&self.distro, &path.to_string_lossy()))
    }

    fn is_volume_mounted(&self, volume_key: &str) -> bool {
        self.table
            .entries()
            .iter()
            .any(|e| volume_key_for(&self.distro, e).as_deref() == Some(volume_key))
    }
}

#[must_use]
pub fn class_slug(class: StoreClass) -> &'static str {
    match class {
        StoreClass::Local => "local",
        StoreClass::Removable => "removable",
        StoreClass::Network => "network",
        StoreClass::Fuse => "fuse",
        StoreClass::Hdd => "hdd",
        StoreClass::Unknown => "unknown",
    }
}

#[must_use]
pub fn class_from_slug(slug: &str) -> StoreClass {
    match slug {
        "local" => StoreClass::Local,
        "removable" => StoreClass::Removable,
        "network" => StoreClass::Network,
        "fuse" => StoreClass::Fuse,
        "hdd" => StoreClass::Hdd,
        _ => StoreClass::Unknown,
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
mod tests {
    use super::{
        class_for_fstype, class_from_slug, class_slug, parse_mountinfo, store_key_for,
        unescape_mountinfo, volume_key_for, MountTable, MountVerdict,
    };
    use crate::mount::StoreClass;

    // Shape taken from proc(5): id, parent, dev, root, mount point, options, optional fields,
    // "-", fstype, source, super options.
    const FIXTURE: &str = "\
23 28 0:22 / /proc rw,nosuid - proc proc rw
26 28 0:5 / /dev rw,nosuid - devtmpfs none rw
28 1 8:32 / / rw,relatime - ext4 /dev/sdc rw,discard
31 28 0:31 / /mnt/wsl rw,relatime - tmpfs none rw
64 28 0:64 / /mnt/c rw,noatime - 9p C:\\134 rw,dirsync
70 28 0:70 / /mnt/backup rw,relatime - nfs4 fileserver:/vol rw
77 28 0:77 / /mnt/my\\040drive rw - exfat /dev/sdd1 rw
";

    fn table() -> MountTable {
        MountTable::from_mountinfo(FIXTURE)
    }

    #[test]
    fn mountinfo_octal_escapes_are_decoded() {
        assert_eq!(unescape_mountinfo(r"/mnt/my\040drive"), "/mnt/my drive");
        assert_eq!(unescape_mountinfo(r"C:\134"), "C:\\");
        assert_eq!(unescape_mountinfo("/plain"), "/plain");
        assert_eq!(unescape_mountinfo(r"trailing\04"), r"trailing\04");
        // A byte-at-a-time decoder that widened each byte to a `char` would return "/mnt/donnÃ©es".
        assert_eq!(unescape_mountinfo(r"/mnt/donn\303\251es"), "/mnt/données");
    }

    #[test]
    fn every_line_yields_a_mount_and_the_fields_after_the_separator_are_read() {
        let entries = parse_mountinfo(FIXTURE);
        assert_eq!(entries.len(), 7);
        let c = entries
            .iter()
            .find(|e| e.mount_point == "/mnt/c")
            .expect("has /mnt/c");
        assert_eq!(c.fstype, "9p");
        assert_eq!(c.source, "C:\\");
    }

    #[test]
    fn the_longest_matching_mount_point_wins() {
        let t = table();
        assert_eq!(
            t.resolve("/home/u/p").map(|e| e.mount_point.as_str()),
            Some("/")
        );
        assert_eq!(
            t.resolve("/mnt/c/code").map(|e| e.mount_point.as_str()),
            Some("/mnt/c")
        );
        assert_eq!(
            t.resolve("/mnt/wsl/x").map(|e| e.mount_point.as_str()),
            Some("/mnt/wsl")
        );
    }

    #[test]
    fn the_class_comes_from_the_filesystem_type_not_from_the_path() {
        // §4.5: `/mnt/data` on ext4 is not a bridge, and a 9p mount at `/opt` is.
        assert_eq!(class_for_fstype("ext4"), StoreClass::Local);
        assert_eq!(class_for_fstype("9p"), StoreClass::Network);
        assert_eq!(class_for_fstype("drvfs"), StoreClass::Network);
        assert_eq!(class_for_fstype("nfs4"), StoreClass::Network);
        assert_eq!(class_for_fstype("exfat"), StoreClass::Removable);
        assert_eq!(class_for_fstype("fuse.sshfs"), StoreClass::Fuse);
        assert_eq!(class_for_fstype("wibble"), StoreClass::Unknown);
    }

    #[test]
    fn two_distros_do_not_share_a_store_key() {
        let entries = parse_mountinfo(FIXTURE);
        let root = entries
            .iter()
            .find(|e| e.mount_point == "/")
            .expect("has /");
        assert_eq!(store_key_for("alpha", root), "wsl:alpha:/");
        assert_ne!(store_key_for("alpha", root), store_key_for("beta", root));
    }

    #[test]
    fn only_a_device_backed_filesystem_claims_a_volume_key() {
        let entries = parse_mountinfo(FIXTURE);
        let root = entries
            .iter()
            .find(|e| e.mount_point == "/")
            .expect("has /");
        let drv = entries
            .iter()
            .find(|e| e.mount_point == "/mnt/c")
            .expect("has /mnt/c");
        let tmp = entries
            .iter()
            .find(|e| e.mount_point == "/mnt/wsl")
            .expect("has /mnt/wsl");
        assert_eq!(
            volume_key_for("alpha", root).as_deref(),
            Some("wsl-distro:alpha:/")
        );
        assert_eq!(volume_key_for("alpha", drv), None);
        assert_eq!(volume_key_for("alpha", tmp), None);
    }

    #[test]
    fn a_windows_backed_mount_is_refused_by_type_and_names_what_it_refused() {
        let t = table();
        assert_eq!(t.verdict_for("/home/u/code"), MountVerdict::Walk);
        assert_eq!(t.verdict_for("/mnt/backup/code"), MountVerdict::Walk);
        assert_eq!(
            t.verdict_for("/mnt/c/code"),
            MountVerdict::SkipWindowsBacked {
                mount_point: "/mnt/c".to_owned(),
                fstype: "9p".to_owned(),
            }
        );
    }

    #[test]
    fn a_drvfs_mount_reports_the_windows_root_it_exposes() {
        let m = table().drvfs_at("/mnt/c/code").expect("is drvfs");
        assert_eq!(m.mount_point, "/mnt/c");
        assert_eq!(m.windows_root, "C:\\");
        assert!(table().drvfs_at("/home/u").is_none());
    }

    #[test]
    fn facts_carry_the_key_the_class_and_the_volume_together() {
        let f = table().facts_for("alpha", "/home/u/code");
        assert_eq!(f.store_key, "wsl:alpha:/");
        assert_eq!(f.class, StoreClass::Local);
        assert_eq!(f.volume_key.as_deref(), Some("wsl-distro:alpha:/"));
    }

    #[test]
    fn a_path_no_mount_covers_is_unknown_rather_than_local() {
        // Never render unknown as something else: an empty table means "not determined".
        let f = MountTable::from_mountinfo("").facts_for("alpha", "/home/u");
        assert_eq!(f.class, StoreClass::Unknown);
        assert_eq!(f.store_key, "wsl:alpha:?");
        assert_eq!(f.volume_key, None);
    }

    #[test]
    fn the_resolver_answers_from_the_table_and_reports_what_is_mounted() {
        use crate::mount::MountResolver;
        use std::path::Path;

        let resolver = super::DistroMountResolver::new("alpha".to_owned(), table());
        let facts = resolver
            .resolve(Path::new("/home/me/widget"))
            .expect("resolves");
        assert_eq!(facts.store_key, "wsl:alpha:/");
        assert!(resolver.is_volume_mounted("wsl-distro:alpha:/"));
        assert!(!resolver.is_volume_mounted("wsl-distro:beta:/"));
    }

    #[test]
    fn the_class_slug_round_trips() {
        for c in [
            StoreClass::Local,
            StoreClass::Removable,
            StoreClass::Network,
            StoreClass::Fuse,
            StoreClass::Hdd,
            StoreClass::Unknown,
        ] {
            assert_eq!(class_from_slug(class_slug(c)), c);
        }
        assert_eq!(class_from_slug("nonsense"), StoreClass::Unknown);
    }
}
