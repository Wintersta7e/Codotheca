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
pub const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "meta_and_projects",
        sql: include_str!("../../migrations/0001_meta_and_projects.sql"),
    },
    Migration {
        version: 2,
        name: "locations_and_roots",
        sql: include_str!("../../migrations/0002_locations_and_roots.sql"),
    },
    Migration {
        version: 3,
        name: "identity_and_events",
        sql: include_str!("../../migrations/0003_identity_and_events.sql"),
    },
    Migration {
        version: 4,
        name: "sessions_targets_collections",
        sql: include_str!("../../migrations/0004_sessions_targets_collections.sql"),
    },
    Migration {
        version: 5,
        name: "scan_and_art",
        sql: include_str!("../../migrations/0005_scan_and_art.sql"),
    },
    Migration {
        version: 6,
        name: "identity_columns",
        sql: include_str!("../../migrations/0006_identity_columns.sql"),
    },
];

/// The latest schema version this build understands.
///
/// This stays a literal for the Rust 1.80 minimum version. The integration test keeps it in
/// sync with the last entry in [`MIGRATIONS`].
pub const SUPPORTED_SCHEMA_VERSION: u32 = 6;

pub fn schema_version(conn: &Connection) -> Result<u32, IndexError> {
    let version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    Ok(u32::try_from(version).unwrap_or(0))
}

/// Apply every migration above the current version, each in its own transaction.
///
/// Applying an already-applied set is successful and returns the version already reached.
/// §1.12: a database written by a newer build is refused, never opened-and-written.
///
/// The error carries `on_disk` and `supported` as numbers rather than a formatted string
/// because §11.2a's window prints them itself; "please update" without them is unactionable.
pub fn guard_not_from_the_future(conn: &Connection, supported: u32) -> Result<(), IndexError> {
    let on_disk = schema_version(conn)?;
    if on_disk > supported {
        return Err(IndexError::SchemaFromFuture { on_disk, supported });
    }
    Ok(())
}

pub fn apply_all(conn: &mut Connection, migrations: &[Migration]) -> Result<u32, IndexError> {
    let ceiling = migrations.last().map_or(0, |m| m.version);
    guard_not_from_the_future(conn, ceiling)?;
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
