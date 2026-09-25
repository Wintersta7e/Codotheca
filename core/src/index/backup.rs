//! §1.12: back up before migrating, via SQLite's own copy and never a file copy.
//!
//! In WAL mode a copy of the main database file alone can omit committed transactions still
//! sitting in `-wal`. `index_migrate.rs` asserts that in both directions rather than trusting
//! the comment.

use std::path::{Path, PathBuf};

use rusqlite::Connection;

use super::IndexError;

/// A self-contained, fully checkpointed copy of the live database.
///
/// # Errors
/// Fails when the destination's directory cannot be created or an old file there removed, or
/// SQLite refuses the `VACUUM INTO`.
pub fn vacuum_into(conn: &Connection, dest: &Path) -> Result<(), IndexError> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // VACUUM INTO refuses an existing file, so a re-run must clear the way first.
    if dest.exists() {
        std::fs::remove_file(dest)?;
    }
    conn.execute("VACUUM INTO ?1", [&dest.to_string_lossy()])?;
    Ok(())
}

/// `<data_dir>/backups/index-pre-<from_version>-<now>.db`.
///
/// # Errors
/// Fails wherever [`vacuum_into`] does.
pub fn backup_before_migrating(
    conn: &Connection,
    data_dir: &Path,
    from_version: u32,
    now: i64,
) -> Result<PathBuf, IndexError> {
    let dest =
        super::Index::backup_dir(data_dir).join(format!("index-pre-{from_version}-{now}.db"));
    vacuum_into(conn, &dest)?;
    Ok(dest)
}

/// Put a backup back. The caller must already have dropped the connection.
///
/// The `-wal` and `-shm` beside the database belong to the schema that just failed, and the
/// backup is a checkpointed database that needs neither, so both are removed.
///
/// # Errors
/// Fails when the copy over the database or the removal of a `-wal` or `-shm` fails.
pub fn restore_over(backup: &Path, db: &Path) -> Result<(), IndexError> {
    std::fs::copy(backup, db)?;
    for suffix in ["-wal", "-shm"] {
        let mut sibling = db.as_os_str().to_os_string();
        sibling.push(suffix);
        let sibling = PathBuf::from(sibling);
        if sibling.exists() {
            std::fs::remove_file(sibling)?;
        }
    }
    Ok(())
}
