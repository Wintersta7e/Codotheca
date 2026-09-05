//! The closed set of failures that can occur while owning the index.

use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum IndexError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("the index at {path} is already open in another process")]
    AlreadyOpen { path: PathBuf },

    #[error("index schema {on_disk} is newer than this build's {supported}")]
    SchemaFromFuture { on_disk: u32, supported: u32 },

    #[error("migration {version} ({name}) failed; restored to schema {restored_to}")]
    MigrationFailed {
        version: u32,
        name: &'static str,
        restored_to: u32,
        restored_at: i64,
        detail: String,
    },

    #[error("the index is not a readable database: {detail}")]
    Corrupt { detail: String },

    #[error("schema version mirror disagrees: user_version {pragma}, app_meta {meta}")]
    VersionMirrorMismatch { pragma: u32, meta: u32 },

    #[error("completion is not computable with zero applicable checks")]
    CompletionNotComputable,

    /// A rebuilding migration asked for `PRAGMA foreign_keys=OFF` and the connection read back
    /// something else, so nothing is applied. R59: the pragma is a documented no-op inside a
    /// transaction, and a pragma that silently did nothing is indistinguishable from one that
    /// worked — which is the whole failure this read-back exists to catch.
    #[error("foreign keys could not be disabled for a rebuilding migration; read back {reported}")]
    ForeignKeysNotDisabled { reported: i64 },

    /// The rebuild ran and `PRAGMA foreign_key_check` returned rows, so the transaction is
    /// dropped and rolled back. The check cannot live in the `.sql` file: it returns rows and
    /// never errors, and `execute_batch` discards them.
    #[error("a rebuilding migration left {count} foreign-key violation(s); rolled back")]
    ForeignKeyViolations { count: i64 },

    /// The pragma the rebuild turned off would not go back to the value the connection started
    /// with. Every later write on that connection would skip referential integrity silently.
    #[error("foreign keys could not be restored to {expected}; read back {reported}")]
    ForeignKeysNotRestored { expected: i64, reported: i64 },

    /// The registered migration chain is not `1..=n`: a gap or a repeat. `apply_all` skips any
    /// migration with `version <= current`, so a database that passes through a gap skips the
    /// missing file **forever**, silently, on the user's machine.
    #[error("migration chain is broken: expected version {expected}, found {found}")]
    MigrationChainBroken { expected: u32, found: u32 },

    /// A guard that validated an empty list is a failing guard.
    #[error("migration chain is empty; there is nothing to apply and nothing was checked")]
    MigrationChainEmpty,

    #[error("sidecar: {0}")]
    Sidecar(String),
}
