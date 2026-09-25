//! §45.2's reads for the deletion analyser: through git, and **failing closed**.
//!
//! Measured on git 2.43 (files backend): `rev-parse --all` exits 0 and silently omits a ref whose
//! loose file it cannot read, omits a dangling symbolic ref without a word, and omits a garbage
//! one with only a warning on stderr; `log -g` over an unreadable stash reflog exits 0 printing
//! nothing. A read that believed the exit code would report *no refs, no stashes* about a
//! repository holding both, and a deletion gate would then call it safe.
//!
//! So every read here returns `Err` — never a shorter answer — on a non-zero exit, on output it
//! cannot parse, and on **any byte on the diagnostic stream of an enumerating read**. The listing
//! is also corroborated, under the files backend, by an error-strict walk of `refs/` and
//! `packed-refs`, and the two name sets must agree. Under reftable git's listing is the only
//! reader: a file walk finds `refs/heads` as a stub.
//!
//! Every subcommand here is on `core/tests/git_readonly.rs`'s allow list; revisions and `-c`
//! values are named constants with a justified allow-list entry each.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{OsStr, OsString};
use std::path::Path;

use crate::cancel::CancelToken;

use super::error::{GitError, GitResult};
use super::exec::{GitExec, GitOutput, RunLimits};
use super::inventory::IndexEntry;
use super::invocation::absent_graft_path;
use super::repo::RepoHandle;

/// The stash ref, as a revision. A named constant because the read-only audit admits a bare
/// literal only when it is a subcommand.
const REFS_STASH: &str = "refs/stash";

/// §45.2 row 7: the untracked cache goes stale on some filesystems' modification times, and a
/// stale cache hides untracked files from `status`. Read with it off, whatever the config says.
const UNTRACKED_CACHE_OFF: &str = "core.untrackedCache=false";

/// The pseudorefs an interrupted operation leaves, resolved **through git**: under reftable no
/// `CHERRY_PICK_HEAD` file exists and `rev-parse --verify` still resolves it (§45.5, measured).
const PSEUDOREFS: [(&str, InterruptedOperation); 3] = [
    ("MERGE_HEAD", InterruptedOperation::Merge),
    ("CHERRY_PICK_HEAD", InterruptedOperation::CherryPick),
    ("REVERT_HEAD", InterruptedOperation::Revert),
];

/// The operation markers that are directories or files in the git dir, not refs.
const MARKERS: [(&str, InterruptedOperation); 4] = [
    ("rebase-merge", InterruptedOperation::RebaseMerge),
    ("rebase-apply", InterruptedOperation::RebaseApply),
    ("sequencer", InterruptedOperation::Sequencer),
    ("BISECT_LOG", InterruptedOperation::Bisect),
];

/// Deepest symbolic-ref chain the walk follows; git itself gives up at five.
const SYMREF_DEPTH: u32 = 5;

/// Deepest directory the walk descends under `refs/` before it refuses.
const WALK_DEPTH: u32 = 32;

/// How a repository stores its refs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefBackend {
    /// Loose files under `refs/` and `packed-refs`; the listing is corroborated by a walk.
    Files,
    /// `<common_dir>/reftable/`; git's listing is the only reader.
    Reftable,
}

/// Where `HEAD` points (§45.2 row 2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HeadState {
    /// A branch, by its full ref name.
    Symbolic(String),
    /// A commit, by its object id — a root of its own.
    Detached(String),
    /// A branch with no commits yet. The only state that is *no commits*.
    Unborn,
}

/// One ref the analyser treats as a root (§45.2 row 1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefEntry {
    /// The full ref name, e.g. `refs/heads/main`.
    pub name: String,
    /// The object it names.
    pub oid: String,
    /// That object's type as git reports it: `commit`, `tag`, `tree` or `blob`. An annotated
    /// tag's object is itself a preserved byte (row 4).
    pub object_type: String,
}

/// Every ref of a repository except `refs/remotes/*` and `refs/prefetch/*`, and its `HEAD`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefListing {
    /// Which storage was read.
    pub backend: RefBackend,
    /// Where `HEAD` points.
    pub head: HeadState,
    /// Every root ref, sorted by name.
    pub refs: Vec<RefEntry>,
}

/// The stash, as §45.2 row 3 reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StashEntries {
    /// Every entry's commit, newest first. A stash ref with no reflog counts as one entry — a
    /// floor, not a count.
    Entries(Vec<String>),
    /// The stash reflog exists and cannot be read: nothing about the stash is known.
    Unreadable,
}

/// One record of `status --porcelain=v2`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StatusEntry {
    /// A tracked path with a staged and/or unstaged change, renames and submodules included.
    Changed {
        /// The two status letters, staged then unstaged; `.` is unchanged.
        xy: String,
        /// Raw path bytes.
        path: Vec<u8>,
    },
    /// A path in a merge conflict.
    Unmerged {
        /// Raw path bytes.
        path: Vec<u8>,
    },
    /// An untracked path. A wholly untracked directory is one entry ending in `/`.
    Untracked {
        /// Raw path bytes.
        path: Vec<u8>,
    },
    /// An ignored path. A wholly ignored directory is one entry ending in `/`.
    Ignored {
        /// Raw path bytes.
        path: Vec<u8>,
    },
}

/// The worktree and index, as §45.2 rows 5–8 read them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeScan {
    /// Every status record, untracked and ignored included.
    pub entries: Vec<StatusEntry>,
    /// Tracked paths marked assume-unchanged or skip-worktree, whose edits `status` cannot see
    /// (row 6). Whether each exists on disk is the caller's question.
    pub hidden: Vec<Vec<u8>>,
    /// The index, one entry per `(path, stage)` (row 5).
    pub index: Vec<IndexEntry>,
}

/// An operation git left unfinished (§45.5's `interrupted_operation`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterruptedOperation {
    /// `MERGE_HEAD` resolves.
    Merge,
    /// `CHERRY_PICK_HEAD` resolves.
    CherryPick,
    /// `REVERT_HEAD` resolves.
    Revert,
    /// `rebase-merge/` exists.
    RebaseMerge,
    /// `rebase-apply/` exists.
    RebaseApply,
    /// `sequencer/` exists.
    Sequencer,
    /// `BISECT_LOG` exists.
    Bisect,
}

/// Replace objects and grafts off, for a read that loads commits: the analyser reads the objects
/// a ref names, never what a replace ref or a graft file substitutes. Two pins, because
/// `--no-replace-objects` does not disable a graft (measured on git 2.43) — and a graft file
/// that is read makes git print a deprecation hint, which a strict read refuses.
fn history_off(exec: &GitExec) -> [(&'static str, OsString); 2] {
    [
        ("GIT_NO_REPLACE_OBJECTS", OsString::from("1")),
        (
            "GIT_GRAFT_FILE",
            absent_graft_path(exec.hooks_dir()).into_os_string(),
        ),
    ]
}

/// Is `text` an object id — 40 or 64 lowercase hex characters?
///
/// **One owner** for the shape: the write path's `ObjectId` refuses exactly what this refuses.
#[must_use]
pub fn is_object_id(text: &str) -> bool {
    (text.len() == 40 || text.len() == 64)
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn unreadable(detail: impl Into<String>) -> GitError {
    GitError::Unreadable {
        detail: detail.into(),
    }
}

fn io_failure(error: &std::io::Error, what: &Path) -> GitError {
    let detail = format!("{}: {error}", what.display());
    if error.kind() == std::io::ErrorKind::PermissionDenied {
        GitError::PermissionDenied { detail }
    } else {
        unreadable(detail)
    }
}

/// §45.6: a non-empty diagnostic stream on an enumerating read is an error in itself — git's
/// warnings are how it reports the refs it skipped.
fn quiet(out: &GitOutput, what: &str) -> GitResult<()> {
    if out.stderr.is_empty() {
        Ok(())
    } else {
        Err(unreadable(format!(
            "{what} wrote to its diagnostic stream: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )))
    }
}

fn text_of(bytes: &[u8], what: &str) -> GitResult<String> {
    String::from_utf8(bytes.to_vec()).map_err(|_| unreadable(format!("{what} printed non-UTF-8")))
}

/// `refs/remotes/*` and `refs/prefetch/*`: the caches a local ref is checked **against**, never
/// roots of their own (§45.2's *Not in `P(L)`*).
fn is_tracking(name: &str) -> bool {
    name.starts_with("refs/remotes/") || name.starts_with("refs/prefetch/")
}

/// `cat-file --batch-check` over `queries`, one answer per line in input order: `Some((oid,
/// type))`, or `None` for an object git reports missing.
fn batch_check(
    exec: &GitExec,
    repo: &RepoHandle,
    queries: &[String],
    env: &[(&str, OsString)],
    limits: RunLimits,
    cancel: &CancelToken,
) -> GitResult<Vec<Option<(String, String)>>> {
    if queries.is_empty() {
        return Ok(Vec::new());
    }
    let mut stdin = Vec::new();
    for query in queries {
        stdin.extend_from_slice(query.as_bytes());
        stdin.push(b'\n');
    }
    let out = exec.run_with_stdin(
        repo,
        &[OsStr::new("cat-file"), OsStr::new("--batch-check")],
        stdin,
        env,
        limits,
        cancel,
    )?;
    let text = text_of(&out.stdout, "cat-file --batch-check")?;
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() != queries.len() {
        return Err(unreadable(format!(
            "cat-file --batch-check answered {} of {} queries",
            lines.len(),
            queries.len()
        )));
    }
    let mut answers = Vec::with_capacity(lines.len());
    for (query, line) in queries.iter().zip(lines) {
        let parts: Vec<&str> = line.split(' ').collect();
        match parts.as_slice() {
            [asked, "missing"] if asked == query => answers.push(None),
            [oid, kind, size] if is_object_id(oid) && size.bytes().all(|b| b.is_ascii_digit()) => {
                answers.push(Some(((*oid).to_owned(), (*kind).to_owned())));
            }
            _ => {
                return Err(unreadable(format!(
                    "cat-file --batch-check answered {line:?} for {query:?}"
                )))
            }
        }
    }
    Ok(answers)
}

/// One loose ref file's contents.
enum Loose {
    Direct,
    Symbolic(String),
}

/// Walk `dir` under `base` (the common dir), collecting every loose ref outside the tracking
/// namespaces. **Error-strict**: a directory or file it cannot read is an error, never a skip.
fn walk_loose(
    base: &Path,
    dir: &Path,
    depth: u32,
    out: &mut BTreeMap<String, Loose>,
) -> GitResult<()> {
    if depth > WALK_DEPTH {
        return Err(unreadable(format!(
            "refs nest deeper than {WALK_DEPTH} under {}",
            dir.display()
        )));
    }
    for entry in std::fs::read_dir(dir).map_err(|e| io_failure(&e, dir))? {
        let entry = entry.map_err(|e| io_failure(&e, dir))?;
        let path = entry.path();
        let rel = path
            .strip_prefix(base)
            .map_err(|_| unreadable(format!("{} escaped the git dir", path.display())))?;
        let name = rel
            .to_str()
            .ok_or_else(|| {
                unreadable(format!(
                    "a ref name that is not UTF-8 under {}",
                    dir.display()
                ))
            })?
            .replace('\\', "/");
        if is_tracking(&format!("{name}/")) || is_tracking(&name) {
            continue;
        }
        let kind = entry.file_type().map_err(|e| io_failure(&e, &path))?;
        if kind.is_dir() {
            walk_loose(base, &path, depth + 1, out)?;
        } else if kind.is_file() {
            let text = std::fs::read_to_string(&path).map_err(|e| io_failure(&e, &path))?;
            let trimmed = text.trim_end();
            let loose = if let Some(target) = trimmed.strip_prefix("ref: ") {
                Loose::Symbolic(target.trim().to_owned())
            } else if is_object_id(trimmed) {
                Loose::Direct
            } else {
                return Err(unreadable(format!("{name} holds no object id")));
            };
            out.insert(name, loose);
        } else {
            return Err(unreadable(format!(
                "{name} is neither a file nor a directory"
            )));
        }
    }
    Ok(())
}

/// `packed-refs`, strictly: an absent file is none, an unreadable or malformed one an error.
fn packed_names(common_dir: &Path) -> GitResult<BTreeSet<String>> {
    let path = common_dir.join("packed-refs");
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeSet::new()),
        Err(e) => return Err(io_failure(&e, &path)),
    };
    let mut names = BTreeSet::new();
    for line in text.lines() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(peeled) = line.strip_prefix('^') {
            if !is_object_id(peeled.trim()) {
                return Err(unreadable("packed-refs holds a malformed peel line"));
            }
            continue;
        }
        match line.split_once(' ') {
            Some((oid, name)) if is_object_id(oid) && name.starts_with("refs/") => {
                names.insert(name.to_owned());
            }
            _ => return Err(unreadable("packed-refs holds a malformed line")),
        }
    }
    Ok(names)
}

/// Resolve `name` as git's listing does: a symbolic ref prints as the ref it finally names, and
/// one that names nothing is an error — git omits it without a word.
fn resolve(
    name: &str,
    loose: &BTreeMap<String, Loose>,
    packed: &BTreeSet<String>,
    depth: u32,
) -> GitResult<String> {
    match loose.get(name) {
        Some(Loose::Direct) => Ok(name.to_owned()),
        Some(Loose::Symbolic(target)) if depth < SYMREF_DEPTH => {
            resolve(target, loose, packed, depth + 1)
        }
        Some(Loose::Symbolic(_)) => Err(unreadable(format!(
            "{name} is a symbolic ref chain deeper than {SYMREF_DEPTH}"
        ))),
        None if packed.contains(name) => Ok(name.to_owned()),
        None => Err(unreadable(format!("{name} is a dangling symbolic ref"))),
    }
}

/// The files backend's name set, by an error-strict walk.
fn walked_names(common_dir: &Path) -> GitResult<BTreeSet<String>> {
    let packed = packed_names(common_dir)?;
    let mut loose = BTreeMap::new();
    let refs = common_dir.join("refs");
    if refs.exists() {
        walk_loose(common_dir, &refs, 0, &mut loose)?;
    }
    let mut names = BTreeSet::new();
    for name in loose.keys().chain(packed.iter()) {
        if is_tracking(name) {
            continue;
        }
        let resolved = resolve(name, &loose, &packed, 0)?;
        if !is_tracking(&resolved) {
            names.insert(resolved);
        }
    }
    Ok(names)
}

/// Where `HEAD` points, before its object is read.
fn head_state(
    exec: &GitExec,
    repo: &RepoHandle,
    limits: RunLimits,
    cancel: &CancelToken,
) -> GitResult<HeadState> {
    match exec.run(
        repo,
        &[
            OsStr::new("rev-parse"),
            OsStr::new("--symbolic-full-name"),
            OsStr::new("HEAD"),
        ],
        limits,
        cancel,
    ) {
        Ok(out) => {
            quiet(&out, "rev-parse HEAD")?;
            let name = text_of(&out.stdout, "rev-parse HEAD")?.trim().to_owned();
            if name == "HEAD" {
                // The OID arrives with the batch read; a placeholder until then.
                Ok(HeadState::Detached(String::new()))
            } else if name.starts_with("refs/") {
                Ok(HeadState::Symbolic(name))
            } else {
                Err(unreadable(format!("HEAD resolved to {name:?}")))
            }
        }
        // `HEAD` does not resolve. Only a branch with no commits is *unborn*; git says so in its
        // own words, and every other reason — a broken ref, a missing object — is unreadable.
        Err(first) => match exec.run(
            repo,
            &[
                OsStr::new("log"),
                OsStr::new("-1"),
                OsStr::new("--format=%H"),
                OsStr::new("--no-show-signature"),
                OsStr::new("--no-color"),
            ],
            limits,
            cancel,
        ) {
            Err(second)
                if second
                    .to_string()
                    .to_ascii_lowercase()
                    .contains("does not have any commits yet") =>
            {
                Ok(HeadState::Unborn)
            }
            Err(second) => Err(second),
            Ok(_) => Err(first),
        },
    }
}

/// §45.2 rows 1–2: every ref but the tracking namespaces, with its object and type, and `HEAD`.
///
/// Spawns `rev-parse` twice (three times when `HEAD` does not resolve, to tell unborn from
/// broken) and `cat-file --batch-check` once. Under the files backend the listing is
/// corroborated by a walk of `refs/` and `packed-refs`.
///
/// # Errors
/// Any failed or noisy git invocation; a ref naming a missing object; under the files backend a
/// directory or ref file that cannot be read, a garbage or dangling ref, or a name set that
/// differs from git's.
pub(crate) fn enumerate_refs(
    exec: &GitExec,
    repo: &RepoHandle,
    limits: RunLimits,
    cancel: &CancelToken,
) -> GitResult<RefListing> {
    let listed = exec.run(
        repo,
        &[
            OsStr::new("rev-parse"),
            OsStr::new("--symbolic-full-name"),
            OsStr::new("--all"),
        ],
        limits,
        cancel,
    )?;
    quiet(&listed, "rev-parse --all")?;
    let mut names = BTreeSet::new();
    for line in text_of(&listed.stdout, "rev-parse --all")?.lines() {
        let name = line.trim();
        if name.is_empty() {
            continue;
        }
        if !name.starts_with("refs/") {
            return Err(unreadable(format!("rev-parse --all listed {name:?}")));
        }
        if !is_tracking(name) {
            names.insert(name.to_owned());
        }
    }

    let backend = if repo.common_dir.join("reftable").is_dir() {
        RefBackend::Reftable
    } else {
        RefBackend::Files
    };
    if backend == RefBackend::Files {
        let walked = walked_names(&repo.common_dir)?;
        if walked != names {
            return Err(unreadable(format!(
                "git listed {} refs and the walk found {}; they differ by {:?}",
                names.len(),
                walked.len(),
                walked.symmetric_difference(&names).collect::<Vec<_>>()
            )));
        }
    }

    let mut head = head_state(exec, repo, limits, cancel)?;
    let mut queries: Vec<String> = names.into_iter().collect();
    let detached = matches!(head, HeadState::Detached(_));
    if detached {
        queries.push("HEAD".to_owned());
    }
    // Replace objects off: the OID a ref names, not the object a replace ref substitutes for it.
    let env = [("GIT_NO_REPLACE_OBJECTS", OsString::from("1"))];
    let answers = batch_check(exec, repo, &queries, &env, limits, cancel)?;
    let mut refs = Vec::with_capacity(answers.len());
    for (name, answer) in queries.into_iter().zip(answers) {
        let Some((oid, object_type)) = answer else {
            return Err(unreadable(format!("{name} names a missing object")));
        };
        if name == "HEAD" {
            head = HeadState::Detached(oid);
        } else {
            refs.push(RefEntry {
                name,
                oid,
                object_type,
            });
        }
    }
    Ok(RefListing {
        backend,
        head,
        refs,
    })
}

/// §45.2 row 3: every stash entry, newest first, through git — `log -g`, never `git stash`.
///
/// Spawns `rev-parse --verify` once and, when a stash exists, `log -g` once.
///
/// # Errors
/// A failed or noisy git invocation, an unparseable entry, or a reflog whose newest entry is not
/// the stash ref. A files-backend reflog that exists and cannot be read is **not** an error: it
/// is the answer [`StashEntries::Unreadable`].
pub(crate) fn stash_entries(
    exec: &GitExec,
    repo: &RepoHandle,
    limits: RunLimits,
    cancel: &CancelToken,
) -> GitResult<StashEntries> {
    // Measured: over a permission-denied reflog `log -g` exits 0 with empty output, which reads
    // exactly like *no reflog*. The file is checked first, and only under the files backend.
    if !repo.common_dir.join("reftable").is_dir() {
        let log = repo.common_dir.join("logs").join("refs").join("stash");
        match std::fs::read(&log) {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Ok(StashEntries::Unreadable),
        }
    }

    let tip = exec.run(
        repo,
        &[
            OsStr::new("rev-parse"),
            OsStr::new("--verify"),
            OsStr::new("--quiet"),
            OsStr::new(REFS_STASH),
        ],
        limits.tolerating(1),
        cancel,
    )?;
    let tip = text_of(&tip.stdout, "rev-parse refs/stash")?
        .trim()
        .to_owned();
    if tip.is_empty() {
        return Ok(StashEntries::Entries(Vec::new()));
    }
    if !is_object_id(&tip) {
        return Err(unreadable(format!("refs/stash resolved to {tip:?}")));
    }

    let log = exec.run_with_stdin(
        repo,
        &[
            OsStr::new("log"),
            OsStr::new("-g"),
            OsStr::new("--format=%H"),
            OsStr::new("--no-show-signature"),
            OsStr::new("--no-color"),
            OsStr::new(REFS_STASH),
        ],
        Vec::new(),
        &history_off(exec),
        limits,
        cancel,
    )?;
    quiet(&log, "log -g refs/stash")?;
    let mut entries = Vec::new();
    for line in text_of(&log.stdout, "log -g refs/stash")?.lines() {
        let oid = line.trim();
        if !is_object_id(oid) {
            return Err(unreadable(format!("log -g printed {oid:?}")));
        }
        entries.push(oid.to_owned());
    }
    if entries.is_empty() {
        // A stash ref with no reflog: the ref proves one entry exists and says nothing more.
        entries.push(tip);
    } else if entries.first() != Some(&tip) {
        return Err(unreadable(
            "the stash reflog's newest entry is not the stash ref",
        ));
    }
    Ok(StashEntries::Entries(entries))
}

/// Parse `status --porcelain=v2 -z` records strictly: an unknown record type is an error.
fn parse_status_entries(bytes: &[u8]) -> GitResult<Vec<StatusEntry>> {
    let malformed = |what: &str| unreadable(format!("status printed a malformed {what} record"));
    let mut entries = Vec::new();
    let mut records = bytes.split(|b| *b == 0).filter(|r| !r.is_empty());
    while let Some(record) = records.next() {
        let path_after = |fields: usize| -> Option<Vec<u8>> {
            record
                .splitn(fields, |b| *b == b' ')
                .nth(fields - 1)
                .map(<[u8]>::to_vec)
        };
        let xy = || {
            record
                .splitn(3, |b| *b == b' ')
                .nth(1)
                .map(|x| String::from_utf8_lossy(x).into_owned())
        };
        match record.first() {
            Some(b'#') => {}
            Some(b'1') => entries.push(StatusEntry::Changed {
                xy: xy().ok_or_else(|| malformed("changed"))?,
                path: path_after(9).ok_or_else(|| malformed("changed"))?,
            }),
            Some(b'2') => {
                entries.push(StatusEntry::Changed {
                    xy: xy().ok_or_else(|| malformed("renamed"))?,
                    path: path_after(10).ok_or_else(|| malformed("renamed"))?,
                });
                // A rename's original path is a second NUL-terminated field.
                records.next().ok_or_else(|| malformed("renamed"))?;
            }
            Some(b'u') => entries.push(StatusEntry::Unmerged {
                path: path_after(11).ok_or_else(|| malformed("unmerged"))?,
            }),
            Some(b'?') => entries.push(StatusEntry::Untracked {
                path: path_after(2).ok_or_else(|| malformed("untracked"))?,
            }),
            Some(b'!') => entries.push(StatusEntry::Ignored {
                path: path_after(2).ok_or_else(|| malformed("ignored"))?,
            }),
            _ => return Err(malformed("unknown")),
        }
    }
    Ok(entries)
}

/// `ls-files -v -z`: the paths whose tag says assume-unchanged (lower case) or skip-worktree
/// (`S`).
fn parse_hidden(bytes: &[u8]) -> GitResult<Vec<Vec<u8>>> {
    let mut hidden = Vec::new();
    for record in bytes.split(|b| *b == 0).filter(|r| !r.is_empty()) {
        let (Some(tag), Some(b' '), Some(path)) = (record.first(), record.get(1), record.get(2..))
        else {
            return Err(unreadable("ls-files -v printed a malformed record"));
        };
        if tag.is_ascii_lowercase() || *tag == b'S' {
            hidden.push(path.to_vec());
        }
    }
    Ok(hidden)
}

/// `ls-files -s -z`, strictly: every record parses or the read fails.
fn parse_index(bytes: &[u8]) -> GitResult<Vec<IndexEntry>> {
    let mut entries = Vec::new();
    for record in bytes.split(|b| *b == 0).filter(|r| !r.is_empty()) {
        let malformed = || unreadable("ls-files -s printed a malformed record");
        let tab = record
            .iter()
            .position(|b| *b == b'\t')
            .ok_or_else(malformed)?;
        let (meta, rest) = record.split_at(tab);
        let meta = std::str::from_utf8(meta).map_err(|_| malformed())?;
        let mut parts = meta.split(' ');
        let (Some(mode), Some(oid), Some(stage), None) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return Err(malformed());
        };
        if !is_object_id(oid) {
            return Err(malformed());
        }
        entries.push(IndexEntry {
            mode: mode.to_owned(),
            oid: oid.to_owned(),
            stage: stage.parse().map_err(|_| malformed())?,
            path: rest.get(1..).unwrap_or_default().to_vec(),
        });
    }
    Ok(entries)
}

/// §45.2 rows 5–8: status with untracked **and** ignored paths, the hidden flags, and the index.
///
/// Spawns three: `status --porcelain=v2 -z --ignore-submodules=none -unormal --ignored=matching`
/// under `-c core.untrackedCache=false`, `ls-files -v -z` and `ls-files -s -z`. **Never
/// `-uall`**: an unignored `node_modules/` is one entry here and 120,003 there (D8, measured).
///
/// # Errors
/// Any failed or noisy invocation, or a record that does not parse.
pub(crate) fn worktree_scan(
    exec: &GitExec,
    repo: &RepoHandle,
    limits: RunLimits,
    cancel: &CancelToken,
) -> GitResult<WorktreeScan> {
    // `HEAD`'s own tree, never a replacement's or a graft's — and no graft hint on stderr.
    let status = exec.run_with_stdin(
        repo,
        &[
            OsStr::new("-c"),
            OsStr::new(UNTRACKED_CACHE_OFF),
            OsStr::new("status"),
            OsStr::new("--porcelain=v2"),
            OsStr::new("-z"),
            OsStr::new("--ignore-submodules=none"),
            OsStr::new("-unormal"),
            OsStr::new("--ignored=matching"),
        ],
        Vec::new(),
        &history_off(exec),
        limits,
        cancel,
    )?;
    quiet(&status, "status")?;
    let flags = exec.run(
        repo,
        &[OsStr::new("ls-files"), OsStr::new("-v"), OsStr::new("-z")],
        limits,
        cancel,
    )?;
    quiet(&flags, "ls-files -v")?;
    let staged = exec.run(
        repo,
        &[OsStr::new("ls-files"), OsStr::new("-s"), OsStr::new("-z")],
        limits,
        cancel,
    )?;
    quiet(&staged, "ls-files -s")?;
    Ok(WorktreeScan {
        entries: parse_status_entries(&status.stdout)?,
        hidden: parse_hidden(&flags.stdout)?,
        index: parse_index(&staged.stdout)?,
    })
}

/// Whether each object is present in the repository's own store, in `oids`' order.
///
/// One `cat-file --batch-check`, replace objects off. An empty list spawns nothing.
///
/// # Errors
/// An id that is not an object id (refused before the spawn), a failed invocation, or an answer
/// that does not parse.
pub(crate) fn objects_present(
    exec: &GitExec,
    repo: &RepoHandle,
    oids: &[String],
    limits: RunLimits,
    cancel: &CancelToken,
) -> GitResult<Vec<bool>> {
    if let Some(bad) = oids.iter().find(|oid| !is_object_id(oid)) {
        return Err(GitError::Internal {
            detail: format!("{bad:?} is not an object id"),
        });
    }
    let env = [("GIT_NO_REPLACE_OBJECTS", OsString::from("1"))];
    Ok(batch_check(exec, repo, oids, &env, limits, cancel)?
        .into_iter()
        .map(|answer| answer.is_some())
        .collect())
}

/// §45.6 step 7: is any commit reachable from `roots` **not** reachable from `covered`?
///
/// One `rev-list --stdin --max-count=1`, stdin the roots then `^<t>` for each covered tip, with
/// replace objects **and** grafts off — two pins, because `--no-replace-objects` does not disable
/// a graft (measured on git 2.43); only a graft file path that does not exist does. The spawn
/// count is one whatever the number of refs. An empty `roots` spawns nothing and is `false`.
///
/// # Errors
/// An id that is not an object id (refused before the spawn: stdin is argv by another channel),
/// a failed walk — a missing object included — or output that is not an object id.
pub(crate) fn any_uncovered(
    exec: &GitExec,
    repo: &RepoHandle,
    roots: &[String],
    covered: &[String],
    limits: RunLimits,
    cancel: &CancelToken,
) -> GitResult<bool> {
    if roots.is_empty() {
        return Ok(false);
    }
    if let Some(bad) = roots.iter().chain(covered).find(|oid| !is_object_id(oid)) {
        return Err(GitError::Internal {
            detail: format!("{bad:?} is not an object id"),
        });
    }
    let mut stdin = Vec::new();
    for root in roots {
        stdin.extend_from_slice(root.as_bytes());
        stdin.push(b'\n');
    }
    for tip in covered {
        stdin.push(b'^');
        stdin.extend_from_slice(tip.as_bytes());
        stdin.push(b'\n');
    }
    let env = history_off(exec);
    let out = exec.run_with_stdin(
        repo,
        &[
            OsStr::new("rev-list"),
            OsStr::new("--stdin"),
            OsStr::new("--max-count=1"),
        ],
        stdin,
        &env,
        limits,
        cancel,
    )?;
    let text = text_of(&out.stdout, "rev-list")?;
    let mut uncovered = false;
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if !is_object_id(line) {
            return Err(unreadable(format!("rev-list printed {line:?}")));
        }
        uncovered = true;
    }
    Ok(uncovered)
}

/// §45.5's interrupted operations: three pseudorefs through `rev-parse --verify --quiet` (exit
/// 1 is *absent*), and four markers as files in the git dir.
///
/// # Errors
/// A failed invocation, or a marker whose presence cannot be established.
pub(crate) fn interrupted_ops(
    exec: &GitExec,
    repo: &RepoHandle,
    limits: RunLimits,
    cancel: &CancelToken,
) -> GitResult<Vec<InterruptedOperation>> {
    let mut found = Vec::new();
    for (pseudoref, operation) in PSEUDOREFS {
        let out = exec.run(
            repo,
            &[
                OsStr::new("rev-parse"),
                OsStr::new("--verify"),
                OsStr::new("--quiet"),
                OsStr::new(pseudoref),
            ],
            limits.tolerating(1),
            cancel,
        )?;
        if !String::from_utf8_lossy(&out.stdout).trim().is_empty() {
            found.push(operation);
        }
    }
    for (marker, operation) in MARKERS {
        let path = repo.git_dir.join(marker);
        match std::fs::symlink_metadata(&path) {
            Ok(_) => found.push(operation),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(io_failure(&e, &path)),
        }
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use super::{is_object_id, parse_hidden, parse_status_entries, StatusEntry};

    #[test]
    fn an_object_id_is_forty_or_sixty_four_lowercase_hex() {
        assert!(is_object_id(&"a".repeat(40)));
        assert!(is_object_id(&"0".repeat(64)));
        for bad in ["", "abc", &"A".repeat(40), &"g".repeat(40), &"a".repeat(41)] {
            assert!(!is_object_id(bad), "{bad:?}");
        }
    }

    #[test]
    fn status_records_parse_and_an_unknown_one_is_refused() {
        let oid = "a".repeat(40);
        let stream = format!(
            "# branch.oid {oid}\0\
             1 .M N... 100644 100644 100644 {oid} {oid} a b.txt\0\
             2 R. N... 100644 100644 100644 {oid} {oid} R100 new.txt\0old.txt\0\
             ? loose dir/\0\
             ! ignored.env\0"
        );
        let entries = parse_status_entries(stream.as_bytes()).unwrap();
        assert_eq!(
            entries,
            vec![
                StatusEntry::Changed {
                    xy: ".M".to_owned(),
                    path: b"a b.txt".to_vec()
                },
                StatusEntry::Changed {
                    xy: "R.".to_owned(),
                    path: b"new.txt".to_vec()
                },
                StatusEntry::Untracked {
                    path: b"loose dir/".to_vec()
                },
                StatusEntry::Ignored {
                    path: b"ignored.env".to_vec()
                },
            ]
        );
        assert!(parse_status_entries(b"x what\0").is_err());
    }

    #[test]
    fn a_lowercase_tag_or_skip_worktree_is_hidden() {
        let hidden = parse_hidden(b"H a\0h b\0S c\0s d\0M e\0").unwrap();
        assert_eq!(hidden, vec![b"b".to_vec(), b"c".to_vec(), b"d".to_vec()]);
        assert!(parse_hidden(b"Hx\0").is_err());
    }
}
