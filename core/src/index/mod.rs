//! The core's single SQLite connection and its storage invariants.

pub mod backup;
pub mod completion;
pub mod error;
pub mod migrate;
pub mod path;
pub mod recovery;
pub mod sidecar;
pub mod subject;

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

    /// Borrow the connection for one read.
    ///
    /// A read opens no transaction, so it needs no `TxGuard`: the interlock exists to stop a
    /// pipe write while a *write* lock is held (§2.2), and a bare `SELECT` holds none.
    pub fn read<T>(
        &self,
        f: impl FnOnce(&Connection) -> Result<T, IndexError>,
    ) -> Result<T, IndexError> {
        f(&self.conn)
    }

    /// Run `f` inside one transaction, committing on success and rolling back on error.
    ///
    /// **Every rusqlite transaction in the core opens through `TxGuard`** (plan 03 Task 4). If
    /// one does not, `FrameSink::send`'s check is always false and the whole pipe-write
    /// interlock is decorative.
    ///
    /// Takes `&mut self` because `rusqlite::Connection::transaction` does. Callers holding the
    /// index behind a `Mutex` lock it mutably for the write and release it immediately — see
    /// `core::jobs::run_one`, which never holds the lock across a git invocation.
    pub fn with_tx<T>(
        &mut self,
        f: impl FnOnce(&rusqlite::Transaction<'_>) -> Result<T, IndexError>,
    ) -> Result<T, IndexError> {
        let _tx_guard = crate::proto::txguard::TxGuard::enter();
        let tx = self.conn.transaction()?;
        let out = f(&tx)?;
        tx.commit()?;
        Ok(out)
    }

    /// §11.2 step 2 and §1.12, in one door. Uses the wall clock for backup names and the
    /// restore time; `open_at` is the same path with the clock supplied.
    pub fn open(data_dir: &Path) -> Result<Self, IndexError> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0));
        Self::open_at(data_dir, now)
    }

    pub fn open_at(data_dir: &Path, now: i64) -> Result<Self, IndexError> {
        open_with_migrations(data_dir, migrate::MIGRATIONS, now)
    }

    pub fn schema_version(&self) -> Result<u32, IndexError> {
        migrate::schema_version(&self.conn)
    }

    /// One `app_meta` value as text, or `None` when the key is unset.
    ///
    /// `optional()` rather than `.ok()`: rusqlite reports "no row" as an error, and swallowing
    /// every error would read a locked or corrupt database as "the key was never written".
    pub fn app_meta(&self, key: &str) -> Result<Option<String>, IndexError> {
        use rusqlite::OptionalExtension as _;
        Ok(self
            .conn
            .query_row(
                "SELECT v FROM app_meta WHERE k = ?1",
                rusqlite::params![key],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// Writes one `app_meta` value, replacing any previous one.
    pub fn set_app_meta(&self, key: &str, value: &str) -> Result<(), IndexError> {
        self.conn.execute(
            "INSERT INTO app_meta (k, v) VALUES (?1, ?2)
             ON CONFLICT(k) DO UPDATE SET v = excluded.v",
            rusqlite::params![key, value],
        )?;
        Ok(())
    }

    /// The one place an `Index` is constructed from an already-open connection.
    fn from_parts(conn: Connection, data_dir: PathBuf) -> Self {
        Self { conn, data_dir }
    }

    /// §1.12's `REBUILD`: quarantine the unreadable files, open a fresh database, and put the
    /// sidecar's global half back so the user is not asked for consent and roots again.
    pub fn rebuild(
        data_dir: &Path,
        now: i64,
    ) -> Result<(Self, recovery::RebuildReport), IndexError> {
        let db = Self::db_path(data_dir);
        let quarantined = recovery::quarantine(&db, now)?;

        let mut conn = open_connection(&db)?;
        migrate::apply_all(&mut conn, migrate::MIGRATIONS)?;

        let sidecar_path = Self::sidecar_path(data_dir);
        let (restored, deferred, gap_started_at) = if sidecar_path.exists() {
            let doc = sidecar::read(&sidecar_path)?;
            let restored = sidecar::restore_global(&conn, &doc)?;
            let all = sidecar::counts(&doc);
            // The global half has landed; what is left is the project-scoped remainder.
            let deferred = sidecar::SidecarCounts {
                roots: 0,
                identities: 0,
                settings: 0,
                view_state: 0,
                collections: 0,
                ..all
            };
            conn.execute(
                "INSERT INTO app_meta (k, v) VALUES ('restore_pending_generation', ?1)
                 ON CONFLICT(k) DO UPDATE SET v = excluded.v",
                [doc.generation.to_string()],
            )?;
            (restored, deferred, Some(doc.written_at))
        } else {
            (
                sidecar::RestoreCounts::default(),
                sidecar::SidecarCounts::default(),
                None,
            )
        };

        let index = Self::from_parts(conn, data_dir.to_path_buf());
        Ok((
            index,
            recovery::RebuildReport {
                quarantined,
                restored,
                deferred,
                gap_started_at,
                gap_counts_recoverable: false,
            },
        ))
    }

    /// Export the non-derivable set and write it atomically. §1.12: hourly and on clean
    /// shutdown, never on every change.
    pub fn export_sidecar(&self, now: i64) -> Result<sidecar::SidecarWriteReport, IndexError> {
        let generation = self.meta_u64("sidecar_generation")?.saturating_add(1);
        let doc = sidecar::export(&self.conn, generation, now)?;
        let path = Self::sidecar_path(&self.data_dir);
        sidecar::write_atomically(&doc, &path)?;
        self.conn.execute(
            "INSERT INTO app_meta (k, v) VALUES ('sidecar_generation', ?1)
             ON CONFLICT(k) DO UPDATE SET v = excluded.v",
            [generation.to_string()],
        )?;
        self.conn.execute(
            "INSERT INTO app_meta (k, v) VALUES ('sidecar_written_at', ?1)
             ON CONFLICT(k) DO UPDATE SET v = excluded.v",
            [now.to_string()],
        )?;
        Ok(sidecar::SidecarWriteReport {
            path,
            generation,
            written_at: now,
            counts: sidecar::counts(&doc),
        })
    }

    pub fn sidecar_due(&self, now: i64) -> Result<bool, IndexError> {
        let last = self.meta_i64("sidecar_written_at")?;
        Ok(sidecar::is_due(last, now))
    }

    fn meta_u64(&self, key: &str) -> Result<u64, IndexError> {
        Ok(self.meta_i64(key)?.unwrap_or(0).max(0).unsigned_abs())
    }

    fn meta_i64(&self, key: &str) -> Result<Option<i64>, IndexError> {
        use rusqlite::OptionalExtension as _;
        let raw: Option<String> = self
            .conn
            .query_row("SELECT v FROM app_meta WHERE k = ?1", [key], |r| r.get(0))
            .optional()?;
        Ok(raw.and_then(|s| s.parse::<i64>().ok()))
    }

    #[must_use]
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }
}

/// Open the one connection, apply the required pragmas, and take its lock eagerly.
/// The open path, with the migration set injectable so the failure branch is testable.
pub fn open_with_migrations(
    data_dir: &Path,
    migrations: &[migrate::Migration],
    now: i64,
) -> Result<Index, IndexError> {
    let db = Index::db_path(data_dir);
    let ceiling = migrations.last().map_or(0, |m| m.version);

    let mut conn = open_connection(&db)?;
    migrate::guard_not_from_the_future(&conn, ceiling)?;

    let from = migrate::schema_version(&conn)?;
    // Nothing to lose at version 0, and a backup of an empty file is noise.
    let backup = if from > 0 && from < ceiling {
        Some(backup::backup_before_migrating(&conn, data_dir, from, now)?)
    } else {
        None
    };

    if let Err(e) = migrate::apply_all(&mut conn, migrations) {
        if let IndexError::SchemaFromFuture { .. } = e {
            return Err(e);
        }
        let failed = migrations
            .iter()
            .find(|m| m.version > from)
            .copied()
            .unwrap_or(migrate::Migration {
                version: from,
                name: "unknown",
                sql: "",
                rebuilds_a_table: false,
            });
        let detail = e.to_string();
        drop(conn);
        if let Some(backup) = backup {
            backup::restore_over(&backup, &db)?;
        }
        return Err(IndexError::MigrationFailed {
            version: failed.version,
            name: failed.name,
            restored_to: from,
            restored_at: now,
            detail,
        });
    }

    // §1.12 and §1.9 both name a schema version. The pragma is authoritative and app_meta
    // mirrors it; a disagreement means something that is not this program wrote one of them.
    let reached = migrate::schema_version(&conn)?;
    let mirror: Option<String> = {
        use rusqlite::OptionalExtension as _;
        conn.query_row("SELECT v FROM app_meta WHERE k='schema_version'", [], |r| {
            r.get(0)
        })
        .optional()?
    };
    match mirror.and_then(|s| s.parse::<u32>().ok()) {
        Some(meta) if meta != reached => {
            return Err(IndexError::VersionMirrorMismatch {
                pragma: reached,
                meta,
            })
        }
        _ => {
            conn.execute(
                "INSERT INTO app_meta (k, v) VALUES ('schema_version', ?1)
                 ON CONFLICT(k) DO UPDATE SET v = excluded.v",
                [reached.to_string()],
            )?;
        }
    }

    Ok(Index::from_parts(conn, data_dir.to_path_buf()))
}

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
        _ => recovery::classify(error),
    }
}
