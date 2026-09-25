//! Sessions and playtime (§9).
//!
//! **Two ledgers, never merged.** Playtime counts only what this app launched. Nothing in this
//! module reads a git-derived table, joins one, or sums across the two; a test in
//! `core/tests/session_credit.rs` asserts that over the source of every file here.
//!
//! **`credited_seconds` is always the sum of segment credits** (§9), whatever ends the session.
//! The end mechanism decides *when* the session closes, never *what* it credits.

pub mod activity;
pub mod manager;
pub mod orphan;
pub mod segment;
pub mod store;
pub mod watch;

pub use crate::protocol::{CloseReason, ClosedBy};

/// §9: a segment closes after 20 minutes with no in-scope worktree change **and** no focus on
/// this project's own view. Shelf browsing is not focus on this project's view.
pub const SEGMENT_IDLE_SECS: i64 = 1_200;
/// Derived, never written twice: idle is decided on the monotonic clock (R3) and recorded in
/// seconds. `unsigned_abs` is a `const fn` and is not an `as` cast, which pedantic denies.
pub const SEGMENT_IDLE_MS: u64 = SEGMENT_IDLE_SECS.unsigned_abs() * 1_000;

/// §9: without wait mode, the session ends when no new segment opens within 60 minutes.
pub const SESSION_IDLE_SECS: i64 = 3_600;
/// The monotonic form of [`SESSION_IDLE_SECS`], derived rather than restated.
pub const SESSION_IDLE_MS: u64 = SESSION_IDLE_SECS.unsigned_abs() * 1_000;

/// After this long with no `session.focus` report, a held focus stops extending a segment. A
/// renderer that died holding focus would otherwise hold one open indefinitely — the farming
/// hole from the other end.
pub const FOCUS_STALE_SECS: i64 = 120;
/// The monotonic form of [`FOCUS_STALE_SECS`], derived rather than restated.
pub const FOCUS_STALE_MS: u64 = FOCUS_STALE_SECS.unsigned_abs() * 1_000;

/// How often `SessionManager::tick` is pumped. It bounds three things at once: idle-close
/// precision, wait-mode exit latency, and how much credit a crash can lose (§9's watermark).
pub const DEFAULT_TICK_SECS: u64 = 15;

// The two relations the three constants above have to hold, asserted at compile time rather
// than in a test: a build that breaks either one cannot exist. A dead renderer must cost less
// than a segment, and the tick has to be finer than the staleness it is there to notice.
const _: () = assert!(FOCUS_STALE_SECS * 2 < SEGMENT_IDLE_SECS);
const _: () = assert!(DEFAULT_TICK_SECS < FOCUS_STALE_SECS.unsigned_abs());

/// The `close_reason` column spelling of a generated variant (§1.6's CHECK).
#[must_use]
pub fn close_reason_str(reason: CloseReason) -> &'static str {
    match reason {
        CloseReason::Stop => "stop",
        CloseReason::Idle => "idle",
        CloseReason::ProcessExit => "process_exit",
        CloseReason::AppExit => "app_exit",
        CloseReason::Crash => "crash",
        CloseReason::Orphaned => "orphaned",
    }
}

/// The inverse of [`close_reason_str`]. `None` for anything outside §1.6's closed set, so a
/// column holding an unknown value becomes an error rather than a silent default.
#[must_use]
pub fn close_reason_from_str(text: &str) -> Option<CloseReason> {
    match text {
        "stop" => Some(CloseReason::Stop),
        "idle" => Some(CloseReason::Idle),
        "process_exit" => Some(CloseReason::ProcessExit),
        "app_exit" => Some(CloseReason::AppExit),
        "crash" => Some(CloseReason::Crash),
        "orphaned" => Some(CloseReason::Orphaned),
        _ => None,
    }
}

/// The `closed_by` column spelling of a generated variant (§1.6's CHECK).
#[must_use]
pub fn closed_by_str(closed_by: ClosedBy) -> &'static str {
    match closed_by {
        ClosedBy::Idle => "idle",
        ClosedBy::SessionEnd => "session_end",
        ClosedBy::AppExit => "app_exit",
        ClosedBy::Crash => "crash",
    }
}

/// The inverse of [`closed_by_str`].
#[must_use]
pub fn closed_by_from_str(text: &str) -> Option<ClosedBy> {
    match text {
        "idle" => Some(ClosedBy::Idle),
        "session_end" => Some(ClosedBy::SessionEnd),
        "app_exit" => Some(ClosedBy::AppExit),
        "crash" => Some(ClosedBy::Crash),
        _ => None,
    }
}

/// The closed set of failures the session subsystem can produce.
#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("index: {0}")]
    Index(#[from] crate::index::IndexError),

    #[error("git: {0}")]
    Git(crate::git::GitError),

    /// The recursive watcher could not be created or could not watch a root.
    #[error("watch: {0}")]
    Watch(String),

    #[error("no session {0}")]
    NoSuchSession(i64),

    /// A stored enum column held a value the closed set does not contain.
    #[error("column {column} holds {value}, which is not a value of its enum")]
    BadColumn { column: &'static str, value: String },
}

impl SessionError {
    /// The wire code §11.5's failure window keys its copy off. The message above is a
    /// diagnostic for the log; it is never rendered raw (§2.4).
    #[must_use]
    pub fn code(&self) -> crate::protocol::ErrorCode {
        use crate::protocol::ErrorCode;
        match self {
            // A caller naming a session that is not there, or a column outside its own enum,
            // is a contract breach rather than an environment failure.
            Self::NoSuchSession(_) | Self::BadColumn { .. } => ErrorCode::Protocol,
            // `protocol_code` returns the schema's spelling; there is no generated parser for
            // `ErrorCode`, so the arms are written out. `None` is a scheduler state (busy, torn
            // read, cancelled), which is not a project error the shell renders.
            Self::Git(err) => match err.protocol_code() {
                Some("GIT_MISSING") => ErrorCode::GitMissing,
                Some("GIT_TOO_OLD") => ErrorCode::GitTooOld,
                Some("UNTRUSTED_REPO") => ErrorCode::UntrustedRepo,
                Some("PERMISSION_DENIED") => ErrorCode::PermissionDenied,
                Some("PATH_GONE") => ErrorCode::PathGone,
                Some("STORE_OFFLINE") => ErrorCode::StoreOffline,
                Some("REPO_UNREADABLE") => ErrorCode::RepoUnreadable,
                Some("BUDGET_EXCEEDED") => ErrorCode::BudgetExceeded,
                _ => ErrorCode::Internal,
            },
            Self::Sqlite(_) | Self::Index(_) | Self::Watch(_) => ErrorCode::Internal,
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use super::*;

    const CLOSE_REASONS: [CloseReason; 6] = [
        CloseReason::Stop,
        CloseReason::Idle,
        CloseReason::ProcessExit,
        CloseReason::AppExit,
        CloseReason::Crash,
        CloseReason::Orphaned,
    ];

    const CLOSED_BYS: [ClosedBy; 4] = [
        ClosedBy::Idle,
        ClosedBy::SessionEnd,
        ClosedBy::AppExit,
        ClosedBy::Crash,
    ];

    #[test]
    fn every_close_reason_round_trips_through_the_string_the_ddl_checks() {
        // §1.6: close_reason TEXT NULL in {stop,idle,process_exit,app_exit,crash,orphaned}
        for (reason, text) in [
            (CloseReason::Stop, "stop"),
            (CloseReason::Idle, "idle"),
            (CloseReason::ProcessExit, "process_exit"),
            (CloseReason::AppExit, "app_exit"),
            (CloseReason::Crash, "crash"),
            (CloseReason::Orphaned, "orphaned"),
        ] {
            assert_eq!(close_reason_str(reason), text);
            assert_eq!(close_reason_from_str(text), Some(reason));
        }
        assert_eq!(
            close_reason_from_str("offline"),
            None,
            "a location going offline is not a value of the enum"
        );
    }

    #[test]
    fn every_closed_by_round_trips_and_the_set_is_exactly_the_four() {
        // §1.6: closed_by TEXT NULL in {idle,session_end,app_exit,crash}
        for (by, text) in [
            (ClosedBy::Idle, "idle"),
            (ClosedBy::SessionEnd, "session_end"),
            (ClosedBy::AppExit, "app_exit"),
            (ClosedBy::Crash, "crash"),
        ] {
            assert_eq!(closed_by_str(by), text);
            assert_eq!(closed_by_from_str(text), Some(by));
        }
        assert_eq!(
            closed_by_from_str("orphaned"),
            None,
            "that is a close_reason, not a closed_by"
        );
    }

    #[test]
    fn the_two_spellings_are_inverse_functions() {
        for reason in CLOSE_REASONS {
            assert_eq!(
                close_reason_from_str(close_reason_str(reason)),
                Some(reason)
            );
        }
        for by in CLOSED_BYS {
            assert_eq!(closed_by_from_str(closed_by_str(by)), Some(by));
        }
    }

    #[test]
    fn the_millisecond_forms_are_derived_from_the_second_forms_and_cannot_drift() {
        assert_eq!(SEGMENT_IDLE_MS, 1_200_000);
        assert_eq!(SESSION_IDLE_MS, 3_600_000);
        assert_eq!(FOCUS_STALE_MS, 120_000);
    }

    // `a_dead_renderer_costs_less_than_a_segment` is the pair of `const _: () = assert!(..)`
    // beside the constants themselves, not a test: both sides are constants, so the relation is
    // a compile-time fact and a build that breaks it cannot be produced.

    #[test]
    fn every_error_maps_onto_the_closed_error_enum() {
        use crate::protocol::ErrorCode;
        assert_eq!(SessionError::NoSuchSession(7).code(), ErrorCode::Protocol);
        assert_eq!(
            SessionError::Watch("gone".to_owned()).code(),
            ErrorCode::Internal
        );
        assert_eq!(
            SessionError::BadColumn {
                column: "close_reason",
                value: "offline".to_owned(),
            }
            .code(),
            ErrorCode::Protocol
        );
        assert_eq!(
            SessionError::Git(crate::git::GitError::Missing).code(),
            ErrorCode::GitMissing
        );
    }
}
