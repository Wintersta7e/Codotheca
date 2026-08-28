//! The core's single SQLite connection and its storage invariants.

pub mod error;

use std::path::{Path, PathBuf};

use rusqlite::Connection;

pub use error::IndexError;

/// The one connection in the process, and the directory it lives in.
#[derive(Debug)]
pub struct Index {
    conn: Connection,
    data_dir: PathBuf,
}

impl Index {
    #[must_use]
    pub fn db_path(data_dir: &Path) -> PathBuf {
        data_dir.join("index.db")
    }

    #[must_use]
    pub fn sidecar_path(data_dir: &Path) -> PathBuf {
        data_dir.join("index-sidecar.json")
    }

    #[must_use]
    pub fn backup_dir(data_dir: &Path) -> PathBuf {
        data_dir.join("backups")
    }

    #[must_use]
    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    pub fn conn_mut(&mut self) -> &mut Connection {
        &mut self.conn
    }

    #[must_use]
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }
}

/// Open the one connection, apply the required pragmas, and take its lock eagerly.
pub fn open_connection(db: &Path) -> Result<Connection, IndexError> {
    if let Some(parent) = db.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let conn = Connection::open(db)?;

    let mode: String = conn
        .query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0))
        .map_err(|error| classify_open_error(error, db))?;
    if !mode.eq_ignore_ascii_case("wal") {
        return Err(IndexError::AlreadyOpen {
            path: db.to_path_buf(),
        });
    }

    conn.execute_batch(
        "PRAGMA foreign_keys=ON;
         PRAGMA synchronous=NORMAL;
         PRAGMA locking_mode=EXCLUSIVE;",
    )
    .map_err(|error| classify_open_error(error, db))?;

    let lock_result = {
        let _tx_guard = crate::proto::txguard::TxGuard::enter();
        conn.execute_batch("BEGIN IMMEDIATE; COMMIT;")
    };
    if let Err(error) = lock_result {
        return Err(match error.sqlite_error_code() {
            Some(rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked) => {
                IndexError::AlreadyOpen {
                    path: db.to_path_buf(),
                }
            }
            _ => classify_open_error(error, db),
        });
    }

    conn.execute_batch("PRAGMA busy_timeout=5000;")?;
    Ok(conn)
}

fn classify_open_error(error: rusqlite::Error, db: &Path) -> IndexError {
    match error.sqlite_error_code() {
        Some(rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked) => {
            IndexError::AlreadyOpen {
                path: db.to_path_buf(),
            }
        }
        _ => IndexError::Sqlite(error),
    }
}
