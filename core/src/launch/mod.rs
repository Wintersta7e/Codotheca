//! Launch targets: discovery, ranking, resolution, verification and the spawn.

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
pub mod wslpath;

pub use catalogue::{CwdMode, TargetKind};

// R3: no `now_secs` helper. `Clock::now_unix()` is already epoch seconds, which is what every
// `session`, `session_segment` and `launch_target` column stores. Call it directly.

/// The closed set of failures the launch subsystem can produce.
#[derive(Debug, thiserror::Error)]
pub enum LaunchError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("index: {0}")]
    Index(#[from] crate::index::IndexError),

    #[error("io: {0}")]
    Io(String),

    #[error("no launch target {0}")]
    NoSuchTarget(i64),

    #[error("no location {0}")]
    NoSuchLocation(i64),

    #[error("location {0} is not present")]
    LocationNotPresent(i64),

    #[error("{exec_display} is not an executable file")]
    NotExecutable { exec_display: String },

    #[error("could not start {exec_display}: {detail}")]
    Spawn {
        exec_display: String,
        detail: String,
    },
}

impl LaunchError {
    /// The wire code §11.5's failure window keys its copy off. The message above is a
    /// diagnostic for the log; it is never rendered raw.
    #[must_use]
    pub fn code(&self) -> crate::protocol::ErrorCode {
        use crate::protocol::ErrorCode;
        match self {
            LaunchError::NoSuchTarget(_) | LaunchError::NoSuchLocation(_) => ErrorCode::Protocol,
            LaunchError::LocationNotPresent(_) => ErrorCode::StoreOffline,
            LaunchError::NotExecutable { .. } => ErrorCode::PermissionDenied,
            LaunchError::Spawn { .. } => ErrorCode::PathGone,
            LaunchError::Sqlite(_) | LaunchError::Index(_) | LaunchError::Io(_) => {
                ErrorCode::Internal
            }
        }
    }
}
