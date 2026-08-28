//! §4.3 — symlinks and junctions.
//!
//! Off by default. Enabled, three refusals stand between a link and the scan: it must land
//! inside an enabled root (the consent boundary), on a store a root already covers (the mount
//! boundary), and at a directory this run has not already reached (which is also what breaks a
//! cycle). A refusal is counted here and logged to stderr; it is **not** a `scan_problem` — the
//! eight groups of §11.1 are closed and none of them is links.

use std::collections::{BTreeSet, HashSet};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use crate::index::path::native_platform;
use crate::mount::MountResolver;
use crate::paths::{is_under, path_display, path_key};

/// Stable identity of a directory: `(device, inode)` on Unix, `(volume serial, file index)` on
/// Windows. Two paths that name one directory compare equal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FileId {
    pub volume: u64,
    pub index: u64,
}

#[cfg(unix)]
pub fn file_id(path: &Path) -> std::io::Result<FileId> {
    use std::os::unix::fs::MetadataExt as _;
    let meta = std::fs::metadata(path)?;
    Ok(FileId {
        volume: meta.dev(),
        index: meta.ino(),
    })
}

#[cfg(windows)]
pub fn file_id(path: &Path) -> std::io::Result<FileId> {
    use std::os::windows::fs::OpenOptionsExt as _;
    // Opening a *directory* on Windows requires backup semantics; `access_mode(0)` asks for
    // metadata only, which is all this reads. `std::os::windows::fs::MetadataExt` exposes the
    // same two numbers but only on nightly, behind `windows_by_handle`.
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    let file = std::fs::OpenOptions::new()
        .access_mode(0)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)?;
    let info = winapi_util::file::information(&file)?;
    Ok(FileId {
        volume: info.volume_serial_number(),
        index: info.file_index(),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkVerdict {
    Follow,
    /// The default policy. Not a refusal and not counted as one.
    NotFollowed,
    RefusedOutsideRoots,
    RefusedCrossStore,
    RefusedDuplicate,
    /// The target's identity or its store could not be established. Refusing is the honest
    /// answer: assuming it onto the root's store would put work on a queue the run never sized,
    /// and on a dead network share that queue blocks for an OS timeout.
    RefusedUnreadable,
}

impl LinkVerdict {
    #[must_use]
    pub const fn is_follow(self) -> bool {
        matches!(self, Self::Follow)
    }
}

/// Shared across the walk's worker threads, so every field is internally synchronised.
///
/// `Debug` is written by hand, not derived: `MountResolver` requires `Debug` but `Mutex<HashSet>`
/// would print a shelf's worth of inodes into a log line.
pub struct LinkPolicy {
    follow_links: bool,
    root_keys: Vec<Vec<u8>>,
    root_stores: BTreeSet<String>,
    mounts: Arc<dyn MountResolver>,
    seen: Mutex<HashSet<FileId>>,
    refused: AtomicU64,
}

impl std::fmt::Debug for LinkPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LinkPolicy")
            .field("follow_links", &self.follow_links)
            .field("roots", &self.root_keys.len())
            .field("refused", &self.refused())
            .finish_non_exhaustive()
    }
}

impl LinkPolicy {
    #[must_use]
    pub fn new(
        follow_links: bool,
        root_keys: Vec<Vec<u8>>,
        root_stores: BTreeSet<String>,
        mounts: Arc<dyn MountResolver>,
    ) -> Self {
        Self {
            follow_links,
            root_keys,
            root_stores,
            mounts,
            seen: Mutex::new(HashSet::new()),
            refused: AtomicU64::new(0),
        }
    }

    /// The three boundaries, in the order that spends least. Membership of an enabled root is a
    /// byte comparison; the store lookup may touch the filesystem; the identity read opens a
    /// handle.
    pub fn judge(&self, target: &Path) -> LinkVerdict {
        if !self.follow_links {
            return LinkVerdict::NotFollowed;
        }
        // R2: a link target is a path on the host's own filesystem, so the host's platform is
        // the right answer here — but it is stated, not inferred from `#[cfg]`.
        let key = path_key(target, native_platform());
        if !self.root_keys.iter().any(|root| is_under(&key, root)) {
            return self.refuse(target, LinkVerdict::RefusedOutsideRoots);
        }
        // R4: the resolver answers with `MountFacts`, not a bare string. An error is not a
        // cross-store verdict — it is "the store could not be established", which is what
        // `RefusedUnreadable` says.
        let Ok(facts) = self.mounts.resolve(target) else {
            return self.refuse(target, LinkVerdict::RefusedUnreadable);
        };
        if !self.root_stores.contains(&facts.store_key) {
            return self.refuse(target, LinkVerdict::RefusedCrossStore);
        }
        let Ok(id) = file_id(target) else {
            return self.refuse(target, LinkVerdict::RefusedUnreadable);
        };
        let Ok(mut seen) = self.seen.lock() else {
            return self.refuse(target, LinkVerdict::RefusedUnreadable);
        };
        if seen.insert(id) {
            LinkVerdict::Follow
        } else {
            drop(seen);
            self.refuse(target, LinkVerdict::RefusedDuplicate)
        }
    }

    #[must_use]
    pub fn refused(&self) -> u64 {
        self.refused.load(Ordering::Relaxed)
    }

    fn refuse(&self, target: &Path, verdict: LinkVerdict) -> LinkVerdict {
        self.refused.fetch_add(1, Ordering::Relaxed);
        // stderr is the only diagnostic channel; stdout carries protocol frames alone (§2.1).
        eprintln!(
            "scan: link refused ({verdict:?}) at {}",
            path_display(target)
        );
        verdict
    }
}
