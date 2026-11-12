//! The `scan_exclusion` rows (§56.4) and the `scan_exclusions` sidecar section (§56.9).
//!
//! A user exclusion is consent, so a rebuild cannot re-derive it: the section carries every row
//! as `{kind, entry, pathBytes, distro, createdAt}` and **never `match_key`**, which the restore
//! computes again from the bytes. A key read back from a file is a key nothing checked.

use rusqlite::{Connection, Transaction};

use crate::index::path::{native_platform, PathPlatform, StoredPath};
use crate::index::sidecar::{RestoreCtx, RestoreOutcome, SectionRow};
use crate::index::IndexError;
use crate::protocol::{Bytes, ExclusionKind};

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
