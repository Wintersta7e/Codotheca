//! The closed set of failures that can occur while owning the index.

use std::path::PathBuf;

/// Why an operation on the index — opening it, migrating it, or reading and writing through the
/// one connection — could not complete.
#[derive(Debug, thiserror::Error)]
pub enum IndexError {
    /// SQLite refused an operation for a reason no other variant names.
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),

    /// A filesystem operation around the database failed — creating the data directory, writing
    /// or restoring a backup, quarantining a file, or writing a JSON document.
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    /// Another process holds the database's exclusive lock, or it would not enter WAL mode.
    #[error("the index at {path} is already open in another process")]
    AlreadyOpen {
        /// The database file whose lock could not be taken.
        path: PathBuf,
    },

    /// The database was written by a newer build; §1.12 refuses it rather than open and write it.
    #[error("index schema {on_disk} is newer than this build's {supported}")]
    SchemaFromFuture {
        /// `PRAGMA user_version` as found on disk.
        on_disk: u32,
        /// The highest version this build's migration chain reaches.
        supported: u32,
    },

    /// A migration run failed, and the pre-migration backup, when there was one, was put back.
    #[error("migration {version} ({name}) failed; restored to schema {restored_to}")]
    MigrationFailed {
        /// The first migration the run attempted — the lowest above the version on disk.
        version: u32,
        /// That migration's registered name, or `unknown` when none was pending.
        name: &'static str,
        /// The version on disk before the run, which the restored backup holds.
        restored_to: u32,
        /// When the open that failed ran, in Unix seconds.
        restored_at: i64,
        /// The underlying failure, as text.
        detail: String,
    },

    /// The file is not a readable database (`SQLITE_NOTADB`, `SQLITE_CORRUPT`), or a stored
    /// value is one this build cannot read.
    #[error("the index is not a readable database: {detail}")]
    Corrupt {
        /// What could not be read, as text.
        detail: String,
    },

    /// `PRAGMA user_version` and `app_meta.schema_version` disagree, so something other than this
    /// program wrote one of them (§1.9, §1.12).
    #[error("schema version mirror disagrees: user_version {pragma}, app_meta {meta}")]
    VersionMirrorMismatch {
        /// `PRAGMA user_version` as found on disk, before any migration ran.
        pragma: u32,
        /// The version `app_meta` mirrors.
        meta: u32,
    },

    /// A computed completion with zero applicable checks, or more lit than applicable — unknown
    /// wearing a number, refused rather than written.
    #[error("completion is not computable with zero applicable checks")]
    CompletionNotComputable,

    /// A rebuilding migration asked for `PRAGMA foreign_keys=OFF` and the connection read back
    /// something else, so nothing is applied. R59: the pragma is a documented no-op inside a
    /// transaction, and a pragma that silently did nothing is indistinguishable from one that
    /// worked — which is the whole failure this read-back exists to catch.
    #[error("foreign keys could not be disabled for a rebuilding migration; read back {reported}")]
    ForeignKeysNotDisabled {
        /// What `PRAGMA foreign_keys` read back instead of `0`.
        reported: i64,
    },

    /// The rebuild ran and `PRAGMA foreign_key_check` returned rows, so the transaction is
    /// dropped and rolled back. The check cannot live in the `.sql` file: it returns rows and
    /// never errors, and `execute_batch` discards them.
    #[error("a rebuilding migration left {count} foreign-key violation(s); rolled back")]
    ForeignKeyViolations {
        /// How many rows `PRAGMA foreign_key_check` returned.
        count: i64,
    },

    /// The pragma the rebuild turned off would not go back to the value the connection started
    /// with. Every later write on that connection would skip referential integrity silently.
    #[error("foreign keys could not be restored to {expected}; read back {reported}")]
    ForeignKeysNotRestored {
        /// The `foreign_keys` value the connection started with.
        expected: i64,
        /// What the pragma read back after the restore.
        reported: i64,
    },

    /// The registered migration chain is not `1..=n`: a gap or a repeat. `apply_all` skips any
    /// migration with `version <= current`, so a database that passes through a gap skips the
    /// missing file **forever**, silently, on the user's machine.
    #[error("migration chain is broken: expected version {expected}, found {found}")]
    MigrationChainBroken {
        /// The version the entry at this position should carry.
        expected: u32,
        /// The version it carries.
        found: u32,
    },

    /// A guard that validated an empty list is a failing guard.
    #[error("migration chain is empty; there is nothing to apply and nothing was checked")]
    MigrationChainEmpty,

    /// A JSON value — the §1.12 sidecar, a diagnostics bundle, or a setting or view-state value
    /// stored as JSON text — failed to serialise, parse or validate.
    #[error("sidecar: {0}")]
    Sidecar(String),
}
