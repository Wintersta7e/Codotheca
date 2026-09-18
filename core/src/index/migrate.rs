//! Forward-only, numbered migrations applied one transaction at a time.
//!
//! There is no down-migration and no parallel schema definition. A numbered file in
//! `core/migrations/` is the only way a table enters the index.

use rusqlite::{Connection, Transaction};

use super::IndexError;
use crate::proto::txguard::TxGuard;

#[derive(Debug, Clone, Copy)]
pub struct Migration {
    pub version: u32,
    pub name: &'static str,
    pub sql: &'static str,
    /// This file performs a create-copy-drop-rename, so the **runner** disables foreign keys
    /// around it (R59). The file itself carries no pragma, and that is not a style choice:
    /// `apply_all` wraps every file in a transaction and `PRAGMA foreign_keys` is a documented
    /// no-op inside one, so `PRAGMA foreign_keys=OFF` written into the SQL would do nothing and
    /// `DROP TABLE project` would fire seven `ON DELETE CASCADE` children with enforcement on —
    /// every art scene, Peek row, job state, collection membership and account link in the
    /// library, emptied silently.
    pub rebuilds_a_table: bool,
}

/// The shipped schema. Later tasks append numbered migrations to this slice.
pub const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "meta_and_projects",
        sql: include_str!("../../migrations/0001_meta_and_projects.sql"),
        rebuilds_a_table: false,
    },
    Migration {
        version: 2,
        name: "locations_and_roots",
        sql: include_str!("../../migrations/0002_locations_and_roots.sql"),
        rebuilds_a_table: false,
    },
    Migration {
        version: 3,
        name: "identity_and_events",
        sql: include_str!("../../migrations/0003_identity_and_events.sql"),
        rebuilds_a_table: false,
    },
    Migration {
        version: 4,
        name: "sessions_targets_collections",
        sql: include_str!("../../migrations/0004_sessions_targets_collections.sql"),
        rebuilds_a_table: false,
    },
    Migration {
        version: 5,
        name: "scan_and_art",
        sql: include_str!("../../migrations/0005_scan_and_art.sql"),
        rebuilds_a_table: false,
    },
    Migration {
        version: 6,
        name: "identity_columns",
        sql: include_str!("../../migrations/0006_identity_columns.sql"),
        rebuilds_a_table: false,
    },
    Migration {
        version: 7,
        name: "jobs_derived",
        sql: include_str!("../../migrations/0007_jobs_derived.sql"),
        rebuilds_a_table: false,
    },
    Migration {
        version: 8,
        name: "accounts",
        sql: include_str!("../../migrations/0008_accounts.sql"),
        rebuilds_a_table: false,
    },
    Migration {
        version: 9,
        name: "remote_identity_and_facts",
        sql: include_str!("../../migrations/0009_remote_identity_and_facts.sql"),
        // The one `project` rebuild phase 2 performs. `project` is STRICT and SQLite has no
        // ALTER CONSTRAINT, so widening `description_source`'s CHECK is a
        // create-copy-drop-rename — which drops a table seven `ON DELETE CASCADE` children
        // hang off.
        rebuilds_a_table: true,
    },
    Migration {
        version: 10,
        name: "install",
        sql: include_str!("../../migrations/0010_install.sql"),
        rebuilds_a_table: false,
    },
    Migration {
        version: 11,
        name: "sync",
        sql: include_str!("../../migrations/0011_sync.sql"),
        rebuilds_a_table: false,
    },
    Migration {
        version: 12,
        name: "content_scan",
        sql: include_str!("../../migrations/0012_content_scan.sql"),
        // §29.10: `project_job_state` is STRICT and SQLite has no ALTER CONSTRAINT, so widening
        // `job`'s CHECK for 'j7' is a create-copy-drop-rename.
        rebuilds_a_table: true,
    },
    Migration {
        version: 13,
        name: "debt",
        sql: include_str!("../../migrations/0013_debt.sql"),
        // §28.8: `xp_events` is STRICT and SQLite has no ALTER CONSTRAINT, so widening `kind`'s
        // CHECK for 'debt_day' and the track equivalence with it is a create-copy-drop-rename.
        rebuilds_a_table: true,
    },
];

/// The latest schema version this build understands.
///
/// This stays a literal for the Rust 1.80 minimum version. The integration test keeps it in
/// sync with the last entry in [`MIGRATIONS`].
pub const SUPPORTED_SCHEMA_VERSION: u32 = 13;

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

/// The registered chain is `1..=n`, in order, with no gap and no repeat. Returns the **count
/// checked**, so a guard that validated nothing cannot report success.
///
/// R68's general half. [`apply_all`] skips any migration whose `version <= current` and stamps
/// `PRAGMA user_version` per file, and nothing else anywhere checks that the registered versions
/// are contiguous — so a database that reaches the far side of a gap skips the missing migration
/// **forever**, silently, on a user's machine, in a lane that ran correctly.
///
/// It is `pub` so a test can feed it a holed list without building a database, and it refuses an
/// empty slice because `apply_all(&[])` returns `Ok(0)` today having asserted nothing at all.
pub fn guard_contiguous(migrations: &[Migration]) -> Result<u32, IndexError> {
    if migrations.is_empty() {
        return Err(IndexError::MigrationChainEmpty);
    }
    let mut checked: u32 = 0;
    for migration in migrations {
        let expected = checked.saturating_add(1);
        if migration.version != expected {
            return Err(IndexError::MigrationChainBroken {
                expected,
                found: migration.version,
            });
        }
        checked = expected;
    }
    Ok(checked)
}

pub fn apply_all(conn: &mut Connection, migrations: &[Migration]) -> Result<u32, IndexError> {
    guard_contiguous(migrations)?;
    let ceiling = migrations.last().map_or(0, |m| m.version);
    guard_not_from_the_future(conn, ceiling)?;
    let mut current = schema_version(conn)?;
    for migration in migrations {
        if migration.version <= current {
            continue;
        }
        if migration.rebuilds_a_table {
            apply_rebuild(conn, migration)?;
        } else {
            apply_one(conn, migration)?;
        }
        current = migration.version;
    }
    Ok(current)
}

fn apply_one(conn: &mut Connection, migration: &Migration) -> Result<(), IndexError> {
    let _tx_guard = TxGuard::enter();
    let tx = conn.transaction()?;
    tx.execute_batch(migration.sql)?;
    stamp_version(&tx, migration.version)?;
    tx.commit()?;
    Ok(())
}

/// R59's procedure, and the reason it lives here rather than in the `.sql` file.
///
/// 1. read the connection's current `foreign_keys` value, **outside** any transaction;
/// 2. set it off and **read it back** — a pragma that silently did nothing is indistinguishable
///    from one that worked, and that is the failure this whole path exists to prevent;
/// 3. run the file in a transaction, then evaluate `foreign_key_check` **in Rust**, because in a
///    `.sql` file it returns rows and never errors;
/// 4. restore the value the connection started with — not a hard-coded `ON` — on both the success
///    and the error path, so a failed rebuild cannot leave the connection in a state it did not
///    start in.
fn apply_rebuild(conn: &mut Connection, migration: &Migration) -> Result<(), IndexError> {
    let prior = read_foreign_keys(conn)?;
    conn.execute_batch("PRAGMA foreign_keys=OFF;")?;
    let reported = read_foreign_keys(conn)?;
    if reported != 0 {
        restore_foreign_keys(conn, prior)?;
        return Err(IndexError::ForeignKeysNotDisabled { reported });
    }

    let applied = run_rebuild(conn, migration);
    let restored = restore_foreign_keys(conn, prior);
    // The migration's own failure is the actionable one and wins; a restore failure surfaces
    // when the file itself succeeded. Either way the caller must not keep using a connection
    // whose enforcement is off.
    applied.and(restored)
}

fn run_rebuild(conn: &mut Connection, migration: &Migration) -> Result<(), IndexError> {
    let _tx_guard = TxGuard::enter();
    let tx = conn.transaction()?;
    tx.execute_batch(migration.sql)?;
    let count = count_foreign_key_violations(&tx)?;
    if count != 0 {
        // Dropping the transaction rolls it back.
        return Err(IndexError::ForeignKeyViolations { count });
    }
    stamp_version(&tx, migration.version)?;
    tx.commit()?;
    Ok(())
}

fn stamp_version(tx: &Transaction<'_>, version: u32) -> Result<(), IndexError> {
    tx.execute_batch(&format!("PRAGMA user_version = {version};"))?;
    Ok(())
}

fn read_foreign_keys(conn: &Connection) -> Result<i64, IndexError> {
    Ok(conn.query_row("PRAGMA foreign_keys", [], |row| row.get(0))?)
}

fn restore_foreign_keys(conn: &Connection, expected: i64) -> Result<(), IndexError> {
    conn.execute_batch(if expected == 0 {
        "PRAGMA foreign_keys=OFF;"
    } else {
        "PRAGMA foreign_keys=ON;"
    })?;
    let reported = read_foreign_keys(conn)?;
    if reported != expected {
        return Err(IndexError::ForeignKeysNotRestored { expected, reported });
    }
    Ok(())
}

/// `PRAGMA foreign_key_check` reports one row per violation. Counted here rather than through
/// `SELECT count(*) FROM pragma_foreign_key_check` so the check does not depend on the
/// table-valued pragma functions being compiled in.
fn count_foreign_key_violations(tx: &Transaction<'_>) -> Result<i64, IndexError> {
    let mut statement = tx.prepare("PRAGMA foreign_key_check")?;
    let mut rows = statement.query([])?;
    let mut count: i64 = 0;
    while rows.next()?.is_some() {
        count += 1;
    }
    Ok(count)
}
