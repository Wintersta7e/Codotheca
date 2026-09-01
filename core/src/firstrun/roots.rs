//! `roots.add`: the only write in first run, and the only place a path enters the database.

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Mutex;

use crate::index::path::PathPlatform;
use crate::index::IndexError;
use crate::protocol::{LocationKind, Root, RootAdd, RootProvenance, RootRefusal, RootState};
use crate::scan::skiplist::SkipList;

/// The path keys the last `roots.suggest` produced.
///
/// `roots.add` carries no origin flag, and §10.1b forbids taking a directory estimate on a
/// suggested row. A path the core itself proposed is in here; a path from the shell's dialog is
/// not. That is the whole distinction, and it needs no schema.
#[derive(Debug, Default)]
pub struct SuggestionCache {
    keys: Mutex<BTreeSet<Vec<u8>>>,
}

impl SuggestionCache {
    #[must_use]
    pub fn new() -> SuggestionCache {
        SuggestionCache::default()
    }

    pub fn remember(&self, keys: impl IntoIterator<Item = Vec<u8>>) {
        if let Ok(mut set) = self.keys.lock() {
            set.clear();
            set.extend(keys);
        }
    }

    #[must_use]
    pub fn contains(&self, key: &[u8]) -> bool {
        self.keys.lock().is_ok_and(|set| set.contains(key))
    }
}

/// Everything `add_root` needs.
#[derive(Debug)]
pub struct AddParams<'a> {
    pub path_bytes: &'a [u8],
    pub confirm_large: bool,
    pub home: &'a Path,
    pub skip: &'a SkipList,
    pub cache: &'a SuggestionCache,
    pub platform: PathPlatform,
    pub distro: &'a str,
    pub provenance: RootProvenance,
    pub now: i64,
    /// The ceiling, overridable so a test does not have to create half a million directories.
    pub ceiling_for_tests: i64,
}

impl AddParams<'_> {
    fn kind(&self) -> LocationKind {
        if self.distro.is_empty() {
            match self.platform {
                PathPlatform::Windows => LocationKind::Win,
                PathPlatform::Unix => LocationKind::Linux,
            }
        } else {
            LocationKind::Wsl
        }
    }
}

fn kind_str(kind: LocationKind) -> &'static str {
    match kind {
        LocationKind::Win => "win",
        LocationKind::Linux => "linux",
        LocationKind::Wsl => "wsl",
    }
}

/// Add a scan root, or say why not.
///
/// # Errors
/// Returns [`IndexError`] when the database write fails.
pub fn add_root(
    conn: &rusqlite::Connection,
    params: &AddParams<'_>,
) -> Result<RootAdd, IndexError> {
    let path = crate::paths::path_from_bytes(params.path_bytes);
    let refused = |refusal: RootRefusal, estimated: Option<i64>| RootAdd {
        root: None,
        refused_because: Some(refusal),
        estimated_dirs: estimated,
    };

    if let Some(refusal) = crate::firstrun::refuse::shape_refusal(&path, params.home) {
        return Ok(refused(refusal, None));
    }

    // R2: `path_key` takes the platform explicitly; it is never decided by the host.
    let key = crate::paths::path_key(&path, params.platform);
    let kind = params.kind();
    if already_a_root(conn, kind_str(kind), params.distro, &key)? {
        return Ok(refused(RootRefusal::AlreadyARoot, None));
    }

    // §10.1b: the estimate is itself a directory read, taken only on a dialog pick.
    if !params.confirm_large && !params.cache.contains(&key) {
        let ceiling = params.ceiling_for_tests;
        let estimated = crate::firstrun::refuse::estimate_directories(&path, ceiling, params.skip);
        if estimated >= ceiling {
            return Ok(refused(RootRefusal::TooManyDirectories, Some(estimated)));
        }
    }

    conn.execute(
        "INSERT INTO scan_root
           (kind, distro, path_bytes, path_key, path_display, enabled, added_by,
            descend_into_repos, added_at)
         VALUES (?1, ?2, ?3, ?4, ?5, 1, ?6, 0, ?7)",
        rusqlite::params![
            kind_str(kind),
            params.distro,
            params.path_bytes,
            key,
            crate::paths::path_display(&path),
            match params.provenance {
                RootProvenance::Dialog => "user",
                _ => "suggested",
            },
            params.now,
        ],
    )?;
    let id = conn.last_insert_rowid();
    Ok(RootAdd {
        root: root_row(conn, id)?,
        refused_because: None,
        estimated_dirs: None,
    })
}

/// True when this exact path, or an enabled root above it, is already a scan root.
fn already_a_root(
    conn: &rusqlite::Connection,
    kind: &str,
    distro: &str,
    key: &[u8],
) -> Result<bool, IndexError> {
    let mut stmt = conn.prepare(
        "SELECT path_key FROM scan_root WHERE kind = ?1 AND distro = ?2 AND enabled = 1",
    )?;
    let rows = stmt.query_map(rusqlite::params![kind, distro], |r| r.get::<_, Vec<u8>>(0))?;
    for existing in rows {
        let existing = existing?;
        if existing == key || crate::paths::is_under(key, &existing) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// One `scan_root` row as the wire sees it.
///
/// # Errors
/// Returns [`IndexError`] when the read fails.
pub fn root_row(conn: &rusqlite::Connection, id: i64) -> Result<Option<Root>, IndexError> {
    let mut stmt = conn.prepare(
        "SELECT id, path_display, kind, distro, enabled, descend_into_repos, added_by, added_at
           FROM scan_root WHERE id = ?1",
    )?;
    let mut rows = stmt.query(rusqlite::params![id])?;
    let Some(row) = rows.next()? else {
        return Ok(None);
    };
    let kind: String = row.get(2)?;
    let added_by: String = row.get(6)?;
    let enabled: i64 = row.get(4)?;
    Ok(Some(Root {
        id: crate::protocol::RootId(row.get(0)?),
        path_display: row.get(1)?,
        kind: match kind.as_str() {
            "win" => LocationKind::Win,
            "wsl" => LocationKind::Wsl,
            _ => LocationKind::Linux,
        },
        distro: row.get(3)?,
        enabled: enabled != 0,
        descend_into_repos: row.get::<_, i64>(5)? != 0,
        provenance: if added_by == "user" {
            RootProvenance::Dialog
        } else {
            RootProvenance::Convention
        },
        state: if enabled != 0 {
            RootState::Watched
        } else {
            RootState::Ignored
        },
        added_at: row.get(7)?,
        // §10.1b: `?` where a count exists but is not computed. Never `0`, and never a
        // fabricated number before a walk has run.
        project_count: None,
    }))
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
    use crate::index::path::PathPlatform;
    use crate::protocol::{RootProvenance, RootRefusal, RootState};
    use crate::scan::skiplist::SkipList;
    use std::path::Path;

    /// A migrated, empty database. The `TempDir` is returned so it outlives the `Index`.
    fn open() -> (tempfile::TempDir, crate::index::Index) {
        let dir = tempfile::tempdir().unwrap();
        let index = crate::index::Index::open_at(dir.path(), 1_700_000_000).unwrap();
        (dir, index)
    }

    fn params<'a>(
        path: &'a str,
        confirm: bool,
        skip: &'a SkipList,
        cache: &'a SuggestionCache,
    ) -> AddParams<'a> {
        AddParams {
            path_bytes: path.as_bytes(),
            confirm_large: confirm,
            home: Path::new("/home/u"),
            skip,
            cache,
            platform: PathPlatform::Unix,
            distro: "",
            provenance: RootProvenance::Dialog,
            now: 1_700_000_000,
            ceiling_for_tests: crate::firstrun::refuse::DIRECTORY_CEILING,
        }
    }

    #[test]
    fn a_good_path_is_written_once_and_reported_back() {
        let (_dir, index) = open();
        let conn = index.conn();
        let skip = SkipList::default();
        let cache = SuggestionCache::new();
        let added = add_root(conn, &params("/home/u/dev", false, &skip, &cache)).unwrap();
        let root = added.root.expect("a root");
        assert!(added.refused_because.is_none());
        assert_eq!(root.path_display, "/home/u/dev");
        assert_eq!(root.state, RootState::Watched);
        assert!(root.enabled);
        assert!(!root.descend_into_repos);
        // §10.1b: the count under PROJECTS is `?` until a walk has run, never `0`.
        assert_eq!(root.project_count, None);
    }

    #[test]
    fn adding_the_same_path_twice_is_refused_with_a_reason() {
        let (_dir, index) = open();
        let conn = index.conn();
        let skip = SkipList::default();
        let cache = SuggestionCache::new();
        add_root(conn, &params("/home/u/dev", false, &skip, &cache)).unwrap();
        let again = add_root(conn, &params("/home/u/dev", false, &skip, &cache)).unwrap();
        assert!(again.root.is_none());
        assert_eq!(again.refused_because, Some(RootRefusal::AlreadyARoot));
    }

    #[test]
    fn a_path_already_covered_by_an_enabled_root_is_refused() {
        let (_dir, index) = open();
        let conn = index.conn();
        let skip = SkipList::default();
        let cache = SuggestionCache::new();
        add_root(conn, &params("/home/u/dev", false, &skip, &cache)).unwrap();
        let inner = add_root(conn, &params("/home/u/dev/inner", false, &skip, &cache)).unwrap();
        assert_eq!(inner.refused_because, Some(RootRefusal::AlreadyARoot));
    }

    #[test]
    fn the_three_shape_refusals_reach_the_reply() {
        let (_dir, index) = open();
        let conn = index.conn();
        let skip = SkipList::default();
        let cache = SuggestionCache::new();
        let root = add_root(conn, &params("/", false, &skip, &cache)).unwrap();
        assert_eq!(root.refused_because, Some(RootRefusal::FilesystemRoot));
        let home = add_root(conn, &params("/home/u", false, &skip, &cache)).unwrap();
        assert_eq!(
            home.refused_because,
            Some(RootRefusal::HomeWithoutNarrowing)
        );
        assert!(root.estimated_dirs.is_none());
    }

    // §10.1b: the estimate is taken only on a path just chosen in the shell's dialog, never on
    // a suggested row, where it would break the standfirst.
    #[test]
    fn a_suggested_path_is_never_estimated() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..6 {
            std::fs::create_dir_all(dir.path().join(format!("d{i}"))).unwrap();
        }
        let (_db, index) = open();
        let conn = index.conn();
        let skip = SkipList::default();
        let cache = SuggestionCache::new();
        let text = dir.path().to_string_lossy().into_owned();

        cache.remember([crate::paths::path_key(dir.path(), PathPlatform::Unix)]);
        let mut p = params(&text, false, &skip, &cache);
        p.provenance = RootProvenance::Gitconfig;
        let added = add_root(conn, &p).unwrap();
        assert!(
            added.root.is_some(),
            "a suggested row is written without an estimate"
        );
        assert!(added.estimated_dirs.is_none());
    }

    #[test]
    fn a_dialog_pick_over_the_ceiling_keeps_its_tick_and_asks() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..6 {
            std::fs::create_dir_all(dir.path().join(format!("d{i}"))).unwrap();
        }
        let (_db, index) = open();
        let conn = index.conn();
        let skip = SkipList::default();
        let cache = SuggestionCache::new();
        let text = dir.path().to_string_lossy().into_owned();

        let mut p = params(&text, false, &skip, &cache);
        p.ceiling_for_tests = 3;
        let asked = add_root(conn, &p).unwrap();
        assert_eq!(asked.refused_because, Some(RootRefusal::TooManyDirectories));
        assert_eq!(asked.estimated_dirs, Some(3));
        assert!(asked.root.is_none());

        let mut confirmed = params(&text, true, &skip, &cache);
        confirmed.ceiling_for_tests = 3;
        let out = add_root(conn, &confirmed).unwrap();
        assert!(out.root.is_some(), "confirmation writes the root");
    }
}
