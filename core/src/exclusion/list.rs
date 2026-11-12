//! The one exclusion list (§56.4): §4.3's built-in thirty and the user's names, matched as the
//! walk always matched them, followed by the user's path entries, matched by `path_key`.
//!
//! A list is immutable. Changing it means building a new one and swapping it into the
//! [`ExclusionGate`](super::gate::ExclusionGate), so every question asked of one list gets one
//! answer however long the asker holds it.

use std::path::Path;

use crate::index::path::{PathPlatform, StoredPath};
use crate::paths::{is_under, path_from_bytes, path_key};
use crate::protocol::LocationKind;
use crate::scan::run::platform_of;
use crate::scan::skiplist::{SkipList, SKIP_LIST};

/// One user path entry, compared by key and never opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathEntry {
    /// The WSL distro the path lives in; empty for a path on the host itself.
    pub distro: String,
    /// The rules `match_key` was folded by, which every key compared with it must share.
    pub platform: PathPlatform,
    /// `paths::path_key` of the entry's bytes (GA2).
    pub match_key: Vec<u8>,
}

/// What coverage of one stored `location` is decided from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocationKey {
    /// The side the copy lives on, which decides how its paths fold.
    pub kind: LocationKind,
    /// The copy's distro; empty unless `kind` is WSL.
    pub distro: String,
    /// The copy's `location.path_bytes`.
    pub path_bytes: Vec<u8>,
    /// The copy's `location.path_key`, folded by its kind's platform.
    pub path_key: Vec<u8>,
    /// The bytes of the innermost scan root containing the copy, enabled or not. `None` when no
    /// root contains it: a name then has nothing to lie below, and only path entries apply.
    pub root_path_bytes: Option<Vec<u8>>,
}

impl LocationKey {
    /// The key of `dir`, a directory inside this copy (a nested repository): the same side,
    /// distro and root, so a name or path entry covering `dir` covers it whether or not it
    /// covers the copy.
    #[must_use]
    pub fn at(&self, dir: &Path) -> Self {
        let path_bytes = crate::paths::path_bytes(dir);
        let path_key = StoredPath::from_bytes(path_bytes.clone(), platform_of(self.kind.as_str()))
            .key()
            .to_vec();
        Self {
            kind: self.kind,
            distro: self.distro.clone(),
            path_bytes,
            path_key,
            root_path_bytes: self.root_path_bytes.clone(),
        }
    }
}

/// The built-ins, the user's names, and the user's paths, in the order they were added.
#[derive(Debug, Clone)]
pub struct ExclusionList {
    names: SkipList,
    paths: Vec<PathEntry>,
}

impl Default for ExclusionList {
    /// The built-ins alone: what a core with no user entry applies.
    fn default() -> Self {
        Self::new(&[], Vec::new())
    }
}

impl ExclusionList {
    /// The built-ins followed by `user_names`, and `paths`, each in the order given.
    #[must_use]
    pub fn new(user_names: &[String], paths: Vec<PathEntry>) -> Self {
        Self {
            names: SkipList::with_user_entries(user_names),
            paths,
        }
    }

    /// The name half — built-ins and user names — as the walk and the root estimate apply it.
    #[must_use]
    pub const fn names(&self) -> &SkipList {
        &self.names
    }

    /// The user's names alone, verbatim and in the order added, without the built-ins every
    /// build already holds.
    #[must_use]
    pub fn user_names(&self) -> &[String] {
        self.names
            .entries()
            .get(SKIP_LIST.len()..)
            .unwrap_or_default()
    }

    /// The user's path entries, in the order added.
    #[must_use]
    pub fn paths(&self) -> &[PathEntry] {
        &self.paths
    }

    /// True when a path entry of `distro` holds `dir`, whose key folds by `platform`: the path
    /// half of [`covers_dir`](Self::covers_dir), for a walk that already tests every directory
    /// it enters and so needs no ancestor of it tested again. Keys `dir` only when such an entry
    /// exists.
    #[must_use]
    pub fn covers_path(&self, dir: &Path, distro: &str, platform: PathPlatform) -> bool {
        let mut entries = self.paths.iter().filter(|e| e.distro == distro).peekable();
        if entries.peek().is_none() {
            return false;
        }
        let key = path_key(dir, platform);
        entries.any(|e| is_under(&key, &e.match_key))
    }

    /// True when the walk must not read `dir`, which lies at or below `root` in `distro`, whose
    /// paths fold by `platform`.
    ///
    /// `root` itself is never covered (§4.3). A path entry is keyed only when one exists for
    /// `distro`, so a list without one costs the walk nothing it did not already pay.
    #[must_use]
    pub fn covers_dir(
        &self,
        dir: &Path,
        root: &Path,
        distro: &str,
        platform: PathPlatform,
    ) -> bool {
        if dir == root {
            return false;
        }
        self.names.covers(dir, root) || self.covers_path(dir, distro, platform)
    }

    /// True when the stored copy `key` describes is covered: a path entry of its distro holds
    /// it, or a name matches a directory strictly below its root down to the copy itself.
    #[must_use]
    pub fn covers_location(&self, key: &LocationKey) -> bool {
        self.covers_key(key, |dir| self.names.skips(dir))
    }

    /// [`covers_location`](Self::covers_location) with the user's entries alone: whether a
    /// repository nested inside a copy is covered. A built-in name there is a walk-cost skip,
    /// and the pre-removal read must still see what a vendored submodule holds (GA38).
    #[must_use]
    pub fn user_covers_location(&self, key: &LocationKey) -> bool {
        self.covers_key(key, |dir| self.names.user_skips(dir))
    }

    /// A path entry holds `key`, or `named` holds a directory strictly below its root down to
    /// the copy itself.
    fn covers_key(&self, key: &LocationKey, named: impl Fn(&Path) -> bool) -> bool {
        if self
            .paths
            .iter()
            .any(|e| e.distro == key.distro && is_under(&key.path_key, &e.match_key))
        {
            return true;
        }
        let Some(root) = &key.root_path_bytes else {
            return false;
        };
        // How many of the copy's own path components lie below its root, counted on the keys so
        // that a root spelled in another case still bounds the walk up (GA36).
        let root_key = StoredPath::from_bytes(root.clone(), platform_of(key.kind.as_str()));
        let below = components(&key.path_key).saturating_sub(components(root_key.key()));
        path_from_bytes(&key.path_bytes)
            .ancestors()
            .take(below)
            .any(named)
    }
}

/// The number of components in a `path_key`, whose separators are already one `/` each.
fn components(key: &[u8]) -> usize {
    key.split(|b| *b == b'/').filter(|c| !c.is_empty()).count()
}
