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

    #[error("sidecar: {0}")]
    Sidecar(String),
}
