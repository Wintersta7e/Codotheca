//! `check-ignore`: which of these paths does this repository's own ignore rules exclude?
//!
//! §9's activity scope is *a change to a tracked file, or to a non-ignored untracked file*, and
//! `check-ignore` **with no `--no-index`** is that rule in one call: it consults the index, so a
//! tracked path is never reported ignored even when a pattern would match it, and an untracked
//! path is reported ignored exactly when the rules say so. The authority is always the
//! repository's own rules, never a built-in list of build-directory names — a built-in list
//! would be wrong for the first project that does not use one.
//!
//! **Paths go in on `--stdin -z`, in bounded batches.** Argv would have been preferable — the
//! five-hour deadlock recorded in this project came from writing a whole input to git's stdin
//! while its stdout filled the pipe — but `check-ignore` rejects `-z` outright without
//! `--stdin` (`fatal: -z only makes sense with --stdin`), and without `-z` the listing is
//! newline-separated and `core.quotePath`-quoted, which cannot round-trip a path containing a
//! newline or a non-ASCII byte. The deadlock is avoided the way it was diagnosed: the write and
//! the drain are on separate threads and stdin is closed when the input is done, which is
//! exactly what `run_piped` does, and each call carries at most
//! [`CHECK_IGNORE_BATCH`] paths so neither side can grow without bound.
//!
//! **The verdict is keyed on the full relative path, never on its parent directory.** Asking
//! about each distinct parent would collapse a 5,000-file build directory to one question, but a
//! force-added tracked file inside an ignored directory would then be dropped with it:
//! `check-ignore` is index-aware for a *path*, and a directory is not a tracked path.

use std::ffi::OsStr;
use std::path::PathBuf;

use crate::cancel::CancelToken;

use super::error::GitResult;
use super::exec::{GitExec, RunLimits};
use super::repo::RepoHandle;

/// One `check-ignore` call carries at most this many paths.
pub const CHECK_IGNORE_BATCH: usize = 64;

/// `check-ignore` exits `1` to mean "none of these paths is ignored". That is an answer, not a
/// failure, and it is the *usual* answer for a repository with no ignore rules at all.
const NONE_IGNORED: i32 = 1;

/// One verdict per input, in order: `true` means "ignored, and therefore out of §9's scope".
///
/// A `GitError` here is a real failure — `check-ignore`'s own "nothing matched" is exit `1`,
/// which is tolerated and yields an empty listing rather than an error.
///
/// # Errors
///
/// Any batch's `check-ignore` failure as [`GitExec::run_piped`] classifies it; no verdict is
/// returned for any path then.
pub fn check_ignore(
    exec: &GitExec,
    repo: &RepoHandle,
    limits: RunLimits,
    cancel: &CancelToken,
    rel: &[PathBuf],
) -> GitResult<Vec<bool>> {
    let args: Vec<&OsStr> = vec![
        OsStr::new("check-ignore"),
        OsStr::new("-z"),
        OsStr::new("--stdin"),
    ];
    let mut out = Vec::with_capacity(rel.len());
    for chunk in rel.chunks(CHECK_IGNORE_BATCH) {
        // Written NUL-separated on the writer thread, which closes stdin when it is done.
        let mut input = Vec::new();
        for path in chunk {
            input.extend_from_slice(path.to_string_lossy().as_bytes());
            input.push(0);
        }
        let stdout = exec.run_piped(
            repo,
            &args,
            limits.tolerating(NONE_IGNORED),
            cancel,
            move |stdin| stdin.write_all(&input),
            |reader| {
                let mut buf = Vec::new();
                reader.read_to_end(&mut buf)?;
                Ok(buf)
            },
        )?;
        // The listing is the ignored subset, NUL-separated. Both sides are compared lossily:
        // git echoes back what it was given, so a path that is not valid UTF-8 matches itself.
        let listed: std::collections::BTreeSet<String> = stdout
            .split(|b| *b == 0)
            .filter(|s| !s.is_empty())
            .map(|s| String::from_utf8_lossy(s).into_owned())
            .collect();
        out.extend(chunk.iter().map(|p| listed.contains(&*p.to_string_lossy())));
    }
    Ok(out)
}
