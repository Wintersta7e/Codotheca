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

pub mod source;

use crate::index::Index;
use crate::proto::dispatch::CommandFailure;
use crate::protocol::ErrorCode;

/// Everything §25.5's index-only commands need. `now` is unix **seconds**, supplied by the
/// caller so no handler reads the clock itself.
#[derive(Debug)]
pub struct ReadmeCtx<'a> {
    pub index: &'a Index,
    pub now: i64,
}

/// The commands this module owns, as data, so the seam and the router cannot drift apart — the
/// shape of the defect R37 found between the schema and the handlers.
pub const README_COMMANDS: [&str; 1] = ["projects.readme"];

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

/// `None` means "this module does not own that command", which is what the router chains on.
#[must_use]
pub fn dispatch_readme_command(
    ctx: &ReadmeCtx<'_>,
    command: &str,
    args: serde_json::Value,
) -> Option<Result<serde_json::Value, CommandFailure>> {
    match command {
        "projects.readme" => Some(source::handle_readme(ctx, args)),
        _ => None,
    }
}
