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
    /// An empty cache: nothing has been suggested yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Replace the remembered keys with the ones the latest `roots.suggest` produced.
    pub fn remember(&self, keys: impl IntoIterator<Item = Vec<u8>>) {
        if let Ok(mut set) = self.keys.lock() {
            set.clear();
            set.extend(keys);
        }
    }

    /// Whether `key` is a path the latest `roots.suggest` proposed.
    #[must_use]
    pub fn contains(&self, key: &[u8]) -> bool {
        self.keys.lock().is_ok_and(|set| set.contains(key))
    }
}

/// Everything `add_root` needs.
#[derive(Debug)]
pub struct AddParams<'a> {
    /// The root's path as raw bytes, from the shell's dialog or a suggestion.
    pub path_bytes: &'a [u8],
    /// The user has already confirmed a large root, so no directory estimate is taken.
    pub confirm_large: bool,
    /// The home directory, which a root may not be on its own (§10.1a).
    pub home: &'a Path,
    /// The directories the estimate does not descend into.
    pub skip: &'a SkipList,
    /// The latest suggestions; a suggested path is never estimated (§10.1b).
    pub cache: &'a SuggestionCache,
    /// The platform whose rules fold the path into its comparison key (R2).
    pub platform: PathPlatform,
    /// The WSL distro the path lives in; empty for a native path.
    pub distro: &'a str,
    /// Where the path came from: the dialog is stored as `user`, anything else as `suggested`.
    pub provenance: RootProvenance,
    /// Unix seconds stamped as the root's `added_at`.
    pub now: i64,
    /// The ceiling, overridable so a test does not have to create half a million directories.
    pub ceiling_for_tests: i64,
}

impl AddParams<'_> {
    const fn kind(&self) -> LocationKind {
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

const fn kind_str(kind: LocationKind) -> &'static str {
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
/// §1.10 gives `path_display` exactly one read door, so it is fetched through
/// `index::path::display_paths_for_ui` rather than selected here; `core/tests/index_paths.rs`
/// scans the source to keep that true. Everything else comes from the row.
///
/// # Errors
/// Returns [`IndexError`] when the read fails.
pub fn root_row(conn: &rusqlite::Connection, id: i64) -> Result<Option<Root>, IndexError> {
    let mut stmt = conn.prepare(
        "SELECT id, kind, distro, enabled, descend_into_repos, added_by, added_at
           FROM scan_root WHERE id = ?1",
    )?;
    let mut rows = stmt.query(rusqlite::params![id])?;
    let Some(row) = rows.next()? else {
        return Ok(None);
    };
    let kind: String = row.get(1)?;
    let added_by: String = row.get(5)?;
    let enabled: i64 = row.get(3)?;
    let root = Root {
        id: crate::protocol::RootId(row.get(0)?),
        path_display: String::new(),
        kind: match kind.as_str() {
            "win" => LocationKind::Win,
            "wsl" => LocationKind::Wsl,
            _ => LocationKind::Linux,
        },
        distro: row.get(2)?,
        enabled: enabled != 0,
        descend_into_repos: row.get::<_, i64>(4)? != 0,
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
        added_at: row.get(6)?,
        // §10.1b: `?` where a count exists but is not computed. Never `0`, and never a
        // fabricated number before a walk has run.
        project_count: None,
    };
    drop(rows);
    drop(stmt);

    let displays = crate::index::path::display_paths_for_ui(
        conn,
        crate::index::path::DisplayPathTable::ScanRoot,
        &[id],
    )?;
    // The row went away between the two reads. A `Root` with no display string is not a row
    // any surface can draw, so it is reported absent rather than blank.
    let Some((_, path_display)) = displays.into_iter().next() else {
        return Ok(None);
    };
    Ok(Some(Root {
        path_display,
        ..root
    }))
}

/// Every scan root, oldest first — §11.3a's rows, and the caption's `M`.
///
/// **R33 gap 1**: §2.4's table declared only the four mutations, so nothing returned the set the
/// surface enumerates. Read-only, and no path crosses in either direction: a `Root` carries
/// `path_display` and a `RootId`, never bytes (§1.3, §2.4).
///
/// `root_row` stays the single place a `scan_root` row becomes a `Root`, so the two cannot drift
/// — including its refusal to invent `project_count`, which is `None` until a walk has run.
///
/// # Errors
/// Returns [`IndexError`] when the read fails.
pub fn list_roots(conn: &rusqlite::Connection) -> Result<Vec<Root>, IndexError> {
    let mut stmt = conn.prepare("SELECT id FROM scan_root ORDER BY added_at, id")?;
    let ids = stmt
        .query_map([], |r| r.get::<_, i64>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        if let Some(root) = root_row(conn, id)? {
            out.push(root);
        }
    }
    Ok(out)
}

/// Stop walking there. **Not a destructive operation, and it must not become one.**
///
/// §17: phase 1 has none. This deletes one `scan_root` row — no `project`, no `location`,
/// nothing on disk. Projects already found stay found; a location no longer under an enabled
/// root becomes `unscanned` through the presence pass (§4.6), never a deleted row.
///
/// Returns false when the id was already gone, which is not an error: §2.2 lets an
/// unacknowledged request be replayed, and a second removal changes nothing.
///
/// # Errors
/// Returns [`IndexError`] when the write fails.
pub fn remove_root(conn: &rusqlite::Connection, id: i64) -> Result<bool, IndexError> {
    Ok(conn.execute("DELETE FROM scan_root WHERE id = ?1", rusqlite::params![id])? > 0)
}

/// §4.6's "disable a root", and §11.3a's checkbox — the reversible control that exists so
/// `roots.remove` never has to be drawn.
///
/// The flag is all this writes. §4.6's *"disabling a root marks its locations `unscanned`"* is
/// computed by the presence pass; its immediate form, `scan::presence::mark_root_unscanned`,
/// needs a `ScanStore`, and **R40** records that no production implementation of that trait
/// exists yet. Re-deriving §4.6's rule in SQL here would be one value in two places.
///
/// # Errors
/// Returns [`IndexError`] when the write fails.
pub fn set_enabled(
    conn: &rusqlite::Connection,
    id: i64,
    enabled: bool,
) -> Result<bool, IndexError> {
    let changed = conn.execute(
        "UPDATE scan_root SET enabled = ?2 WHERE id = ?1",
        rusqlite::params![id, i64::from(enabled)],
    )?;
    Ok(changed > 0)
}

/// §4.2's `descend_into_repos`: the walk stops at a repository root unless this is set. Nested
/// submodules are enumerated from `.gitmodules` either way, so this changes coverage, not
/// correctness.
///
/// # Errors
/// Returns [`IndexError`] when the write fails.
pub fn set_descend(
    conn: &rusqlite::Connection,
    id: i64,
    descend_into_repos: bool,
) -> Result<bool, IndexError> {
    let changed = conn.execute(
        "UPDATE scan_root SET descend_into_repos = ?2 WHERE id = ?1",
        rusqlite::params![id, i64::from(descend_into_repos)],
    )?;
    Ok(changed > 0)
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

    /// A project and a location beneath the root, so the removal test can prove that found
    /// work survives. A **fixture**, not a production writer: the production writer of a
    /// `location` is `identity::store::upsert_location` (R1, R27).
    fn seed_found_project(conn: &rusqlite::Connection, path: &str) {
        conn.execute(
            "INSERT INTO project (name, seed_basename, created_at, updated_at)
             VALUES ('p', 'p', 1, 1)",
            [],
        )
        .unwrap();
        let project = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO location
               (project_id, kind, distro, path_bytes, path_key, path_display, store_key,
                presence, repo_kind)
             VALUES (?1, 'linux', '', ?2, ?3, ?4, 's', 'present', 'worktree')",
            rusqlite::params![
                project,
                path.as_bytes(),
                crate::paths::path_key(Path::new(path), PathPlatform::Unix),
                path,
            ],
        )
        .unwrap();
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
    #[test]
    fn the_set_is_listed_in_the_order_it_was_added_and_no_count_is_invented() {
        let (_dir, index) = open();
        let conn = index.conn();
        let skip = SkipList::default();
        let cache = SuggestionCache::new();
        assert!(
            list_roots(conn).unwrap().is_empty(),
            "an empty set is empty, not an error"
        );

        add_root(conn, &params("/home/u/dev", false, &skip, &cache)).unwrap();
        add_root(conn, &params("/home/u/work", false, &skip, &cache)).unwrap();
        let rows = list_roots(conn).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].path_display, "/home/u/dev");
        assert_eq!(rows[1].path_display, "/home/u/work");
        // §10.1b / §11.3a: uncomputed, never zero.
        assert!(rows.iter().all(|r| r.project_count.is_none()));
    }

    // §17: phase 1 has no destructive operation. Removing a root removes a scan_root row and
    // nothing else — the projects and locations already found are untouched.
    #[test]
    fn removing_a_root_removes_the_root_row_and_deletes_nothing_else() {
        let (_dir, index) = open();
        let conn = index.conn();
        let skip = SkipList::default();
        let cache = SuggestionCache::new();
        let id = add_root(conn, &params("/home/u/dev", false, &skip, &cache))
            .unwrap()
            .root
            .expect("a root")
            .id
            .0;
        seed_found_project(conn, "/home/u/dev/p");

        assert!(remove_root(conn, id).unwrap());
        assert!(list_roots(conn).unwrap().is_empty());
        // The project and its location survive: they were found, and finding is not undone.
        let projects: i64 = conn
            .query_row("SELECT COUNT(*) FROM project", [], |r| r.get(0))
            .unwrap();
        let locations: i64 = conn
            .query_row("SELECT COUNT(*) FROM location", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            (projects, locations),
            (1, 1),
            "removing a root deletes no found work"
        );

        // §2.2: replaying an unacknowledged request must not fail the second time.
        assert!(!remove_root(conn, id).unwrap());
    }

    // §4.6: the reversible control, which is why §11.3a draws this and never roots.remove.
    #[test]
    fn disabling_a_root_keeps_the_row_and_flips_only_the_flag() {
        let (_dir, index) = open();
        let conn = index.conn();
        let skip = SkipList::default();
        let cache = SuggestionCache::new();
        let id = add_root(conn, &params("/home/u/dev", false, &skip, &cache))
            .unwrap()
            .root
            .expect("a root")
            .id
            .0;

        assert!(set_enabled(conn, id, false).unwrap());
        let row = root_row(conn, id).unwrap().expect("still there");
        assert!(!row.enabled);
        assert_eq!(row.state, crate::protocol::RootState::Ignored);
        assert!(!row.descend_into_repos, "the other toggle did not move");

        assert!(set_enabled(conn, id, true).unwrap());
        assert!(root_row(conn, id).unwrap().expect("still there").enabled);
        // An id that is not a root writes nothing and is not an error.
        assert!(!set_enabled(conn, id + 999, false).unwrap());
    }

    // §4.2: stop at a repository root unless descend_into_repos is set.
    #[test]
    fn the_descend_toggle_is_independent_of_the_enabled_one() {
        let (_dir, index) = open();
        let conn = index.conn();
        let skip = SkipList::default();
        let cache = SuggestionCache::new();
        let id = add_root(conn, &params("/home/u/dev", false, &skip, &cache))
            .unwrap()
            .root
            .expect("a root")
            .id
            .0;

        assert!(set_descend(conn, id, true).unwrap());
        let row = root_row(conn, id).unwrap().expect("still there");
        assert!(row.descend_into_repos);
        assert!(row.enabled, "the other toggle did not move");
        assert!(!set_descend(conn, id + 999, true).unwrap());
    }
}
