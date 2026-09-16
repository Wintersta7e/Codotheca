//! §25.5 — the README the project page renders, and the assets it is allowed to show.
//!
//! **The panel needs a document and the index holds a paragraph.** `j6_content::persist` stores
//! `first_paragraph(readme_excerpt)` in `peek_cache`, which is what `ReadmeState.text` carries;
//! the ~256 KB J6 read is discarded. A markup renderer fed that renders one paragraph in a frame,
//! so [`source`] reads the file again — under J6's own cap, read from J6's own constant — and the
//! wire gains `truncated` rather than a second cap.
//!
//! **Nothing here renders and nothing here parses.** The core answers bytes and states; the
//! sanitiser, the frame and every string live in the renderer. What the core owns is the part a
//! sandbox cannot help with: which file was read, whether it was cut, whether an image reference
//! is inside the location root, and whether a remote host may be reached at all.

pub mod assets;
pub mod consent;
pub mod fetch;
pub mod source;

use crate::index::Index;
use crate::proto::dispatch::CommandFailure;
use crate::proto::pubsub::EventSink;
use crate::protocol::ErrorCode;

/// Everything §25.5's index-only commands need. `now` is unix **seconds**, supplied by the
/// caller so no handler reads the clock itself.
pub struct ReadmeCtx<'a> {
    pub index: &'a Index,
    pub events: &'a dyn EventSink,
    pub now: i64,
}

/// Written by hand: `&dyn EventSink` does not require `Debug`, and a derived one would demand it
/// of every implementor.
impl std::fmt::Debug for ReadmeCtx<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReadmeCtx")
            .field("now", &self.now)
            .finish_non_exhaustive()
    }
}

/// The commands this module owns, as data, so the seam and the router cannot drift apart — the
/// shape of the defect R37 found between the schema and the handlers.
///
/// **It is read, not decorative.** [`dispatch_readme_command`] refuses anything outside it before
/// it matches, and `core/tests/readme_source.rs` compares it against the commands
/// `crate::assembly::route` actually sends here. A constant that nothing reads is a comment
/// claiming a guarantee it does not provide (R90).
pub const README_COMMANDS: [&str; 3] = [
    "projects.readme",
    "projects.readmeAssets",
    "projects.setReadmeRemote",
];

/// What a README read can refuse with, and the closed code each maps to.
///
/// `Unreadable` is **not** `absent`: a working directory that cannot be listed says nothing
/// about whether a README is in it, and `absent` is a positive claim about the user's disk.
#[derive(Debug)]
pub enum ReadmeError {
    /// The id names no live project, or the location is not that project's.
    UnknownSubject,
    /// The location's working directory could not be listed or opened.
    Unreadable(std::io::Error),
    Sqlite(rusqlite::Error),
}

impl std::fmt::Display for ReadmeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownSubject => write!(f, "no such project and location"),
            Self::Unreadable(e) => write!(f, "the working directory could not be read: {e}"),
            Self::Sqlite(e) => write!(f, "sqlite: {e}"),
        }
    }
}

impl std::error::Error for ReadmeError {}

impl From<rusqlite::Error> for ReadmeError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Sqlite(e)
    }
}

impl ReadmeError {
    /// The closed code the shell narrows on (§2.4).
    #[must_use]
    pub fn code(&self) -> ErrorCode {
        match self {
            Self::UnknownSubject => ErrorCode::Protocol,
            Self::Unreadable(_) => ErrorCode::RepoUnreadable,
            Self::Sqlite(_) => ErrorCode::Internal,
        }
    }
}

impl From<ReadmeError> for CommandFailure {
    /// `outcome: None` for every variant: a read that refused took no effect, and
    /// `projects.setReadmeRemote`'s write either commits or does not.
    fn from(e: ReadmeError) -> Self {
        Self {
            code: e.code(),
            message: e.to_string(),
            outcome: None,
        }
    }
}

/// `projects.readmeAssets`, answered **without** the index guard (R75).
///
/// **R94, clause 2 — this is the side that MUST lock**, because nothing above it holds the
/// guard: `Route::ReadmeNet` reaches it from `handle` before the lock is taken. It locks once,
/// reads the two values the read needs, and releases before the first socket. The remote branch
/// makes up to `ASSET_COUNT_CAP` calls of `REMOTE_TIMEOUT_SECS` each, and holding the process's
/// one SQLite mutex across them would stop every other command for as long as two minutes.
///
/// # Errors
/// `PROTOCOL` for an argument shape the schema does not admit or a pair that names no live
/// location; `INTERNAL` for an index fault.
pub fn handle_readme_assets_off_lock(
    index: &std::sync::Mutex<Index>,
    http: &dyn crate::http::HttpTransport,
    resolve: fetch::HostResolver,
    args: serde_json::Value,
    now: i64,
) -> Result<serde_json::Value, CommandFailure> {
    let args: crate::protocol::ProjectsReadmeAssetsArgs = crate::proto::dispatch::parse_args(args)?;
    let (work_dir, consent) = {
        let guard = index
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let work_dir = source::work_dir_of(guard.conn(), args.project_id, args.location_id)?;
        let consent = consent::readme_remote_at(guard.conn(), args.project_id)?;
        (work_dir, consent)
    };

    let deps = assets::AssetDeps {
        work_dir,
        consent,
        http,
        resolve,
        now,
    };
    let rows = assets::read_readme_assets(&deps, &args.local, &args.remote);
    serde_json::to_value(rows).map_err(|e| CommandFailure::internal(e.to_string()))
}

/// `None` means "this module does not own that command", which is what the router chains on.
#[must_use]
pub fn dispatch_readme_command(
    ctx: &ReadmeCtx<'_>,
    command: &str,
    args: serde_json::Value,
) -> Option<Result<serde_json::Value, CommandFailure>> {
    // The census decides ownership; the match below only decides which handler. Two lists that
    // could disagree are one list too many.
    if !README_COMMANDS.contains(&command) {
        return None;
    }
    match command {
        "projects.readme" => Some(source::handle_readme(ctx, args)),
        "projects.setReadmeRemote" => Some(consent::handle_set_readme_remote(ctx, args)),
        // `projects.readmeAssets` is in the census and is answered off the index guard, so it
        // never reaches this dispatcher — `Route::ReadmeNet` returns before the lock is taken.
        _ => None,
    }
}
