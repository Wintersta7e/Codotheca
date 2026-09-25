//! Failure classification (§3.5, §11.1).
//!
//! The rule this file exists to enforce: a read that failed is **not** a repository that is
//! gone. Exactly one variant, [`GitError::PathGone`], may set `presence = 'missing'`; every
//! Windows sharing violation, access denial and cloud-placeholder failure lands on
//! [`GitError::Stale`] instead.

/// The result type every reader in this module returns.
pub type GitResult<T> = Result<T, GitError>;

/// Which lock or operation marker made a repository unreadable right now.
///
/// Distinct from `interrupted_op` (§1.3), which stores only `merge` and `rebase` because §3.3
/// reads only `MERGE_HEAD` and `REBASE_HEAD`. This enum is wider because §3.5 defers on any
/// operation marker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BusyMarker {
    /// `index.lock` held by another git process.
    IndexLock,
    /// `MERGE_HEAD` present.
    Merge,
    /// `REBASE_HEAD`, `rebase-merge/` or `rebase-apply/` present.
    Rebase,
    /// `CHERRY_PICK_HEAD` present.
    CherryPick,
    /// `BISECT_LOG` present.
    Bisect,
    /// `REVERT_HEAD` present.
    Revert,
}

/// Everything the git layer can fail with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitError {
    /// No git binary.
    Missing,
    /// Below [`crate::git::GIT_FLOOR`]; carries the raw version line for the upgrade hint.
    TooOld {
        /// The `git --version` line as printed.
        found: String,
    },
    /// Dubious ownership. Resolved by `location.trusted_at` plus `-c safe.directory=<path>`.
    Untrusted {
        /// Lossy display form of the path git refused, diagnostic only.
        path: String,
    },
    /// The filesystem refused the read on a Unix-style permission error.
    PermissionDenied {
        /// Diagnostic detail; never rendered raw.
        detail: String,
    },
    /// The path is genuinely not there. **The only variant that implies absence.**
    PathGone {
        /// Diagnostic detail; never rendered raw.
        detail: String,
    },
    /// The backing store answered as unmounted or unreachable.
    StoreOffline {
        /// Diagnostic detail; never rendered raw.
        detail: String,
    },
    /// It looks like a repository and git could not open it.
    Unreadable {
        /// Diagnostic detail; never rendered raw.
        detail: String,
    },
    /// The read failed in a way that means *unknown*: sharing violation, access denial or an
    /// unhydrated cloud placeholder (§3.5). Never absence.
    Stale {
        /// Diagnostic detail; never rendered raw.
        detail: String,
    },
    /// A lock or operation marker is in the way; the caller defers (§4.1 `deferred_slow`).
    Busy {
        /// Which marker.
        marker: BusyMarker,
    },
    /// The ref state moved between the two fingerprints of one observation (§3.5).
    TornRead,
    /// The job's deadline elapsed and the process tree was killed.
    Budget {
        /// How long the child ran before it was killed.
        after_ms: u64,
    },
    /// A cancellation token fired.
    Cancelled,
    /// A defect in this program.
    Internal {
        /// Diagnostic detail; never rendered raw.
        detail: String,
    },
}

impl GitError {
    /// The closed-enum code from `protocol/schema/protocol.json`, or `None` when the failure is
    /// a scheduler state rather than a project error the shell renders (§11.1).
    #[must_use]
    pub const fn protocol_code(&self) -> Option<&'static str> {
        match self {
            Self::Missing => Some("GIT_MISSING"),
            Self::TooOld { .. } => Some("GIT_TOO_OLD"),
            Self::Untrusted { .. } => Some("UNTRUSTED_REPO"),
            Self::PermissionDenied { .. } => Some("PERMISSION_DENIED"),
            Self::PathGone { .. } => Some("PATH_GONE"),
            Self::StoreOffline { .. } => Some("STORE_OFFLINE"),
            Self::Unreadable { .. } | Self::Stale { .. } => Some("REPO_UNREADABLE"),
            Self::Budget { .. } => Some("BUDGET_EXCEEDED"),
            Self::Internal { .. } => Some("INTERNAL"),
            Self::Busy { .. } | Self::TornRead | Self::Cancelled => None,
        }
    }

    /// True only for the one failure that means the path is not there. A caller may set
    /// `presence = 'missing'` **only** when this is true (§3.5, §4.6).
    #[must_use]
    pub const fn implies_absent(&self) -> bool {
        matches!(self, Self::PathGone { .. })
    }

    /// True when the right response is to requeue rather than to record an error (§4.1).
    #[must_use]
    pub const fn is_deferral(&self) -> bool {
        matches!(self, Self::Busy { .. } | Self::TornRead | Self::Cancelled)
    }
}

impl std::fmt::Display for GitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing => write!(f, "git is not on PATH"),
            Self::TooOld { found } => write!(f, "git is below the floor: {found}"),
            Self::Untrusted { path } => write!(f, "dubious ownership: {path}"),
            Self::PermissionDenied { detail } => write!(f, "permission denied: {detail}"),
            Self::PathGone { detail } => write!(f, "path gone: {detail}"),
            Self::StoreOffline { detail } => write!(f, "store offline: {detail}"),
            Self::Unreadable { detail } => write!(f, "unreadable: {detail}"),
            Self::Stale { detail } => write!(f, "stale read: {detail}"),
            Self::Busy { marker } => write!(f, "busy: {marker:?}"),
            Self::TornRead => write!(f, "torn read"),
            Self::Budget { after_ms } => write!(f, "budget exceeded after {after_ms} ms"),
            Self::Cancelled => write!(f, "cancelled"),
            Self::Internal { detail } => write!(f, "internal: {detail}"),
        }
    }
}

impl std::error::Error for GitError {}

/// `core::cancel` sits below `core::git` and cannot name `GitError`, so it returns its own
/// error type; this keeps every `cancel.check()?` inside a `GitResult` function compiling.
impl From<crate::cancel::Cancelled> for GitError {
    fn from(_: crate::cancel::Cancelled) -> Self {
        Self::Cancelled
    }
}

/// Classify a spawn failure.
#[must_use]
pub fn classify_spawn(err: &std::io::Error) -> GitError {
    match err.kind() {
        std::io::ErrorKind::NotFound => GitError::Missing,
        std::io::ErrorKind::PermissionDenied => GitError::PermissionDenied {
            detail: err.to_string(),
        },
        _ => GitError::Internal {
            detail: err.to_string(),
        },
    }
}

/// Classify a non-zero exit from git's stderr.
///
/// Matching is on git's own English message text, which is why every invocation sets `LC_ALL=C`
/// (§3.2's environment, `invocation::neutralise_env`). Order matters: the trust and lock cases
/// are checked before the generic access-denied case, because their messages contain it.
#[must_use]
pub fn classify(exit_code: i32, stderr: &[u8]) -> GitError {
    let _ = exit_code;
    let text = String::from_utf8_lossy(stderr);
    let low = text.to_ascii_lowercase();
    let detail = text.trim().to_owned();
    let has = |needle: &str| low.contains(needle);

    if has("dubious ownership") {
        let path = low
            .split_once("repository at '")
            .and_then(|(_, rest)| rest.split_once('\''))
            .map_or_else(String::new, |(p, _)| p.to_owned());
        return GitError::Untrusted { path };
    }
    if has("index.lock") || has("another git process seems to be running") {
        return GitError::Busy {
            marker: BusyMarker::IndexLock,
        };
    }
    // §3.5's Windows rule. Every string below means *unknown*, never *gone*.
    if has("being used by another process")      // ERROR_SHARING_VIOLATION
        || has("access is denied")               // ERROR_ACCESS_DENIED
        || has("the process cannot access")      // ERROR_LOCK_VIOLATION
        || has("cloud file provider")            // ERROR_CLOUD_FILE_PROVIDER_NOT_RUNNING
        || has("cloud operation")                // the ERROR_CLOUD_FILE_* block
        || has("not available on this computer") // an unhydrated placeholder
        || has("cannot access the file")
    {
        return GitError::Stale { detail };
    }
    if has("network name is no longer available")
        || has("network path was not found")
        || has("host is down")
        || has("transport endpoint is not connected")
        || has("input/output error")
    {
        return GitError::StoreOffline { detail };
    }
    if has("no such file or directory")
        || has("cannot change to")
        || has("the system cannot find the path")
    {
        return GitError::PathGone { detail };
    }
    if has("permission denied") || has("operation not permitted") {
        return GitError::PermissionDenied { detail };
    }
    GitError::Unreadable { detail }
}
