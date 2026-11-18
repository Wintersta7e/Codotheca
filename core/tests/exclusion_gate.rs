//! §56.4–§56.5 and GA23: the one exclusion list, the gate that swaps it, the dialog-path
//! normaliser, and `location.excluded` as the list's answer materialised.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::Path;
use std::sync::Arc;

use codotheca_core::exclusion::gate::{Covered, ExclusionGate, GateSite};
use codotheca_core::exclusion::list::{ExclusionList, LocationKey, PathEntry};
use codotheca_core::exclusion::store::normalise_dialog_path;
#[cfg(unix)]
use codotheca_core::exclusion::store::{apply_coverage, load_list};
use codotheca_core::index::path::{native_platform, PathPlatform};
#[cfg(unix)]
use codotheca_core::index::rebuild::{rebuild_in_place, RebuildOutcome};
use codotheca_core::index::Index;
use codotheca_core::paths::{path_from_bytes, path_key};
use codotheca_core::proto::txguard::TxGuard;
#[cfg(unix)]
use codotheca_core::protocol::LocationId;
use codotheca_core::protocol::LocationKind;
#[cfg(unix)]
use rusqlite::{params, Connection};

const NOW: i64 = 1_760_000_000;

fn key(raw: &[u8], platform: PathPlatform) -> Vec<u8> {
    path_key(&path_from_bytes(raw), platform)
}

fn entry(distro: &str, raw: &[u8], platform: PathPlatform) -> PathEntry {
    PathEntry {
        distro: distro.to_owned(),
        platform,
        match_key: key(raw, platform),
    }
}

fn location(kind: LocationKind, distro: &str, raw: &[u8], root: Option<&[u8]>) -> LocationKey {
    let platform = if kind == LocationKind::Win {
        PathPlatform::Windows
    } else {
        PathPlatform::Unix
    };
    LocationKey {
        kind,
        distro: distro.to_owned(),
        path_bytes: raw.to_vec(),
        path_key: key(raw, platform),
        root_path_bytes: root.map(<[u8]>::to_vec),
    }
}

fn names(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| (*s).to_owned()).collect()
}

/// §4.3: a name applies strictly below a root; a root that carries the name is still read.
#[test]
fn a_name_covers_only_strictly_below_its_root() {
    let list = ExclusionList::new(&names(&["scratch"]), Vec::new());
    let unix = PathPlatform::Unix;
    assert!(list.covers_dir(Path::new("/r/scratch"), Path::new("/r"), "", unix));
    assert!(list.covers_dir(Path::new("/r/scratch/deep"), Path::new("/r"), "", unix));
    assert!(!list.covers_dir(Path::new("/r/keep"), Path::new("/r"), "", unix));
    assert!(
        !list.covers_dir(Path::new("/x/scratch"), Path::new("/x/scratch"), "", unix),
        "a root named like a name entry is still read"
    );

    let below = location(LocationKind::Linux, "", b"/r/scratch/repo", Some(b"/r"));
    let rooted_on_it = location(
        LocationKind::Linux,
        "",
        b"/x/scratch/repo",
        Some(b"/x/scratch"),
    );
    let no_root = location(LocationKind::Linux, "", b"/r/scratch/repo", None);
    assert!(list.covers_location(&below));
    assert!(!list.covers_location(&rooted_on_it));
    assert!(
        !list.covers_location(&no_root),
        "with no root a name has nothing to be below, so only path entries apply"
    );
    // The built-ins are in the same list, ahead of the user's.
    assert!(list.covers_dir(Path::new("/r/node_modules"), Path::new("/r"), "", unix));
}

/// A path entry covers itself and what lies beneath it, on a component boundary only.
#[test]
fn a_path_entry_covers_itself_and_descendants_on_a_component_boundary() {
    let unix = PathPlatform::Unix;
    let list = ExclusionList::new(&[], vec![entry("", b"/a/b", unix)]);
    assert!(list.covers_dir(Path::new("/a/b"), Path::new("/"), "", unix));
    assert!(list.covers_dir(Path::new("/a/b/c"), Path::new("/"), "", unix));
    assert!(!list.covers_dir(Path::new("/a/bc"), Path::new("/"), "", unix));
    assert!(!list.covers_dir(Path::new("/a"), Path::new("/"), "", unix));
    assert!(
        !list.covers_dir(Path::new("/a/b/c"), Path::new("/"), "d", unix),
        "a native entry covers nothing inside a distro"
    );

    assert!(list.covers_location(&location(LocationKind::Linux, "", b"/a/b", None)));
    assert!(list.covers_location(&location(LocationKind::Linux, "", b"/a/b/r", None)));
    assert!(!list.covers_location(&location(LocationKind::Linux, "", b"/a/bc/r", None)));
}

/// A Windows key folds case and separators, so a differently spelled location is still covered.
#[test]
fn windows_platform_keys_fold_case() {
    let win = PathPlatform::Windows;
    let list = ExclusionList::new(&[], vec![entry("", br"C:\Dev\Private", win)]);
    let spelled = location(LocationKind::Win, "", br"c:\DEV\private\repo", None);
    assert!(list.covers_location(&spelled));
    assert!(list.covers_dir(Path::new(r"c:/dev/PRIVATE/x"), Path::new(r"c:/"), "", win));
    assert!(!list.covers_location(&location(LocationKind::Win, "", br"C:\Dev\Public", None)));
}

/// Disagreement 2: never canonicalise; strip the verbatim prefix so the entry keys the bytes the
/// walk keys a location by. The platform here is explicit, so it runs on every host.
#[test]
fn a_verbatim_dialog_path_covers_a_location_stored_in_the_plain_form() {
    let (bytes, distro, _) = normalise_dialog_path(br"\\?\C:\x", "");
    assert_eq!(bytes.as_slice(), br"C:\x".as_slice());
    assert_eq!(distro, "");
    let (unc, _, _) = normalise_dialog_path(br"\\?\UNC\server\share\x", "");
    assert_eq!(unc.as_slice(), br"\\server\share\x".as_slice());

    let list = ExclusionList::new(&[], vec![entry("", &bytes, PathPlatform::Windows)]);
    assert!(list.covers_location(&location(LocationKind::Win, "", br"C:\x\r", None)));
}

/// The same, through the platform the normaliser itself answers on the host that draws the
/// dialog.
#[cfg(windows)]
#[test]
fn a_verbatim_dialog_path_covers_its_location_on_the_host() {
    let (bytes, distro, platform) = normalise_dialog_path(br"\\?\C:\x", "");
    assert_eq!(platform, PathPlatform::Windows);
    let list = ExclusionList::new(&[], vec![entry(&distro, &bytes, platform)]);
    assert!(list.covers_location(&location(LocationKind::Win, "", br"C:\x\r", None)));
}

/// Disagreement 3: a folder inside a distro reaches the dialog as a bridge path, and is kept as
/// the distro's Linux path, so it covers that distro's `wsl` locations and no other.
#[test]
fn a_bridge_path_covers_that_distros_wsl_locations_only() {
    for raw in [
        br"\\wsl.localhost\Ubuntu\home\u\p".as_slice(),
        br"\\?\UNC\wsl.localhost\Ubuntu\home\u\p".as_slice(),
    ] {
        let (bytes, distro, platform) = normalise_dialog_path(raw, "");
        assert_eq!(
            (bytes.as_slice(), distro.as_str(), platform),
            (b"/home/u/p".as_slice(), "Ubuntu", PathPlatform::Unix)
        );
    }
    let list = ExclusionList::new(&[], vec![entry("Ubuntu", b"/home/u/p", PathPlatform::Unix)]);
    assert!(list.covers_location(&location(LocationKind::Wsl, "Ubuntu", b"/home/u/p/r", None)));
    assert!(!list.covers_location(&location(LocationKind::Wsl, "Debian", b"/home/u/p/r", None)));
    assert!(!list.covers_location(&location(LocationKind::Linux, "", b"/home/u/p/r", None)));
    assert!(list.covers_dir(
        Path::new("/home/u/p/r"),
        Path::new("/home"),
        "Ubuntu",
        PathPlatform::Unix
    ));
}

/// GA2: a Linux path is arbitrary bytes and the key is a BLOB, so a non-UTF-8 entry still covers.
#[cfg(unix)]
#[test]
fn a_non_utf8_path_entry_covers_its_location() {
    let raw = b"/a/\xffpriv";
    let list = ExclusionList::new(&[], vec![entry("", raw, PathPlatform::Unix)]);
    assert!(list.covers_location(&location(LocationKind::Linux, "", b"/a/\xffpriv/r", None)));
    assert!(list.covers_dir(
        &path_from_bytes(b"/a/\xffpriv/r"),
        Path::new("/"),
        "",
        PathPlatform::Unix
    ));
    assert!(!list.covers_location(&location(LocationKind::Linux, "", b"/a/priv/r", None)));
}

/// GA23: `swap` returns the old list for the swap-back, `snapshot` answers the new one, and the
/// epoch moves so a walk caching a snapshot knows to read it again.
#[test]
fn swap_then_snapshot_returns_the_new_list_and_bumps_the_epoch() {
    let gate = ExclusionGate::new(ExclusionList::default());
    let before = gate.epoch();
    let held = gate.snapshot();
    let next = Arc::new(ExclusionList::new(&names(&["scratch"]), Vec::new()));
    let old = gate.swap(Arc::clone(&next));
    assert!(Arc::ptr_eq(&old, &held));
    assert!(Arc::ptr_eq(&gate.snapshot(), &next));
    assert!(gate.epoch() > before);
    assert!(
        !held.covers_dir(
            Path::new("/r/scratch"),
            Path::new("/r"),
            "",
            PathPlatform::Unix
        ),
        "a snapshot already taken keeps the list it was taken under"
    );

    let back = gate.swap(old);
    assert!(Arc::ptr_eq(&back, &next));
    assert!(Arc::ptr_eq(&gate.snapshot(), &held));
}

/// GA23: the mutex is held only to clone or replace the pointer. Every guard the gate takes is a
/// temporary consumed inside the expression that took it; a guard that is the whole value of a
/// statement is one bound to a name, which outlives that statement and can be held across a call
/// out.
#[test]
fn the_gate_holds_its_mutex_for_one_statement_only() {
    let source = include_str!("../src/exclusion/gate.rs");
    let code: String = source
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .flat_map(str::chars)
        .filter(|c| !c.is_whitespace())
        .collect();
    let sites: Vec<&str> = code
        .split([';', '{', '}'])
        .filter(|s| s.contains(".lock()"))
        .collect();
    eprintln!("lock sites: {}", sites.len());
    assert!(sites.len() >= 3, "snapshot, swap and the record each lock");
    for site in &sites {
        for bound in [
            ".lock()",
            ".lock()?",
            ".lock().unwrap()",
            ".lock().unwrap_or_else(PoisonError::into_inner)",
        ] {
            assert!(
                !site.ends_with(bound),
                "a guard outliving its statement: {site}"
            );
        }
    }
}

/// `check_read` and `check_write` answer `Covered` naming the site, and admit what is not.
#[test]
fn a_check_answers_covered_with_its_site() {
    let unix = PathPlatform::Unix;
    let gate = ExclusionGate::new(ExclusionList::new(&[], vec![entry("", b"/p", unix)]));
    let covered = location(LocationKind::Linux, "", b"/p/r", None);
    let allowed = location(LocationKind::Linux, "", b"/q/r", None);
    assert_eq!(
        gate.check_read(GateSite::Presence, &covered),
        Err(Covered {
            site: GateSite::Presence
        })
    );
    assert_eq!(gate.check_read(GateSite::Presence, &allowed), Ok(()));

    let (_dir, mut index) = fresh();
    let _guard = TxGuard::enter();
    let tx = index.conn_mut().transaction().unwrap();
    assert_eq!(
        gate.check_write(GateSite::JobWrite, &tx, &covered),
        Err(Covered {
            site: GateSite::JobWrite
        })
    );
    assert_eq!(gate.check_write(GateSite::JobWrite, &tx, &allowed), Ok(()));
    drop(tx);

    let sites = GateSite::ALL;
    for (i, a) in sites.iter().enumerate() {
        assert!(!sites[i + 1..].contains(a), "{a:?} listed twice");
    }
    eprintln!("gate sites: {}", sites.len());
    assert_eq!(gate.checks().len(), 4);
    assert_eq!(
        gate.checks()[0],
        (GateSite::Presence, false),
        "the record says the read was refused"
    );
}

fn fresh() -> (tempfile::TempDir, Index) {
    let dir = tempfile::tempdir().unwrap();
    let index = Index::open_at(dir.path(), NOW).unwrap();
    (dir, index)
}

#[cfg(unix)]
fn project(conn: &Connection) -> i64 {
    conn.execute(
        "INSERT INTO project (name, seed_basename, created_at, updated_at)
         VALUES ('p', 'p', 1, 1)",
        [],
    )
    .unwrap();
    conn.last_insert_rowid()
}

/// A fixture `location` row; its production writer is `identity::store::upsert_location`.
#[cfg(unix)]
fn seed_location(conn: &Connection, project: i64, kind: &str, distro: &str, raw: &[u8]) -> i64 {
    let platform = if kind == "win" {
        PathPlatform::Windows
    } else {
        PathPlatform::Unix
    };
    conn.execute(
        "INSERT INTO location
           (project_id, kind, distro, path_bytes, path_key, path_display, store_key, presence,
            repo_kind)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 's', 'present', 'worktree')",
        params![
            project,
            kind,
            distro,
            raw,
            key(raw, platform),
            String::from_utf8_lossy(raw)
        ],
    )
    .unwrap();
    conn.last_insert_rowid()
}

/// A fixture `scan_root` row whose stored key is given, so GA36's case can store a wrong one.
#[cfg(unix)]
fn seed_root(conn: &Connection, kind: &str, distro: &str, raw: &[u8], stored_key: &[u8]) {
    conn.execute(
        "INSERT INTO scan_root
           (kind, distro, path_bytes, path_key, path_display, added_by, added_at)
         VALUES (?1, ?2, ?3, ?4, ?5, 'user', 1)",
        params![kind, distro, raw, stored_key, String::from_utf8_lossy(raw)],
    )
    .unwrap();
}

#[cfg(unix)]
fn add_exclusion(conn: &Connection, kind: &str, entry: &str, raw: Option<&[u8]>, distro: &str) {
    let match_key = raw.map_or_else(
        || entry.as_bytes().to_vec(),
        |raw| key(raw, PathPlatform::Unix),
    );
    conn.execute(
        "INSERT INTO scan_exclusion (kind, entry, path_bytes, match_key, distro, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, 1)",
        params![kind, entry, raw, match_key, distro],
    )
    .unwrap();
}

#[cfg(unix)]
fn excluded(conn: &Connection, id: i64) -> i64 {
    conn.query_row("SELECT excluded FROM location WHERE id = ?1", [id], |r| {
        r.get(0)
    })
    .unwrap()
}

#[cfg(unix)]
fn apply(index: &mut Index) -> (Vec<LocationId>, Vec<LocationId>) {
    let list = load_list(index.conn()).unwrap();
    let _guard = TxGuard::enter();
    let tx = index.conn_mut().transaction().unwrap();
    let delta = apply_coverage(&tx, &list).unwrap();
    tx.commit().unwrap();
    (delta.newly_covered, delta.newly_uncovered)
}

/// `apply_coverage` sets and clears `excluded` for every non-removed location, and answers which
/// moved. A root is keyed from its stored bytes under its kind's platform (GA36), never from its
/// stored key: here a WSL root's key was folded like a Windows path, and the name below it must
/// still apply.
#[cfg(unix)]
#[test]
fn apply_coverage_sets_and_clears_excluded_and_returns_the_deltas() {
    let (_dir, mut index) = fresh();
    let (keep, by_name, by_path, removed, in_distro) = {
        let conn = index.conn();
        seed_root(conn, "linux", "", b"/r", b"/r");
        seed_root(conn, "wsl", "d", b"/home/U/dev", b"/home/u/dev");
        let p = project(conn);
        let keep = seed_location(conn, p, "linux", "", b"/r/keep");
        let by_name = seed_location(conn, p, "linux", "", b"/r/scratch/x");
        let by_path = seed_location(conn, p, "linux", "", b"/elsewhere/Private/x");
        let removed = seed_location(conn, p, "linux", "", b"/elsewhere/Private/gone");
        conn.execute(
            "UPDATE location SET removed_at = 5 WHERE id = ?1",
            [removed],
        )
        .unwrap();
        let in_distro = seed_location(conn, p, "wsl", "d", b"/home/U/dev/scratch/r");
        add_exclusion(conn, "name", "scratch", None, "");
        add_exclusion(
            conn,
            "path",
            "/elsewhere/Private",
            Some(b"/elsewhere/Private"),
            "",
        );
        (keep, by_name, by_path, removed, in_distro)
    };

    let (covered, uncovered) = apply(&mut index);
    eprintln!("newly covered {covered:?}, newly uncovered {uncovered:?}");
    assert_eq!(
        covered,
        vec![
            LocationId(by_name),
            LocationId(by_path),
            LocationId(in_distro)
        ]
    );
    assert_eq!(uncovered, Vec::<LocationId>::new());
    let conn = index.conn();
    assert_eq!(excluded(conn, keep), 0);
    assert_eq!(excluded(conn, by_name), 1);
    assert_eq!(excluded(conn, by_path), 1);
    assert_eq!(
        excluded(conn, removed),
        0,
        "a removed copy is not maintained"
    );
    assert_eq!(excluded(conn, in_distro), 1);

    let (again, none) = apply(&mut index);
    assert_eq!(
        (again, none),
        (Vec::new(), Vec::new()),
        "a second pass moves nothing"
    );

    index
        .conn()
        .execute("DELETE FROM scan_exclusion WHERE kind = 'path'", [])
        .unwrap();
    let (now_covered, now_uncovered) = apply(&mut index);
    assert_eq!(now_covered, Vec::<LocationId>::new());
    assert_eq!(now_uncovered, vec![LocationId(by_path)]);
    assert_eq!(excluded(index.conn(), by_path), 0);
    assert_eq!(excluded(index.conn(), by_name), 1);
}

/// §56.9: a rebuild restores the list, and the locations it re-creates carry the list's answer —
/// a stale `excluded` in the exported row is not trusted.
#[cfg(unix)]
#[test]
fn a_rebuild_applies_the_restored_list_to_the_restored_locations() {
    let (dir, index) = fresh();
    let (covered, stale) = {
        let conn = index.conn();
        let p = project(conn);
        // Removed by the user, so no scan can rediscover it and the sidecar carries it whole.
        conn.execute("UPDATE project SET removed_at = 5 WHERE id = ?1", [p])
            .unwrap();
        let covered = seed_location(conn, p, "linux", "", b"/elsewhere/Private/x");
        let stale = seed_location(conn, p, "linux", "", b"/kept/y");
        conn.execute("UPDATE location SET excluded = 1 WHERE id = ?1", [stale])
            .unwrap();
        add_exclusion(
            conn,
            "path",
            "/elsewhere/Private",
            Some(b"/elsewhere/Private"),
            "",
        );
        (covered, stale)
    };
    index.export_sidecar(NOW).unwrap();
    drop(index);
    std::fs::write(Index::db_path(dir.path()), b"this is not a database").unwrap();
    match rebuild_in_place(dir.path(), NOW + 1) {
        Ok(RebuildOutcome::Rebuilt(_)) => {}
        other => panic!("expected a rebuild, got {other:?}"),
    }
    let rebuilt = Index::open_at(dir.path(), NOW + 2).unwrap();
    assert_eq!(excluded(rebuilt.conn(), covered), 1);
    assert_eq!(excluded(rebuilt.conn(), stale), 0);
}

/// The native platform is what the normaliser answers for a plain host path.
#[test]
fn a_plain_dialog_path_keeps_its_bytes_and_the_host_platform() {
    let (bytes, distro, platform) = normalise_dialog_path(b"/home/u/x", "");
    assert_eq!(
        (bytes.as_slice(), distro.as_str(), platform),
        (b"/home/u/x".as_slice(), "", native_platform())
    );
}
