//! `projects.readme` — the whole document, under the cap J6 already owns.
//!
//! **Four answers, and none of them is a guess.** A file that opened is `present` with the name
//! that matched; a readable directory holding none of `README_NAMES` is `absent`; a directory
//! that cannot be listed is `REPO_UNREADABLE`, never `absent`; an id pair that names no live
//! location is `PROTOCOL`.
//!
//! **Why this does not call `j6_content::read_content`.** That reader takes exactly `cap` bytes
//! (`j6_content.rs:36-42`) and returns them with no way to tell a file that ended from a file
//! that was cut — the module header claims *"on exceed the field is omitted, never truncated
//! into a claim"* and the code truncates. J6's behaviour is phase-1 shipped contract and is not
//! this plan's to change, so the read here asks for `cap + 1` bytes and reports the difference.
//! The **cap itself** still has one owner: `j6_content::J6_BYTE_CAP`, read from the constant.

use std::io::Read as _;

use crate::jobs::j6_content::{J6_BYTE_CAP, README_NAMES};
use crate::proto::dispatch::{parse_args, CommandFailure};
use crate::protocol::{LocationId, ProjectId, ProjectsReadmeArgs, ReadmeSource, ReadmeStateKind};
use crate::readme::{ReadmeCtx, ReadmeError};

/// Where one location's working copy lives, read the way `read_place` reads it
/// (`jobs/mod.rs:352-383`): from `path_bytes` through `path_from_bytes`, never from
/// `path_display`, which §1.10 makes lossy and forbids for opening.
///
/// `pub` because `projects.readmeAssets` resolves the same root, off the index guard, and a
/// second copy of this query would be two answers to *where does this location live*.
///
/// # Errors
/// `UnknownSubject` when the pair names no live location; `Sqlite` for an index fault.
pub fn work_dir_of(
    conn: &rusqlite::Connection,
    project: ProjectId,
    location: LocationId,
) -> Result<std::path::PathBuf, ReadmeError> {
    let bytes: Option<Vec<u8>> = conn
        .query_row(
            "SELECT l.path_bytes
               FROM location l
               JOIN project p ON p.id = l.project_id
              WHERE l.id = ?1 AND l.project_id = ?2 AND p.merged_into IS NULL",
            rusqlite::params![location.0, project.0],
            |r| r.get::<_, Vec<u8>>(0),
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(ReadmeError::Sqlite(other)),
        })?;
    bytes
        .map(|b| crate::paths::path_from_bytes(&b))
        .ok_or(ReadmeError::UnknownSubject)
}

/// The largest prefix of `bytes` that is valid UTF-8 and no longer than `cap`.
///
/// A cut at an arbitrary byte lands mid-codepoint on any non-ASCII document, and
/// `from_utf8_lossy` would then render a replacement character **the file does not contain** —
/// an invented glyph inside a panel whose whole job is to show what is there.
fn cut_to_char_boundary(bytes: &[u8], cap: usize) -> String {
    let mut end = bytes.len().min(cap);
    loop {
        // `get`, never a slice: the crate denies `indexing_slicing`, and a prefix that is not
        // there must be `None` rather than a panic in the process the shell supervises.
        match bytes.get(..end) {
            Some(prefix) if std::str::from_utf8(prefix).is_ok() => {
                return String::from_utf8_lossy(prefix).into_owned();
            }
            _ if end == 0 => return String::new(),
            _ => end -= 1,
        }
    }
}

/// Read `work_dir`'s README, or say why there is none.
///
/// # Errors
/// `UnknownSubject` when the pair names no live location; `Unreadable` when the directory cannot
/// be listed; `Sqlite` for an index fault.
pub fn read_readme_source(
    ctx: &ReadmeCtx<'_>,
    project: ProjectId,
    location: LocationId,
) -> Result<ReadmeSource, ReadmeError> {
    let work_dir = work_dir_of(ctx.index.conn(), project, location)?;

    // **Five `open` calls, not a directory listing.** This runs under the process's one SQLite
    // mutex (`Route::Readme`), once per project-page open, and the corpus this project measures
    // itself against includes a 14,000-file working tree — reading every entry of it to decide
    // membership of a five-element list is an unbounded wait on the writer lock, which is the
    // thing R75's carve-out exists to keep off it. The listing below still happens, but only on
    // the branch that needs it.
    for name in README_NAMES {
        let path = work_dir.join(name);
        let Ok(file) = std::fs::File::open(&path) else {
            continue;
        };
        let mut bytes = Vec::new();
        // `cap + 1`: one byte past the cap is what distinguishes a document that ended from one
        // that was cut, and it costs one byte rather than the whole file.
        file.take(J6_BYTE_CAP as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(ReadmeError::Unreadable)?;
        let truncated = bytes.len() > J6_BYTE_CAP;
        return Ok(ReadmeSource {
            state: ReadmeStateKind::Present,
            path: Some((*name).to_owned()),
            text: Some(cut_to_char_boundary(&bytes, J6_BYTE_CAP)),
            read_at: Some(ctx.now),
            truncated,
        });
    }

    // **Nothing opened — and only now does the distinction cost anything.** A missing file and an
    // unreadable directory both answer `NotFound` on some platforms, so a failed `open` alone
    // cannot tell *absent* from *we could not look*: `absent` is a positive claim about the user's
    // disk and is never made on a directory this process could not read. One `read_dir` answers
    // that, on the one branch where the answer is in doubt.
    std::fs::read_dir(&work_dir).map_err(ReadmeError::Unreadable)?;

    Ok(ReadmeSource {
        state: ReadmeStateKind::Absent,
        path: None,
        text: None,
        read_at: Some(ctx.now),
        truncated: false,
    })
}

/// `projects.readme`.
///
/// # Errors
/// `PROTOCOL` for an argument shape the schema does not admit or a pair that names no live
/// location; `REPO_UNREADABLE` for a working directory that cannot be listed; `INTERNAL` for an
/// index fault.
pub fn handle_readme(
    ctx: &ReadmeCtx<'_>,
    args: serde_json::Value,
) -> Result<serde_json::Value, CommandFailure> {
    let args: ProjectsReadmeArgs = parse_args(args)?;
    let source = read_readme_source(ctx, args.project_id, args.location_id)?;
    serde_json::to_value(source).map_err(|e| CommandFailure::internal(e.to_string()))
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
mod tests {
    use super::cut_to_char_boundary;

    #[test]
    fn a_cut_mid_codepoint_moves_back_to_the_boundary() {
        // 'é' is two bytes, so a cap of 2 lands inside it.
        let bytes = "aé".as_bytes();
        assert_eq!(bytes.len(), 3);
        assert_eq!(cut_to_char_boundary(bytes, 2), "a");
        assert_eq!(cut_to_char_boundary(bytes, 3), "aé");
    }

    #[test]
    fn a_cap_past_the_end_returns_the_whole_input() {
        assert_eq!(cut_to_char_boundary(b"abc", 99), "abc");
    }
}
