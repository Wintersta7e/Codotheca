//! Why a suggested root arrives unticked.
//!
//! Two of the three reasons are properties of the path; the third is a property of a distro.
//! Cloud sync is detected from what the filesystem reports rather than from a list of product
//! names, because the cost the user is being warned about — a recall over a metered link — is
//! the same whichever product produced the placeholder, and a name list silently misses the
//! ones it has not heard of.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::mount::{MountResolver, StoreClass};
// R9: the distro facts struct is plan 18's. This module declares no `DistroInfo` of its own.
use crate::wsl::distros::DistroInfo;

/// What a suggested root is, for the purpose of arriving ticked or unticked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RootTrait {
    Ordinary,
    CloudSynced,
    SystemRoot,
}

/// The seam. Plan 06's `MountResolver` is behind the native implementation; a test supplies
/// `FixedClassifier` instead and never touches a real mount table.
pub trait RootClassifier: Send + Sync + std::fmt::Debug {
    fn classify(&self, path: &Path) -> RootTrait;
}

/// Directory prefixes that are the operating system's, not a person's work.
pub const SYSTEM_PREFIXES: [&str; 14] = [
    "/usr",
    "/etc",
    "/var",
    "/opt",
    "/bin",
    "/sbin",
    "/lib",
    "/boot",
    "/proc",
    "/sys",
    "/snap",
    "/nix",
    "windows",
    "program files",
];

/// True when the path is inside an operating-system directory.
///
/// Comparison is on whole segments, so `/home/u/variable` is not `/var`.
#[must_use]
pub fn is_system_path(path: &Path) -> bool {
    let text = path.to_string_lossy().replace('\\', "/").to_lowercase();
    let mut segments: Vec<&str> = text.split('/').filter(|s| !s.is_empty()).collect();
    // A drive letter is not a segment for this purpose.
    if segments.first().is_some_and(|s| s.ends_with(':')) {
        segments.remove(0);
    }
    let Some(first) = segments.first() else {
        return false;
    };
    SYSTEM_PREFIXES
        .iter()
        .any(|prefix| prefix.trim_start_matches('/').eq_ignore_ascii_case(first))
}

/// The classifier the running product uses.
#[derive(Debug)]
pub struct NativeRootClassifier {
    mounts: Arc<dyn MountResolver>,
}

impl NativeRootClassifier {
    #[must_use]
    pub fn new(mounts: Arc<dyn MountResolver>) -> NativeRootClassifier {
        NativeRootClassifier { mounts }
    }
}

impl RootClassifier for NativeRootClassifier {
    fn classify(&self, path: &Path) -> RootTrait {
        if is_system_path(path) {
            return RootTrait::SystemRoot;
        }
        if recalls_on_access(path) {
            return RootTrait::CloudSynced;
        }
        match self.mounts.resolve(path) {
            Ok(facts) if facts.class == StoreClass::Fuse => RootTrait::CloudSynced,
            _ => RootTrait::Ordinary,
        }
    }
}

/// True when the operating system will fetch this directory's contents from elsewhere on access.
#[cfg(windows)]
fn recalls_on_access(path: &Path) -> bool {
    use std::os::windows::fs::MetadataExt;
    // FILE_ATTRIBUTE_RECALL_ON_OPEN | FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS
    const RECALL: u32 = 0x0004_0000 | 0x0040_0000;
    std::fs::metadata(path).is_ok_and(|m| m.file_attributes() & RECALL != 0)
}

#[cfg(not(windows))]
fn recalls_on_access(_path: &Path) -> bool {
    false
}

/// A classifier whose answers are supplied. Test seam.
#[derive(Debug)]
pub struct FixedClassifier {
    map: Vec<(PathBuf, RootTrait)>,
}

impl FixedClassifier {
    #[must_use]
    pub fn new(mut map: Vec<(PathBuf, RootTrait)>) -> FixedClassifier {
        map.sort_by_key(|(p, _)| std::cmp::Reverse(p.components().count()));
        FixedClassifier { map }
    }
}

impl RootClassifier for FixedClassifier {
    fn classify(&self, path: &Path) -> RootTrait {
        for (prefix, verdict) in &self.map {
            if path.starts_with(prefix) {
                return *verdict;
            }
        }
        RootTrait::Ordinary
    }
}

/// **R9.** The distro facts struct is plan 18's `crate::wsl::distros::DistroInfo { name, state }`
/// and is consumed, never redeclared: plan 18 owns §13, and `DistroState` carries strictly more
/// than the `running: bool` this module used to fold into it.
///
/// The display string is derived here instead of living inside those facts. §8.5 makes a WSL
/// location's `path_display` the **Linux** form — the distro travels in `RootSuggestion.distro`,
/// never inside the path, which is exactly the reading §4bis.4 forbids — and §13 forbids starting
/// a stopped distro, so no username can be probed. The row names the home parent.
pub const DISTRO_HOME_DISPLAY: &str = "/home";

/// The seam plan 18 fills. §13: starting a stopped distro is an explicit, consented action and
/// never automatic during first run, so the probe reports and never starts.
pub trait DistroProbe: Send + Sync + std::fmt::Debug {
    fn distros(&self) -> Vec<DistroInfo>;
}

/// The probe used until plan 18 supplies one: no distros, and therefore no distro rows.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoDistros;

impl DistroProbe for NoDistros {
    fn distros(&self) -> Vec<DistroInfo> {
        Vec::new()
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
    use super::*;
    use crate::mount::{MountError, MountFacts, MountResolver, StoreClass};
    use crate::wsl::distros::{DistroInfo, DistroState}; // R9: plan 18 owns the facts struct.
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    #[derive(Debug)]
    struct OneClass(StoreClass);
    impl MountResolver for OneClass {
        fn resolve(&self, _path: &Path) -> Result<MountFacts, MountError> {
            Ok(MountFacts {
                store_key: "s".to_owned(),
                volume_key: Some("v".to_owned()),
                class: self.0,
            })
        }
        fn is_volume_mounted(&self, _volume_key: &str) -> bool {
            true
        }
    }

    // §10.1a: cloud-sync roots are shown unchecked because scanning them can silently cost
    // someone gigabytes of metered bandwidth. A userspace filesystem is that signal on Linux.
    #[test]
    fn a_userspace_filesystem_reads_as_cloud_synced() {
        let c = NativeRootClassifier::new(Arc::new(OneClass(StoreClass::Fuse)));
        assert_eq!(
            c.classify(Path::new("/home/u/synced")),
            RootTrait::CloudSynced
        );
    }

    #[test]
    fn an_ordinary_local_store_is_ordinary() {
        let c = NativeRootClassifier::new(Arc::new(OneClass(StoreClass::Local)));
        assert_eq!(c.classify(Path::new("/home/u/dev")), RootTrait::Ordinary);
    }

    // §10.1a: system roots are shown unchecked and labelled. A system path outranks the store
    // class, because a system directory on a local disk is still not a project folder.
    #[test]
    fn a_system_path_outranks_the_store_class() {
        let c = NativeRootClassifier::new(Arc::new(OneClass(StoreClass::Local)));
        assert_eq!(c.classify(Path::new("/usr/share/x")), RootTrait::SystemRoot);
        assert_eq!(c.classify(Path::new("/var/lib/y")), RootTrait::SystemRoot);
        assert!(is_system_path(Path::new("C:\\Windows\\System32")));
        assert!(is_system_path(Path::new("C:/Program Files/Thing")));
        assert!(!is_system_path(Path::new("/home/u/dev")));
        assert!(!is_system_path(Path::new("/home/u/variable")));
    }

    #[test]
    fn the_default_distro_probe_finds_none_so_no_distro_row_is_invented() {
        assert!(NoDistros.distros().is_empty());
    }

    // R9: the display string is derived here rather than carried in plan 18's facts struct.
    // §8.5 makes a WSL location's `path_display` the *Linux* form — the distro travels in
    // `RootSuggestion.distro`, never inside the path — and §13 forbids starting a stopped distro,
    // so no username can be probed. The row names the home parent and nothing deeper.
    #[test]
    fn the_distro_home_display_is_the_linux_form_and_needs_no_probe() {
        let _stopped = DistroInfo {
            name: "alpha".to_owned(),
            state: DistroState::Stopped,
        };
        assert_eq!(DISTRO_HOME_DISPLAY, "/home");
        assert!(!DISTRO_HOME_DISPLAY.contains("wsl.localhost"));
    }

    #[test]
    fn the_fixed_classifier_answers_by_longest_matching_prefix() {
        let c = FixedClassifier::new(vec![
            (PathBuf::from("/a"), RootTrait::CloudSynced),
            (PathBuf::from("/a/b"), RootTrait::Ordinary),
        ]);
        assert_eq!(c.classify(Path::new("/a/x")), RootTrait::CloudSynced);
        assert_eq!(c.classify(Path::new("/a/b/x")), RootTrait::Ordinary);
        assert_eq!(c.classify(Path::new("/z")), RootTrait::Ordinary);
    }
}
