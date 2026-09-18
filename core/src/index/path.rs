//! §1.3's three path columns, built once from OS bytes.
//!
//! `path_bytes` is operational and is a BLOB because Linux paths are arbitrary bytes.
//! `path_key` is the canonical comparison form. `path_display` is lossy, is for the UI, and is
//! never used to open, launch or compare (§1.10) — `index_paths.rs` scans the source to keep
//! that true rather than trusting this sentence.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathPlatform {
    Windows,
    Unix,
}

#[must_use]
pub fn native_platform() -> PathPlatform {
    if cfg!(windows) {
        PathPlatform::Windows
    } else {
        PathPlatform::Unix
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredPath {
    bytes: Vec<u8>,
    key: Vec<u8>,
    display: String,
}

impl StoredPath {
    #[must_use]
    pub fn from_bytes(bytes: Vec<u8>, platform: PathPlatform) -> Self {
        let key = canonical_key(&bytes, platform);
        let display = String::from_utf8_lossy(&bytes).into_owned();
        Self {
            bytes,
            key,
            display,
        }
    }

    #[must_use]
    pub fn from_os(path: &std::ffi::OsStr, platform: PathPlatform) -> Self {
        Self::from_bytes(os_bytes(path), platform)
    }

    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    #[must_use]
    pub fn key(&self) -> &[u8] {
        &self.key
    }

    /// `(path_bytes, path_key, path_display)`, in the order the DDL declares them. This is the
    /// only way a display string leaves this module.
    #[must_use]
    pub fn as_params(&self) -> (&[u8], &[u8], &str) {
        (&self.bytes, &self.key, &self.display)
    }
}

// R2a: the *folding* is decided by the `PathPlatform` argument, but obtaining the bytes stays
// host-scoped. On a Windows host an OsStr is UTF-16, so there is no byte view to borrow.
//
// `crate::paths` re-exposes this as a free function rather than declaring its own: two byte
// encodings for one `location.path_bytes` column is the "one value stated twice" defect with a
// BLOB behind it, and a path written by the scanner must equal the same path written by plan 08.
#[cfg(unix)]
pub(crate) fn os_bytes(s: &std::ffi::OsStr) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt as _;
    s.as_bytes().to_vec()
}

#[cfg(not(unix))]
pub(crate) fn os_bytes(s: &std::ffi::OsStr) -> Vec<u8> {
    s.to_string_lossy().into_owned().into_bytes()
}

/// Separators normalised to `/`, repeats collapsed, one trailing separator dropped unless the
/// whole path is that separator, and ASCII case folded on Windows only.
///
/// ASCII only: non-ASCII case folding is locale-dependent, and a `path_key` that varies with
/// the user's locale is a worse defect than two Windows paths differing in the case of a
/// non-ASCII character.
fn canonical_key(bytes: &[u8], platform: PathPlatform) -> Vec<u8> {
    let windows = platform == PathPlatform::Windows;
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut last_was_sep = false;
    for b in bytes {
        let c = if windows && *b == b'\\' { b'/' } else { *b };
        let c = if windows { c.to_ascii_lowercase() } else { c };
        if c == b'/' {
            if last_was_sep {
                continue;
            }
            last_was_sep = true;
        } else {
            last_was_sep = false;
        }
        out.push(c);
    }
    if out.len() > 1 && out.last() == Some(&b'/') {
        out.pop();
    }
    out
}

/// The three tables that hold a `path_display`. §11.1 renders `scan_root` and `scan_problem`
/// paths; §8.5.2 renders `location` paths.
#[derive(Debug, Clone, Copy)]
pub enum DisplayPathTable {
    Location,
    ScanRoot,
    ScanProblem,
    /// [p3] §28.9's `DebtItem.pathDisplay`. The item's own `path_bytes` is what anything would
    /// open or compare with; this is the lossy string the list draws, and it goes through the one
    /// permitted reader like every other.
    DebtItem,
}

impl DisplayPathTable {
    const fn table(self) -> &'static str {
        match self {
            Self::Location => "location",
            Self::ScanRoot => "scan_root",
            Self::ScanProblem => "scan_problem",
            Self::DebtItem => "debt_item",
        }
    }
}

/// The **only** function in the core permitted to read a `path_display` back out.
///
/// `index_paths.rs` scans the source to keep that true. Anything that needs a path to open,
/// launch or compare takes `path_bytes` instead — that is the whole of §1.10's rule.
pub fn display_paths_for_ui(
    conn: &rusqlite::Connection,
    table: DisplayPathTable,
    ids: &[i64],
) -> Result<Vec<(i64, String)>, super::IndexError> {
    let mut out = Vec::with_capacity(ids.len());
    let sql = format!(
        "SELECT id, path_display FROM {} WHERE id = ?1",
        table.table()
    );
    let mut stmt = conn.prepare(&sql)?;
    for id in ids {
        let mut rows = stmt.query([id])?;
        if let Some(row) = rows.next()? {
            out.push((row.get(0)?, row.get(1)?));
        }
    }
    Ok(out)
}
