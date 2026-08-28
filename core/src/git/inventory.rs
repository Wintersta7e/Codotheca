//! §3.3's third row: `ls-files -z -s` into `cat-file --batch-check`.
//!
//! **This is the deadlock.** Writing every object id into `cat-file`'s stdin and only then
//! reading its stdout hangs the moment git's stdout fills the pipe buffer; it hung for five
//! hours on the first large repository. The only spawn allowed here is
//! [`GitExec::run_piped`](super::exec::GitExec::run_piped), which writes stdin on one thread,
//! drains stdout on another, and closes stdin so git sees EOF.

use std::collections::{BTreeMap, HashMap};
use std::ffi::OsStr;
use std::io::{BufRead, Write};

use crate::cancel::CancelToken;
use crate::clock::Clock;

use super::error::{GitError, GitResult};
use super::exec::{GitExec, RunLimits};
use super::repo::RepoHandle;

/// One row of `git ls-files -s`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexEntry {
    /// The blob's object id.
    pub oid: String,
    /// Merge stage: `0` ordinarily, `1`/`2`/`3` while conflicted.
    pub stage: u8,
    /// Raw path bytes. Linux paths are arbitrary bytes and never round-trip through `String`.
    pub path: Vec<u8>,
}

/// What J3 produces.
#[derive(Debug, Clone)]
pub struct TrackedInventory {
    /// `project.tracked_files`.
    pub tracked_files: u32,
    /// `project.size_tracked_bytes` — HEAD blob bytes, never worktree bytes (§5.3).
    pub size_tracked_bytes: u64,
    /// Bytes per lowercase file extension; `""` collects everything without a usable one.
    /// Mapping extensions to languages belongs to the caller, not here.
    pub extension_bytes: BTreeMap<String, u64>,
    /// When this was observed.
    pub observed_at: i64,
}

/// Parse `ls-files -z -s`: `<mode> <oid> <stage>\t<path>` per NUL-terminated record.
#[must_use]
pub fn parse_ls_files_z(bytes: &[u8]) -> Vec<IndexEntry> {
    let mut out = Vec::new();
    for record in bytes.split(|b| *b == 0).filter(|r| !r.is_empty()) {
        let Some(tab) = record.iter().position(|b| *b == b'\t') else {
            continue;
        };
        let (meta, rest) = record.split_at(tab);
        let path = rest.get(1..).unwrap_or_default().to_vec();
        let meta = String::from_utf8_lossy(meta);
        let mut parts = meta.split_whitespace();
        let (_mode, oid, stage) = (parts.next(), parts.next(), parts.next());
        let (Some(oid), Some(stage)) = (oid, stage) else {
            continue;
        };
        let Ok(stage) = stage.parse::<u8>() else {
            continue;
        };
        out.push(IndexEntry {
            oid: oid.to_owned(),
            stage,
            path,
        });
    }
    out
}

/// The lowercase extension of a path's last component, or `""` when there is no usable one.
#[must_use]
pub fn path_extension(path: &[u8]) -> String {
    let last = path
        .rsplit(|b| *b == b'/' || *b == b'\\')
        .next()
        .unwrap_or(path);
    let Some(dot) = last.iter().rposition(|b| *b == b'.') else {
        return String::new();
    };
    if dot == 0 {
        return String::new(); // a dotfile has no extension
    }
    let ext = last.get(dot + 1..).unwrap_or_default();
    if ext.is_empty() || ext.len() > 16 || !ext.iter().all(u8::is_ascii_alphanumeric) {
        return String::new();
    }
    String::from_utf8_lossy(ext).to_ascii_lowercase()
}

/// Preference order for one path's index entries. Lower wins.
///
/// Ordering on the stage number itself would rank stage 1 — the merge *base* — above stage 2,
/// so a conflicted file would be measured at the size it had before either side touched it.
fn stage_rank(stage: u8) -> u8 {
    match stage {
        0 => 0, // the ordinary, unconflicted entry
        2 => 1, // "ours" while conflicted, which is what is on disk
        _ => 2, // the base and their side are never what this inventory measures
    }
}

/// Choose one entry per path: stage 0 when it exists, otherwise stage 2 (ours).
fn one_per_path(entries: Vec<IndexEntry>) -> Vec<IndexEntry> {
    let mut chosen: HashMap<Vec<u8>, IndexEntry> = HashMap::new();
    for e in entries {
        // `Option::is_none_or` is stable only from 1.82 and the workspace MSRV is 1.80.
        let better = chosen.get(&e.path).map_or(true, |existing| {
            stage_rank(e.stage) < stage_rank(existing.stage)
        });
        if better {
            chosen.insert(e.path.clone(), e);
        }
    }
    let mut out: Vec<IndexEntry> = chosen.into_values().collect();
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

/// Inventory the index and the HEAD blobs it points at.
pub fn tracked_inventory(
    exec: &GitExec,
    repo: &RepoHandle,
    limits: RunLimits,
    cancel: &CancelToken,
    clock: &dyn Clock,
) -> GitResult<TrackedInventory> {
    let listed = exec.run(
        repo,
        &[OsStr::new("ls-files"), OsStr::new("-z"), OsStr::new("-s")],
        limits,
        cancel,
    )?;
    let entries = one_per_path(parse_ls_files_z(&listed.stdout));
    if entries.is_empty() {
        return Ok(TrackedInventory {
            tracked_files: 0,
            size_tracked_bytes: 0,
            extension_bytes: BTreeMap::new(),
            observed_at: clock.now_unix(),
        });
    }

    let mut unique: Vec<String> = entries.iter().map(|e| e.oid.clone()).collect();
    unique.sort_unstable();
    unique.dedup();
    let queries = unique;

    let sizes: HashMap<String, u64> = exec.run_piped(
        repo,
        &[OsStr::new("cat-file"), OsStr::new("--batch-check")],
        limits,
        cancel,
        move |stdin: &mut dyn Write| {
            for oid in &queries {
                stdin.write_all(oid.as_bytes())?;
                stdin.write_all(b"\n")?;
            }
            stdin.flush()
        },
        |stdout: &mut dyn BufRead| {
            let mut map = HashMap::new();
            let mut line = String::new();
            loop {
                line.clear();
                if stdout.read_line(&mut line)? == 0 {
                    break;
                }
                let mut parts = line.split_whitespace();
                let (Some(oid), Some(kind), Some(size)) =
                    (parts.next(), parts.next(), parts.next())
                else {
                    continue; // "<oid> missing" has two fields and contributes nothing
                };
                if kind == "blob" {
                    if let Ok(n) = size.parse::<u64>() {
                        map.insert(oid.to_owned(), n);
                    }
                }
            }
            Ok(map)
        },
    )?;

    let mut total: u64 = 0;
    let mut extension_bytes: BTreeMap<String, u64> = BTreeMap::new();
    for e in &entries {
        // A missing object contributes nothing; it is not a zero-byte file.
        let Some(size) = sizes.get(&e.oid) else {
            continue;
        };
        total = total.saturating_add(*size);
        *extension_bytes.entry(path_extension(&e.path)).or_insert(0) += *size;
    }

    Ok(TrackedInventory {
        tracked_files: u32::try_from(entries.len()).map_err(|_| GitError::Internal {
            detail: "tracked file count exceeds u32".to_owned(),
        })?,
        size_tracked_bytes: total,
        extension_bytes,
        observed_at: clock.now_unix(),
    })
}
