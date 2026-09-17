//! Install's durable run-state vocabulary, and where a clone lands.

pub mod destination;

pub use destination::{is_safe_path_segment, refuse_unsafe_name};

/// The lifecycle state stored in `install_run.state`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallRunState {
    /// The clone is still in progress.
    Running,
    /// The clone and location commit completed.
    Done,
    /// The run ended unsuccessfully.
    Failed,
    /// The user cancelled the run.
    Cancelled,
}

impl InstallRunState {
    /// Every stored state, so the Rust vocabulary can be checked against the database column.
    pub const ALL: [InstallRunState; 4] = [
        InstallRunState::Running,
        InstallRunState::Done,
        InstallRunState::Failed,
        InstallRunState::Cancelled,
    ];

    /// The value stored in `install_run.state`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            InstallRunState::Running => "running",
            InstallRunState::Done => "done",
            InstallRunState::Failed => "failed",
            InstallRunState::Cancelled => "cancelled",
        }
    }
}
