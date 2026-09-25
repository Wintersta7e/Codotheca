//! Launch targets: discovery, ranking, resolution, verification and the spawn.

pub mod argv;
pub mod catalogue;
pub mod detect;
pub mod probe;
#[cfg(not(windows))]
pub mod probe_linux;
// Declared unconditionally on purpose: the Shell Link and shell-verb parsers are platform-free,
// so they compile and are tested everywhere while only the registry walk is `#[cfg(windows)]`.
#[cfg_attr(not(windows), allow(dead_code))]
pub mod probe_windows;
pub mod rank;
pub mod recents;
pub mod resolve;
pub mod spawn;
pub mod verify;
pub mod wslpath;

pub use catalogue::{CwdMode, TargetKind};

// R3: no `now_secs` helper. `Clock::now_unix()` is already epoch seconds, which is what every
// `session`, `session_segment` and `launch_target` column stores. Call it directly.

/// The closed set of failures the launch subsystem can produce.
#[derive(Debug, thiserror::Error)]
pub enum LaunchError {
    /// SQLite refused a statement.
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),

    /// The index layer failed beneath a statement.
    #[error("index: {0}")]
    Index(#[from] crate::index::IndexError),

    /// An internal fault carried as text — in practice, a stored column holding a value outside
    /// its enum.
    #[error("io: {0}")]
    Io(String),

    /// No `launch_target` row has this id.
    #[error("no launch target {0}")]
    NoSuchTarget(i64),

    /// No `location` row has this id.
    #[error("no location {0}")]
    NoSuchLocation(i64),

    /// The location with this id is not `present`, so there is nothing to launch in.
    #[error("location {0} is not present")]
    LocationNotPresent(i64),

    /// The target's executable is not a file the OS would run.
    #[error("{exec_display} is not an executable file")]
    NotExecutable {
        /// The executable's path in display form, for the log.
        exec_display: String,
    },

    /// The OS refused to start the process.
    #[error("could not start {exec_display}: {detail}")]
    Spawn {
        /// The executable's path in display form, for the log.
        exec_display: String,
        /// The OS's own error text.
        detail: String,
    },
}

impl LaunchError {
    /// The wire code §11.5's failure window keys its copy off. The message above is a
    /// diagnostic for the log; it is never rendered raw.
    #[must_use]
    pub const fn code(&self) -> crate::protocol::ErrorCode {
        use crate::protocol::ErrorCode;
        match self {
            Self::NoSuchTarget(_) | Self::NoSuchLocation(_) => ErrorCode::Protocol,
            Self::LocationNotPresent(_) => ErrorCode::StoreOffline,
            Self::NotExecutable { .. } => ErrorCode::PermissionDenied,
            Self::Spawn { .. } => ErrorCode::PathGone,
            Self::Sqlite(_) | Self::Index(_) | Self::Io(_) => ErrorCode::Internal,
        }
    }
}
