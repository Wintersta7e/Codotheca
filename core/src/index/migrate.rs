//! Forward-only, numbered migrations applied one transaction at a time.
//!
//! There is no down-migration and no parallel schema definition. A numbered file in
//! `core/migrations/` is the only way a table enters the index.

use rusqlite::Connection;

use super::IndexError;
use crate::proto::txguard::TxGuard;

#[derive(Debug, Clone, Copy)]
pub struct Migration {
    pub version: u32,
    pub name: &'static str,
    pub sql: &'static str,
}

/// The shipped schema. Later tasks append numbered migrations to this slice.
pub const MIGRATIONS: &[Migration] = &[];

/// The latest schema version this build understands.
///
/// This stays a literal for the Rust 1.80 minimum version. The integration test keeps it in
/// sync with the last entry in [`MIGRATIONS`].
pub const SUPPORTED_SCHEMA_VERSION: u32 = 0;

pub fn schema_version(conn: &Connection) -> Result<u32, IndexError> {
    let version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    Ok(u32::try_from(version).unwrap_or(0))
}

/// Apply every migration above the current version, each in its own transaction.
///
/// Applying an already-applied set is successful and returns the version already reached.
pub fn apply_all(conn: &mut Connection, migrations: &[Migration]) -> Result<u32, IndexError> {
    let mut current = schema_version(conn)?;
    for migration in migrations {
        if migration.version <= current {
            continue;
        }

        let _tx_guard = TxGuard::enter();
        let tx = conn.transaction()?;
        tx.execute_batch(migration.sql)?;
        tx.execute_batch(&format!("PRAGMA user_version = {};", migration.version))?;
        tx.commit()?;
        current = migration.version;
    }
    Ok(current)
}
