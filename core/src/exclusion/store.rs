//! The `scan_exclusion` rows (§56.4), the one dialog-path normaliser, `location.excluded`
//! maintenance, and the `scan_exclusions` sidecar section (§56.9).
//!
//! A user exclusion is consent, so a rebuild cannot re-derive it: the section carries every row
//! as `{kind, entry, pathBytes, distro, createdAt}` and **never `match_key`**, which the restore
//! computes again from the bytes. A key read back from a file is a key nothing checked.

use rusqlite::{Connection, Transaction};

use super::list::{ExclusionList, LocationKey, PathEntry};
use crate::index::path::{native_platform, PathPlatform, StoredPath};
use crate::index::sidecar::{RestoreCtx, RestoreOutcome, SectionRow};
use crate::index::IndexError;
use crate::paths::is_under;
use crate::protocol::{Bytes, ExclusionKind, LocationId, LocationKind};
use crate::scan::run::platform_of;

/// One `scan_exclusion` row as the sidecar carries it: every column but the id and the match
/// key, both of which the restoring index computes for itself.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RestoredExclusion {
    /// A typed name or a dialog path.
    pub kind: ExclusionKind,
    /// The trimmed name, or the path's lossy display — shown, never compared (§1.10).
    pub entry: String,
    /// The path's bytes as `paths::path_bytes` wrote them; absent for a name.
    pub path_bytes: Option<Bytes>,
    /// The WSL distro a path lives in; empty for a native path and for every name.
    pub distro: String,
    /// When the user added the entry, in Unix seconds; the list's order after the built-ins.
    pub created_at: i64,
}

/// The rules a path entry's key is folded by: a path inside a distro is a Linux path, and a
/// native one is the host's own, because only the host's dialog produces it (§56.4).
#[must_use]
pub const fn entry_platform(distro: &str) -> PathPlatform {
    if distro.is_empty() {
        native_platform()
    } else {
        PathPlatform::Unix
    }
}

/// The stored word for `kind`, the column's `CHECK` vocabulary.
const fn kind_word(kind: ExclusionKind) -> &'static str {
    match kind {
        ExclusionKind::Name => "name",
        ExclusionKind::Path => "path",
    }
}

fn section_error(error: impl std::fmt::Display) -> IndexError {
    IndexError::Sidecar(format!("a scan_exclusions row: {error}"))
}

/// Write one exported row back, its `match_key` recomputed from its bytes (§56.9).
///
/// A row whose `(kind, match_key, distro)` the index already holds is left as it stands, so a
/// restore never rewrites an entry the user made after the rebuild. Answers whether it wrote.
///
/// # Errors
/// Fails when a path row carries no bytes, a name row carries some, or SQLite refuses the
/// write.
pub fn insert_restored(tx: &Transaction<'_>, row: &RestoredExclusion) -> Result<bool, IndexError> {
    let (path_bytes, match_key) = match (row.kind, &row.path_bytes) {
        (ExclusionKind::Path, Some(bytes)) => {
            let stored = StoredPath::from_bytes(bytes.0.clone(), entry_platform(&row.distro));
            (Some(bytes.0.clone()), stored.key().to_vec())
        }
        (ExclusionKind::Name, None) => (None, row.entry.as_bytes().to_vec()),
        (kind, bytes) => {
            return Err(section_error(format!(
                "a {} entry {} path bytes",
                kind_word(kind),
                if bytes.is_some() { "with" } else { "without" }
            )))
        }
    };
    let wrote = tx.execute(
        "INSERT INTO scan_exclusion (kind, entry, path_bytes, match_key, distro, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT (kind, match_key, distro) DO NOTHING",
        rusqlite::params![
            kind_word(row.kind),
            row.entry,
            path_bytes,
            match_key,
            row.distro,
            row.created_at,
        ],
    )?;
    Ok(wrote > 0)
}

/// The `scan_exclusions` section's export: every row, in the list's order, without its key.
///
/// # Errors
/// Fails when the rows cannot be read or encoded.
pub fn export_section(conn: &Connection) -> Result<Vec<SectionRow>, IndexError> {
    /// `(kind, entry, path_bytes, distro, created_at)`, as the columns hold them.
    type Columns = (String, String, Option<Vec<u8>>, String, i64);
    let rows: Vec<Columns> = conn
        .prepare(
            "SELECT kind, entry, path_bytes, distro, created_at FROM scan_exclusion
             ORDER BY created_at, id",
        )?
        .query_map([], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })?
        .collect::<Result<_, _>>()?;
    rows.into_iter()
        .map(|(kind, entry, path_bytes, distro, created_at)| {
            let kind: ExclusionKind =
                serde_json::from_value(serde_json::Value::String(kind)).map_err(section_error)?;
            let record = RestoredExclusion {
                kind,
                entry,
                path_bytes: path_bytes.map(Bytes),
                distro,
                created_at,
            };
            Ok(SectionRow {
                subject: None,
                location_keys: Vec::new(),
                data: serde_json::to_value(record).map_err(section_error)?,
            })
        })
        .collect()
}

/// The `scan_exclusions` section's restore, through [`insert_restored`]. Never `Pending`: an
/// entry names no project.
///
/// # Errors
/// Fails when the row does not decode as an exclusion, or [`insert_restored`] fails.
pub fn restore_section(
    tx: &Transaction<'_>,
    row: &SectionRow,
    _ctx: &RestoreCtx<'_>,
) -> Result<RestoreOutcome, IndexError> {
    let record: RestoredExclusion =
        serde_json::from_value(row.data.clone()).map_err(section_error)?;
    Ok(RestoreOutcome::Applied(u64::from(insert_restored(
        tx, &record,
    )?)))
}

/// A dialog's folder as the bytes, distro and platform an entry is keyed by (§56.4).
///
/// Never canonicalised: a location's key is the walk's bytes, and resolving a junction or a
/// `subst` drive on one side only would leave the entry covering nothing. A verbatim `\\?\` or
/// `\\?\UNC\` prefix is stripped instead, and a WSL bridge path becomes the distro's own Linux
/// path, so it covers that distro's `wsl` copies rather than none. Anything else keeps its bytes,
/// as `roots.add` keeps a root's.
#[must_use]
pub fn normalise_dialog_path(bytes: &[u8], distro: &str) -> (Vec<u8>, String, PathPlatform) {
    let plain = bytes.strip_prefix(br"\\?\UNC\").map_or_else(
        || bytes.strip_prefix(br"\\?\").unwrap_or(bytes).to_vec(),
        |share| [br"\\".as_slice(), share].concat(),
    );
    if let Some(bridge) = std::str::from_utf8(&plain)
        .ok()
        .and_then(crate::wsl::path::parse_bridge_path)
    {
        return (
            bridge.linux_path.into_bytes(),
            bridge.distro,
            PathPlatform::Unix,
        );
    }
    (plain, distro.to_owned(), entry_platform(distro))
}

/// The list in force for this index: the built-ins, then every stored entry in the order the
/// user added it.
///
/// # Errors
/// Fails when SQLite refuses the read.
pub fn load_list(conn: &Connection) -> Result<ExclusionList, IndexError> {
    /// `(kind, entry, match_key, distro)`, as the columns hold them.
    type Columns = (String, String, Vec<u8>, String);
    let rows: Vec<Columns> = conn
        .prepare(
            "SELECT kind, entry, match_key, distro FROM scan_exclusion ORDER BY created_at, id",
        )?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
        .collect::<Result<_, _>>()?;
    let mut names = Vec::new();
    let mut paths = Vec::new();
    for (kind, entry, match_key, distro) in rows {
        if kind == kind_word(ExclusionKind::Name) {
            names.push(entry);
        } else {
            paths.push(PathEntry {
                platform: entry_platform(&distro),
                distro,
                match_key,
            });
        }
    }
    Ok(ExclusionList::new(&names, paths))
}

/// The copies whose `location.excluded` one [`apply_coverage`] changed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CoverageDelta {
    /// Copies the list now covers that it did not, in id order.
    pub newly_covered: Vec<LocationId>,
    /// Copies the list no longer covers, in id order.
    pub newly_uncovered: Vec<LocationId>,
}

/// One `scan_root` as coverage reads it: its kind, distro and the key of its stored bytes.
struct RootKey {
    kind: String,
    distro: String,
    bytes: Vec<u8>,
    key: Vec<u8>,
}

/// Every `scan_root`, keyed from its bytes under the platform its `kind` implies (GA36).
fn load_roots(conn: &Connection) -> Result<Vec<RootKey>, IndexError> {
    Ok(conn
        .prepare("SELECT kind, distro, path_bytes FROM scan_root")?
        .query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get(1)?, r.get::<_, Vec<u8>>(2)?))
        })?
        .map(|row| {
            row.map(|(kind, distro, bytes)| {
                let key = StoredPath::from_bytes(bytes.clone(), platform_of(&kind))
                    .key()
                    .to_vec();
                RootKey {
                    kind,
                    distro,
                    bytes,
                    key,
                }
            })
        })
        .collect::<Result<_, _>>()?)
}

/// The bytes of the innermost root holding a copy keyed `path_key`: the longest key it lies under.
fn innermost_root(
    roots: &[RootKey],
    kind: LocationKind,
    distro: &str,
    path_key: &[u8],
) -> Option<Vec<u8>> {
    roots
        .iter()
        .filter(|r| r.kind == kind.as_str() && r.distro == distro && is_under(path_key, &r.key))
        .max_by_key(|r| r.key.len())
        .map(|r| r.bytes.clone())
}

/// What coverage of a copy at `path_bytes` is decided from before it has a `location` row: the
/// hand-off's candidate (§56.5 rows 2 and 4), keyed as its row would be.
///
/// # Errors
/// Fails when SQLite refuses the read of `scan_root`.
pub fn candidate_key(
    conn: &Connection,
    kind: LocationKind,
    distro: &str,
    path_bytes: &[u8],
) -> Result<LocationKey, IndexError> {
    let path_key = StoredPath::from_bytes(path_bytes.to_vec(), platform_of(kind.as_str()))
        .key()
        .to_vec();
    Ok(LocationKey {
        kind,
        distro: distro.to_owned(),
        path_bytes: path_bytes.to_vec(),
        root_path_bytes: innermost_root(&load_roots(conn)?, kind, distro, &path_key),
        path_key,
    })
}

/// Set `location.excluded` on every non-removed copy to `list`'s answer for it, and answer which
/// copies moved.
///
/// A root is keyed from its stored `path_bytes` under the platform its `kind` implies, never from
/// its stored `path_key` (GA36): the two disagree for a WSL root added on a Windows host, whose
/// key was folded like a Windows path while its copies keep case.
///
/// # Errors
/// Fails when SQLite refuses a read or a write, or a row holds a `kind` this build does not know.
pub fn apply_coverage(
    tx: &Transaction<'_>,
    list: &ExclusionList,
) -> Result<CoverageDelta, IndexError> {
    /// `(id, kind, distro, path_bytes, path_key, excluded)`.
    type Copy = (i64, LocationKind, String, Vec<u8>, Vec<u8>, bool);
    let roots = load_roots(tx)?;
    let copies: Vec<Copy> = tx
        .prepare(
            "SELECT id, kind, distro, path_bytes, path_key, excluded FROM location
             WHERE removed_at IS NULL ORDER BY id",
        )?
        .query_map([], |r| {
            // A kind this build does not know is a row from a newer schema; guessing would fold
            // its path by the wrong rules.
            let text: String = r.get(1)?;
            let kind = LocationKind::parse(&text).ok_or_else(|| {
                rusqlite::Error::FromSqlConversionFailure(
                    1,
                    rusqlite::types::Type::Text,
                    format!("location kind {text:?}").into(),
                )
            })?;
            Ok((r.get(0)?, kind, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?))
        })?
        .collect::<Result<_, _>>()?;

    let mut delta = CoverageDelta::default();
    let mut update = tx.prepare("UPDATE location SET excluded = ?1 WHERE id = ?2")?;
    for (id, kind, distro, path_bytes, path_key, was) in copies {
        let root = innermost_root(&roots, kind, &distro, &path_key);
        let now = list.covers_location(&LocationKey {
            kind,
            distro,
            path_bytes,
            path_key,
            root_path_bytes: root,
        });
        if now != was {
            update.execute(rusqlite::params![now, id])?;
            if now {
                delta.newly_covered.push(LocationId(id));
            } else {
                delta.newly_uncovered.push(LocationId(id));
            }
        }
    }
    Ok(delta)
}
