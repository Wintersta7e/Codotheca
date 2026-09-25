//! §8.2's `projects.list`, §8.4's `projects.peek` and §1.2's three organisation primitives
//! behind `projects.setFlags`.
//!
//! Those are the three commands R37 found declared in the schema, called by the renderer, and
//! implemented nowhere.
//!
//! Nothing under this module reads a clock, a zone or a filesystem path: `now` and
//! `tz_offset_min` arrive on `ProjectsCtx`, and §2.5's rule means a path leaves here only as
//! `LocationRef { id, path_display }`. Nothing here is destructive — §17 gives phase 1 no
//! delete, clean, push or checkout, and `setFlags` writes three integer columns.

pub mod flags;
pub mod list;
pub mod peek;
pub mod rows;
#[cfg(feature = "testkit")]
pub mod testrow;

use crate::index::{Index, IndexError};
use crate::proto::dispatch::CommandFailure;
use crate::proto::EventSink;
use crate::protocol::ErrorCode;

// R15: `parse_args` is **not** declared here. Plan 03 declares the one command-argument helper
// beside `CommandFailure` in `core/src/proto/dispatch.rs`; each submodule imports it directly as
// `use crate::proto::dispatch::parse_args;`, so there is one path to it and not two.
// R16: `EventSink` is plan 03's trait, declared beside `Publisher`.

/// Everything a §8 command needs.
///
/// `now` is unix **seconds**; `tz_offset_min` is the machine's UTC offset in minutes, supplied by
/// the caller because §8.1's bands cut on the **local** calendar year and a core that read the
/// zone itself could not be tested across a New Year.
pub struct ProjectsCtx<'a> {
    /// The index, borrowed from the guard the dispatcher holds.
    pub index: &'a Index,
    /// Where the commands publish their events.
    pub events: &'a dyn EventSink,
    /// §6's freshness request. `projects.peek` asks for a current worktree reading; nothing
    /// else in this module does, because a shelf of a thousand rows would queue a thousand
    /// status jobs on every keystroke.
    pub jobs: &'a dyn crate::jobs::JobSink,
    /// The store class the queued job is scheduled against. Not a column (R27): it is a
    /// property of the mount right now.
    pub mounts: &'a dyn crate::mount::MountResolver,
    /// §21.5: a Peek also asks for its **remote** facts. The other of exactly two sites, and
    /// deliberately not `projects.list` — a shelf of a thousand rows must not queue a thousand
    /// network tasks.
    pub sync: &'a dyn crate::sync::runner::SyncSink,
    /// The command's reading of the wall clock, in unix seconds.
    pub now: i64,
    /// The machine's UTC offset in minutes, for §8.1's local-year bands.
    pub tz_offset_min: i32,
}

impl std::fmt::Debug for ProjectsCtx<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProjectsCtx")
            .field("now", &self.now)
            .field("tz_offset_min", &self.tz_offset_min)
            .finish_non_exhaustive()
    }
}

/// Why a §8 command could not be answered.
#[derive(Debug)]
pub enum ProjectsError {
    /// An index-level read failed, e.g. the display paths.
    Index(IndexError),
    /// A statement against the index failed.
    Sqlite(rusqlite::Error),
    /// The caller named a project that is not in the index, or one merged away.
    UnknownProject(i64),
    /// A stored enum string the schema no longer admits. Ours, not the caller's.
    BadColumn {
        /// The column (or table) that was read, usually spelled `table.column`.
        column: &'static str,
        /// What it held, as read.
        value: String,
    },
}

impl std::fmt::Display for ProjectsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Index(e) => write!(f, "index: {e:?}"),
            Self::Sqlite(e) => write!(f, "sqlite: {e}"),
            Self::UnknownProject(id) => write!(f, "no project {id}"),
            Self::BadColumn { column, value } => write!(f, "bad {column}: {value:?}"),
        }
    }
}

impl std::error::Error for ProjectsError {}

impl From<rusqlite::Error> for ProjectsError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Sqlite(e)
    }
}

impl From<IndexError> for ProjectsError {
    fn from(e: IndexError) -> Self {
        Self::Index(e)
    }
}

impl ProjectsError {
    /// The closed code the shell narrows on (§2.4). A caller's mistake is `PROTOCOL`; ours is
    /// `INTERNAL`.
    #[must_use]
    pub const fn code(&self) -> ErrorCode {
        match self {
            Self::UnknownProject(_) => ErrorCode::Protocol,
            _ => ErrorCode::Internal,
        }
    }
}

impl From<ProjectsError> for CommandFailure {
    fn from(e: ProjectsError) -> Self {
        match e.code() {
            ErrorCode::Protocol => Self::protocol(e.to_string()),
            _ => Self::internal(e.to_string()),
        }
    }
}

/// The commands this module owns, in the order the dispatcher matches them. Exposed so the
/// table can be asserted without constructing an `Index`.
#[must_use]
pub const fn dispatch_projects_command_names() -> [&'static str; 3] {
    ["projects.list", "projects.peek", "projects.setFlags"]
}

/// `None` means "this module does not own that command" — plan 21's router chains on it.
#[must_use]
pub fn dispatch_projects_command(
    ctx: &ProjectsCtx<'_>,
    command: &str,
    args: serde_json::Value,
) -> Option<Result<serde_json::Value, CommandFailure>> {
    match command {
        "projects.list" => Some(list::handle(ctx, args)),
        "projects.peek" => Some(peek::handle(ctx, args)),
        "projects.setFlags" => Some(flags::handle(ctx, args)),
        _ => None,
    }
}

#[cfg(all(test, feature = "testkit"))]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
mod tests {
    use super::*;
    use crate::art::testsupport::CollectingSink;

    #[test]
    fn dispatch_declines_a_command_this_module_does_not_own() {
        let dir = tempfile::tempdir().expect("tempdir");
        let index = Index::open(dir.path()).expect("open");
        let events = CollectingSink::default();
        let jobs = crate::jobs::NullJobSink;
        let mounts = crate::testing::FakeMountResolver::default();
        let sync = crate::sync::runner::NullSyncSink;
        let ctx = ProjectsCtx {
            index: &index,
            events: &events,
            jobs: &jobs,
            mounts: &mounts,
            sync: &sync,
            now: 1_700_000_000,
            tz_offset_min: 0,
        };
        // Plan 14's, plan 08's, plan 10b's. `None` is how plan 21's router learns to keep going.
        assert!(dispatch_projects_command(&ctx, "projects.get", serde_json::json!({})).is_none());
        assert!(dispatch_projects_command(&ctx, "projects.merge", serde_json::json!({})).is_none());
        assert!(dispatch_projects_command(&ctx, "art.url", serde_json::json!({})).is_none());
        assert!(dispatch_projects_command(&ctx, "", serde_json::json!({})).is_none());
    }

    #[test]
    fn the_owned_names_are_exactly_r37s_row_for_this_plan() {
        assert_eq!(
            dispatch_projects_command_names(),
            ["projects.list", "projects.peek", "projects.setFlags"]
        );
        assert!(dispatch_projects_command_names()
            .iter()
            .all(|n| n.starts_with("projects.")));
    }

    #[test]
    fn an_unknown_project_is_a_protocol_failure_not_an_internal_one() {
        // §2.4: the core's `message` is diagnostic and never shown raw; the code is what the
        // shell narrows on, and a caller naming a project that is gone is not a core fault.
        assert_eq!(ProjectsError::UnknownProject(7).code(), ErrorCode::Protocol);
        assert_eq!(
            ProjectsError::BadColumn {
                column: "presence",
                value: "x".into()
            }
            .code(),
            ErrorCode::Internal
        );
    }
}
