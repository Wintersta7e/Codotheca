//! §29.1's HEAD enumeration and §29.6's blob read — J7's half of the git seam.
//!
//! **This sits beside [`parse_ls_files_z`](super::inventory::parse_ls_files_z) and does not
//! replace it.** `ls-files -s` reads the **index**, which is the right basis for J3 and the wrong
//! one for J7: a content scan built on it would yield findings from uncommitted edits — debt
//! flickering as the user types, and a blob id HEAD never contained landing in a permanent,
//! library-wide cache.
//!
//! **The deadlock is the same one `inventory.rs` records, one level down.** `cat-file --batch`
//! writes every object's whole body, so its stdout fills the pipe far sooner than
//! `--batch-check`'s does. The only spawn allowed here is
//! [`GitExec::run_piped`](super::exec::GitExec::run_piped), which writes stdin on one thread,
//! drains stdout on another, and closes stdin so git sees EOF.

use std::ffi::OsStr;
use std::io::{BufRead, Read, Write};

use crate::cancel::CancelToken;

use super::error::GitResult;
use super::exec::{GitExec, RunLimits};
use super::repo::RepoHandle;

/// One record of `git ls-tree -r -z --full-tree HEAD`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TreeEntry {
    /// The six-digit octal mode, as git printed it. Kept as text for the same reason
    /// `IndexEntry.mode` is: the only questions asked of it are `== "100644"` and `== "100755"`,
    /// and an octal parse turns one comparison into two conversions that can disagree.
    pub mode: String,
    /// `blob`, `tree` or `commit`. `-r` flattens trees; a `commit` record is a gitlink and a
    /// submodule is its own project (§4.4).
    pub kind: String,
    /// The object id.
    pub oid: String,
    /// Raw path bytes. A Linux path is arbitrary bytes and never round-trips through `String`.
    pub path: Vec<u8>,
}

/// One object `cat-file --batch` answered for.
///
/// `size_bytes` comes from `--batch`'s own header line, so a blob over the caller's cap is
/// **recorded and its body discarded** rather than never measured: `too_large` without the size
/// is a verdict with no evidence (§29.2 rule 4).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BlobRead {
    /// The object id asked for.
    pub oid: String,
    /// The size git reported, whether or not the body was kept.
    pub size_bytes: u64,
    /// The body, or `None` when it exceeded `byte_cap`.
    pub bytes: Option<Vec<u8>>,
}

/// Parse `ls-tree -r -z`: `<mode> SP <type> SP <oid> TAB <path>` per NUL-terminated record.
///
/// A malformed record is skipped, never guessed at.
#[must_use]
pub fn parse_ls_tree_z(bytes: &[u8]) -> Vec<TreeEntry> {
    let mut out = Vec::new();
    for record in bytes.split(|b| *b == 0).filter(|r| !r.is_empty()) {
        let Some(tab) = record.iter().position(|b| *b == b'\t') else {
            continue;
        };
        let (meta, rest) = record.split_at(tab);
        let path = rest.get(1..).unwrap_or_default().to_vec();
        let meta = String::from_utf8_lossy(meta);
        let mut parts = meta.split_whitespace();
        let (Some(mode), Some(kind), Some(oid)) = (parts.next(), parts.next(), parts.next()) else {
            continue;
        };
        out.push(TreeEntry {
            mode: mode.to_owned(),
            kind: kind.to_owned(),
            oid: oid.to_owned(),
            path,
        });
    }
    out
}

/// Enumerate the committed tree at `HEAD`, recursively.
///
/// **Bare is not the discriminator** (§29.1): `--full-tree` needs no working tree, so a bare
/// repository with commits enumerates exactly like any other. The honest gate is an unborn HEAD,
/// which the caller answers from `location.head_oid` before ever reaching here.
pub fn head_tree(
    exec: &GitExec,
    repo: &RepoHandle,
    limits: RunLimits,
    cancel: &CancelToken,
) -> GitResult<Vec<TreeEntry>> {
    let out = exec.run(
        repo,
        &[
            OsStr::new("ls-tree"),
            OsStr::new("-r"),
            OsStr::new("-z"),
            OsStr::new("--full-tree"),
            OsStr::new("HEAD"),
        ],
        limits,
        cancel,
    )?;
    Ok(parse_ls_tree_z(&out.stdout))
}

/// One `cat-file --batch` invocation's result.
///
/// `covered` is what makes the batch resumable: the caller cannot tell a body the budget refused
/// from one git answered `missing` for, and it must not stall its cursor on either.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BlobBatch {
    /// One entry per **blob** the batch kept a verdict for, in request order.
    pub reads: Vec<BlobRead>,
    /// How many leading entries of the request list were answered before `budget_bytes` stopped
    /// it. Equal to the request length when the budget was never reached.
    pub covered: usize,
}

/// Read `oids` through one `cat-file --batch`, keeping each body up to `byte_cap` and stopping at
/// `budget_bytes` of kept body across the whole batch.
///
/// An oid git answers `missing` for contributes nothing, exactly as `tracked_inventory` already
/// handles it — but its header line must still be consumed, or the reader falls out of step with
/// the stream and every later body is attributed to the wrong object.
///
/// **Past the budget the stream is drained and discarded rather than abandoned.** Returning early
/// would leave git writing into a pipe nobody reads, which is the deadlock this module exists to
/// avoid — the cost is one batch of transfer, and the memory ceiling is what the budget is for.
pub fn read_blobs(
    exec: &GitExec,
    repo: &RepoHandle,
    oids: &[String],
    byte_cap: u64,
    budget_bytes: u64,
    limits: RunLimits,
    cancel: &CancelToken,
) -> GitResult<BlobBatch> {
    if oids.is_empty() {
        return Ok(BlobBatch {
            reads: Vec::new(),
            covered: 0,
        });
    }
    let queries: Vec<String> = oids.to_vec();

    exec.run_piped(
        repo,
        &[OsStr::new("cat-file"), OsStr::new("--batch")],
        limits,
        cancel,
        move |stdin: &mut dyn Write| {
            for oid in &queries {
                stdin.write_all(oid.as_bytes())?;
                stdin.write_all(b"\n")?;
            }
            stdin.flush()
        },
        move |stdout: &mut dyn BufRead| {
            let mut reads = Vec::new();
            let mut header = String::new();
            let mut kept_bytes = 0_u64;
            let mut covered = 0_usize;
            let mut spent = false;
            loop {
                header.clear();
                if stdout.read_line(&mut header)? == 0 {
                    break;
                }
                let mut parts = header.split_whitespace();
                let (Some(oid), Some(kind), Some(size)) =
                    (parts.next(), parts.next(), parts.next())
                else {
                    // "<oid> missing" has two fields and carries no body. It is still one of the
                    // requested oids answered, so the cursor may pass it.
                    if !spent {
                        covered += 1;
                    }
                    continue;
                };
                let Ok(size) = size.parse::<u64>() else {
                    continue;
                };
                // The body follows, then one LF, whatever the type is. Both are consumed here
                // even for a type this caller keeps nothing of.
                let keep = !spent && kind == "blob" && size <= byte_cap;
                let body = read_body(stdout, size, keep)?;
                if spent {
                    continue;
                }
                covered += 1;
                if kind == "blob" {
                    reads.push(BlobRead {
                        oid: oid.to_owned(),
                        size_bytes: size,
                        bytes: body,
                    });
                    if keep {
                        kept_bytes = kept_bytes.saturating_add(size);
                    }
                }
                if kept_bytes >= budget_bytes {
                    spent = true;
                }
            }
            Ok(BlobBatch { reads, covered })
        },
    )
}

/// Consume `size` bytes plus git's trailing newline, keeping them only when asked.
fn read_body(stdout: &mut dyn BufRead, size: u64, keep: bool) -> std::io::Result<Option<Vec<u8>>> {
    let kept = if keep {
        let mut buf = Vec::new();
        (&mut *stdout).take(size).read_to_end(&mut buf)?;
        Some(buf)
    } else {
        std::io::copy(&mut (&mut *stdout).take(size), &mut std::io::sink())?;
        None
    };
    let mut newline = [0_u8; 1];
    // A truncated stream is the end of the batch, not a failure of this object.
    let _ = stdout.read_exact(&mut newline);
    Ok(kept)
}
