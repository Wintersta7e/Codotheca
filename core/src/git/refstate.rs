//! §3.3's first row and §6's basis: ref state read straight off the filesystem.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use sha2::Digest as _;

use crate::cancel::CancelToken;
use crate::clock::Clock;

use super::error::{GitError, GitResult};
use super::exec::{GitExec, RunLimits};
use super::repo::RepoHandle;

// [superseded by R5] a 128-bit FNV-1a hasher used to live here and both fingerprints were
// `u128`. `location.refstate_basis` is a `TEXT` column, and `serde_json` cannot carry a `u128`
// outside `u64` range without `arbitrary_precision` — so the digest is SHA-256 and the value is
// its lowercase hex. Plan 09's `BasisInputs` feeds the same computation.

/// §6's ref-state basis, stored as `location.refstate_basis`.
///
/// A digest, and a digest has no age: nothing may recover a time from it (§6, §1.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefFingerprint(String);

impl RefFingerprint {
    /// Build one from a computed digest, rejecting anything that is not 64 lowercase hex
    /// characters.
    ///
    /// This is the **only** way to make a `RefFingerprint` outside this module, and it validates
    /// on purpose: plan 09's `compute_basis` produces the digest, plan 04 reads the value back
    /// out of `refstate_basis TEXT`, and plan 18 parses it off the wire. A public tuple field
    /// would let any of the three store a string that is not a digest, and the freshness
    /// comparison this type exists to drive would then quietly never match.
    #[must_use]
    pub fn from_hex(s: &str) -> Option<Self> {
        let ok = s.len() == 64
            && s.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        ok.then(|| Self(s.to_owned()))
    }

    /// The digest producer's constructor, for `core::freshness::compute_basis` and nothing else.
    ///
    /// Deliberately not public. `from_hex` stays the only door for a value that arrives from the
    /// database or the wire, which is where the validation is load-bearing; a freshly finalised
    /// SHA-256 formatted with `{:x}` cannot fail it, and routing it through an `Option` would
    /// force a fallback branch that can never be taken.
    pub(crate) fn from_digest(digest: impl std::fmt::LowerHex) -> Self {
        Self(format!("{digest:x}"))
    }

    /// The 64-character lowercase hex form written to the database.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The same value, owned. §13's worker sends the basis as hex, and `from_hex` reads it back.
    #[must_use]
    pub fn to_hex(&self) -> String {
        self.0.clone()
    }
}

// §13: the basis crosses the wire out of the distro. Both impls are hand-written because the
// wire form must be the bare hex string `refstate_basis TEXT` holds and not a wrapper object,
// and because a value that is not a digest has to be refused *on the way in* — a derived
// `Deserialize` would build one and let it reach the column, and the freshness comparison this
// type exists to drive would then quietly never match. `from_hex` is the one validator.
impl serde::Serialize for RefFingerprint {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> serde::Deserialize<'de> for RefFingerprint {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = <String as serde::Deserialize>::deserialize(deserializer)?;
        Self::from_hex(&raw)
            .ok_or_else(|| serde::de::Error::custom("not a hexadecimal fingerprint"))
    }
}

/// The ref basis plus the index, used only by §3.5's torn-read guard. Never stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservationFingerprint(String);

/// An operation git left half-finished. §3.3 reads only `MERGE_HEAD` and `REBASE_HEAD`, so
/// phase 1 stores only these two (§5.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InterruptedOp {
    /// A merge is open.
    Merge,
    /// A rebase is open.
    Rebase,
}

impl InterruptedOp {
    /// The lowercase word §5.6 puts in its sentence.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Merge => "merge",
            Self::Rebase => "rebase",
        }
    }
}

/// The branch's configured upstream and where it currently points.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct UpstreamRef {
    /// `branch.<name>.remote`.
    pub remote: String,
    /// The full remote-tracking ref name.
    pub ref_name: String,
    /// Its tip, or `None` when the tracking ref does not exist locally.
    pub oid: Option<String>,
}

/// Everything J1 produces from file reads.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RefState {
    /// `location.head_oid`. `None` on a bare or unborn HEAD — not computed, never a zero oid.
    pub head_oid: Option<String>,
    /// `location.branch`. `None` when HEAD is detached.
    pub branch: Option<String>,
    /// The upstream pair [`divergence`] needs (§3.3).
    pub upstream: Option<UpstreamRef>,
    /// **R5** `location.ahead`. `None` until [`divergence`] has counted it — file reads cannot,
    /// and an uncounted ahead is *not computed*, never `0` (§8.5.2).
    pub ahead: Option<u32>,
    /// **R5** `location.behind`, with the same rule. §5.1 and §7.7 omit the chip entirely when
    /// this is `None`; they never draw `BEHIND 0`.
    pub behind: Option<u32>,
    /// Count of refs under `refs/tags`.
    pub tag_count: u32,
    /// `location.stash_count`, from the stash reflog.
    ///
    /// **[p2-24b] R51: `None` is *unreadable*, not zero.** A stash reflog the app cannot read is
    /// not an empty one, and a deletion gate that read `0` there would clear a copy holding work.
    /// `Some(0)` is *no stash* and is a real observation: git creates the reflog on the first
    /// stash, so its absence genuinely means none.
    pub stash_count: Option<u32>,
    /// `project.is_shallow`.
    pub is_shallow: bool,
    /// `project.is_bare`.
    pub is_bare: bool,
    /// `location.interrupted_op`.
    pub interrupted_op: Option<InterruptedOp>,
    /// `location.fetch_head_at`. `None` means no fetch recorded — never `0`, never an age.
    pub fetch_head_at: Option<i64>,
    /// **R5** `location.reflog_tail_at`: the mtime of `logs/HEAD`, which §5.1 uses as the last
    /// sign of local activity. `None` means the reflog is absent, never "long ago".
    pub reflog_tail_at: Option<i64>,
    /// `location.refstate_basis`, a lowercase-hex SHA-256 (**R5**).
    pub basis: RefFingerprint,
    /// `location.refstate_observed_at` — when the app looked, never when the user stopped.
    pub observed_at: i64,
    /// §39.4: the user's own push record, `location.push_record` and its two push columns.
    pub push: PushObservation,
}

/// §39.4 — exactly the message git writes into a pusher's remote-tracking reflog. Measured on
/// git 2.43.0 and pinned against the running git by `ship_evidence.rs`.
pub const PUSH_REFLOG_MESSAGE: &str = "update by push";

/// `location.push_record`: what the push-record reader could say about one copy (§39.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PushRecord {
    /// The remote-tracking reflogs were read; a push may or may not be among them.
    Read,
    /// The config names no remote and no remote-tracking ref exists.
    NoRemote,
    /// Ref logging is off, so a push would have left no line to read.
    Absent,
    /// An I/O error on the config, a ref or a reflog file.
    Unreadable,
    /// The config declares a ref backend other than `files`, which keeps no `logs/`.
    Unsupported,
}

impl PushRecord {
    /// Every record, in the column's CHECK order.
    pub const ALL: [Self; 5] = [
        Self::Read,
        Self::NoRemote,
        Self::Absent,
        Self::Unreadable,
        Self::Unsupported,
    ];

    /// The column's spelling.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::NoRemote => "no_remote",
            Self::Absent => "absent",
            Self::Unreadable => "unreadable",
            Self::Unsupported => "unsupported",
        }
    }
}

/// One reading of a copy's push record.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PushObservation {
    /// Which row of §39.4's table matched.
    pub record: PushRecord,
    /// The newest `update by push` line's timestamp, in its own frame. `Some` only under `Read`.
    pub last_push_at: Option<i64>,
    /// That line's own offset, minutes east. `Some` iff `last_push_at` is.
    pub last_push_tz_offset_min: Option<i32>,
}

impl PushObservation {
    /// A record that carries no push: every row but `Read`, and `Read` with no push line.
    #[must_use]
    pub const fn without_push(record: PushRecord) -> Self {
        Self {
            record,
            last_push_at: None,
            last_push_tz_offset_min: None,
        }
    }
}

/// An I/O error the push-record reader turns into `unreadable`.
struct PushUnreadable;

impl From<std::io::Error> for PushUnreadable {
    fn from(_: std::io::Error) -> Self {
        Self
    }
}

/// The three facts the push-record reader takes from the repository config.
#[derive(Default)]
struct PushConfig {
    ref_storage: Option<String>,
    log_all_ref_updates: Option<bool>,
    has_remote: bool,
}

/// git's boolean spelling: `false`/`no`/`off`/`0` are false; a bare key, `always` and every other
/// value are true.
fn git_bool(value: Option<&str>) -> bool {
    value.is_none_or(|v| {
        !["false", "no", "off", "0"]
            .iter()
            .any(|f| v.eq_ignore_ascii_case(f))
    })
}

/// `extensions.refStorage`, `core.logAllRefUpdates` and whether a `[remote …]` section exists, in
/// `config_is_bare`'s idiom: section and key names compared case-insensitively, the last
/// assignment winning. **`include` and `includeIf` are not followed** — a setting delivered that
/// way is what the `absent` row's second clause catches.
fn parse_push_config(text: &str) -> PushConfig {
    let mut out = PushConfig::default();
    let mut section = String::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some(header) = line.strip_prefix('[') {
            let name = header
                .split(|c: char| c == ']' || c == '.' || c.is_whitespace())
                .next()
                .unwrap_or_default();
            section = name.to_ascii_lowercase();
            out.has_remote |= section == "remote";
            continue;
        }
        let (key, value) = match line.split_once('=') {
            Some((k, v)) => {
                let v = v.split(['#', ';']).next().unwrap_or_default().trim();
                (k.trim(), Some(v.trim_matches('"')))
            }
            None => (line, None),
        };
        if section == "extensions" && key.eq_ignore_ascii_case("refstorage") {
            out.ref_storage = value.map(str::to_owned);
        } else if section == "core" && key.eq_ignore_ascii_case("logallrefupdates") {
            out.log_all_ref_updates = Some(git_bool(value));
        }
    }
    out
}

/// Every remote-tracking ref, loose and packed. Unlike [`loose_refs`] the walk reports its I/O
/// errors: a ref the reader could not list is `unreadable`, never *no ref*.
fn tracking_refs(common_dir: &Path) -> Result<Vec<String>, PushUnreadable> {
    fn walk(
        base: &Path,
        dir: &Path,
        depth: u32,
        out: &mut Vec<String>,
    ) -> Result<(), PushUnreadable> {
        if depth > 16 {
            return Ok(());
        }
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(e) if depth == 0 && e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e.into()),
        };
        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            if entry.metadata()?.is_dir() {
                walk(base, &path, depth + 1, out)?;
            } else if let Ok(rel) = path.strip_prefix(base) {
                out.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    walk(
        common_dir,
        &common_dir.join("refs").join("remotes"),
        0,
        &mut out,
    )?;
    match std::fs::read_to_string(common_dir.join("packed-refs")) {
        Ok(text) => out.extend(
            text.lines()
                .filter(|line| !line.starts_with('#') && !line.starts_with('^'))
                .filter_map(|line| line.split_once(' '))
                .map(|(_, name)| name.trim())
                .filter(|name| name.starts_with("refs/remotes/"))
                .map(str::to_owned),
        ),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    out.sort_unstable();
    out.dedup();
    Ok(out)
}

/// One reflog line's `(timestamp, offset)` when its message is exactly [`PUSH_REFLOG_MESSAGE`].
///
/// The header is `<old> <new> <identity> <timestamp> <+HHMM>` and the identity may hold spaces, so
/// the stamp is read from its last two tokens. A malformed line answers `None`.
fn push_line_stamp(line: &str) -> Option<(i64, i32)> {
    let (header, message) = line.split_once('\t')?;
    if message.trim_end_matches(['\r', '\n']) != PUSH_REFLOG_MESSAGE {
        return None;
    }
    let mut tokens = header.split_whitespace().rev();
    let tz = super::history::parse_tz_offset_min(tokens.next()?)?;
    let at = tokens.next()?.parse::<i64>().ok()?;
    Some((at, tz))
}

/// `logs/<name>` for a full ref name.
fn reflog_path(common_dir: &Path, name: &str) -> PathBuf {
    let mut path = common_dir.join("logs");
    for part in name.split('/') {
        path.push(part);
    }
    path
}

fn push_record_or_unreadable(common_dir: &Path) -> Result<PushObservation, PushUnreadable> {
    let config = parse_push_config(&std::fs::read_to_string(common_dir.join("config"))?);
    if config
        .ref_storage
        .as_deref()
        .is_some_and(|backend| !backend.eq_ignore_ascii_case("files"))
    {
        return Ok(PushObservation::without_push(PushRecord::Unsupported));
    }
    // Every read happens before any verdict, so an I/O error anywhere wins over the rows below
    // it, as §39.4's first-match order says.
    let refs = tracking_refs(common_dir)?;
    let mut logged = 0_usize;
    let mut newest: Option<(i64, i32)> = None;
    for name in &refs {
        let bytes = match std::fs::read(reflog_path(common_dir, name)) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e.into()),
        };
        logged += 1;
        for line in String::from_utf8_lossy(&bytes).split('\n') {
            if let Some(stamp) = push_line_stamp(line) {
                if newest.is_none_or(|(at, _)| stamp.0 > at) {
                    newest = Some(stamp);
                }
            }
        }
    }
    if !config.has_remote && refs.is_empty() {
        return Ok(PushObservation::without_push(PushRecord::NoRemote));
    }
    if config.log_all_ref_updates == Some(false) || (!refs.is_empty() && logged == 0) {
        return Ok(PushObservation::without_push(PushRecord::Absent));
    }
    Ok(PushObservation {
        record: PushRecord::Read,
        last_push_at: newest.map(|(at, _)| at),
        last_push_tz_offset_min: newest.map(|(_, tz)| tz),
    })
}

/// §39.4's record, first match: `unsupported`, `unreadable`, `no_remote`, `absent`, `read`.
///
/// File reads only — no spawn and no git verb. Any I/O error on the config (a missing one
/// included: a repository that cannot say which ref backend it uses cannot claim *absent*), on a
/// remote-tracking ref or on a reflog file is `unreadable`.
#[must_use]
pub fn read_push_record(common_dir: &Path) -> PushObservation {
    push_record_or_unreadable(common_dir)
        .unwrap_or_else(|PushUnreadable| PushObservation::without_push(PushRecord::Unreadable))
}

fn read_trimmed(path: &Path) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_owned())
}

fn mtime_nanos(path: &Path) -> Option<u128> {
    let meta = std::fs::metadata(path).ok()?;
    let modified = meta.modified().ok()?;
    modified
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_nanos())
}

fn mtime_secs(path: &Path) -> Option<i64> {
    let meta = std::fs::metadata(path).ok()?;
    let modified = meta.modified().ok()?;
    let d = modified.duration_since(UNIX_EPOCH).ok()?;
    i64::try_from(d.as_secs()).ok()
}

/// Every loose ref under `refs/`, as `name -> (mtime_nanos, len)`, bounded in depth.
fn loose_refs(common_dir: &Path) -> BTreeMap<String, (u128, u64)> {
    fn walk(base: &Path, dir: &Path, depth: u32, out: &mut BTreeMap<String, (u128, u64)>) {
        if depth > 16 {
            return;
        }
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            if meta.is_dir() {
                walk(base, &path, depth + 1, out);
            } else if let Ok(rel) = path.strip_prefix(base) {
                let name = rel.to_string_lossy().replace('\\', "/");
                let nanos = meta
                    .modified()
                    .ok()
                    .and_then(|m| m.duration_since(UNIX_EPOCH).ok())
                    .map_or(0, |d| d.as_nanos());
                out.insert(name, (nanos, meta.len()));
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(common_dir, &common_dir.join("refs"), 0, &mut out);
    out
}

/// `packed-refs` as a map. **`pub(crate)` so §24.7A's stash reader uses this parser rather than
/// a second one** — two parsers for one file is the one-value-twice defect on the file that says
/// whether a stash exists.
pub(crate) fn packed_refs(common_dir: &Path) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let Ok(text) = std::fs::read_to_string(common_dir.join("packed-refs")) else {
        return out;
    };
    for line in text.lines() {
        if line.starts_with('#') || line.starts_with('^') {
            continue;
        }
        if let Some((oid, name)) = line.split_once(' ') {
            out.insert(name.trim().to_owned(), oid.trim().to_owned());
        }
    }
    out
}

/// Every local ref under `refs/heads`, `refs/tags` and `refs/notes`, loose **and** packed.
///
/// §24.7A checks all three: a deletion gate that read only `refs/heads` would clear a release tag
/// or a note that exists nowhere else. `refs/remotes` is deliberately absent — those are the
/// things a local ref is checked *against*.
#[must_use]
pub fn local_ref_names(common_dir: &Path) -> Vec<String> {
    const WANTED: [&str; 3] = ["refs/heads/", "refs/tags/", "refs/notes/"];
    let mut names: std::collections::BTreeSet<String> = loose_refs(common_dir)
        .into_keys()
        .filter(|name| WANTED.iter().any(|prefix| name.starts_with(prefix)))
        .collect();
    names.extend(
        packed_refs(common_dir)
            .into_keys()
            .filter(|name| WANTED.iter().any(|prefix| name.starts_with(prefix))),
    );
    names.into_iter().collect()
}

fn resolve_ref(common_dir: &Path, packed: &BTreeMap<String, String>, name: &str) -> Option<String> {
    let mut rel = PathBuf::from(common_dir);
    for part in name.split('/') {
        rel.push(part);
    }
    read_trimmed(&rel)
        .filter(|s| !s.is_empty())
        .or_else(|| packed.get(name).cloned())
}

/// `[branch "<name>"] remote = …, merge = …` out of the repository config, without spawning git.
fn config_upstream(common_dir: &Path, branch: &str) -> Option<(String, String)> {
    let text = std::fs::read_to_string(common_dir.join("config")).ok()?;
    let want = format!("[branch \"{branch}\"]");
    let mut in_section = false;
    let (mut remote, mut merge) = (None, None);
    for raw in text.lines() {
        let line = raw.trim();
        if line.starts_with('[') {
            in_section = line == want;
            continue;
        }
        if !in_section {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            match k.trim() {
                "remote" => remote = Some(v.trim().to_owned()),
                "merge" => merge = Some(v.trim().to_owned()),
                _ => {}
            }
        }
    }
    Some((remote?, merge?))
}

fn config_is_bare(common_dir: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(common_dir.join("config")) else {
        return false;
    };
    let mut in_core = false;
    for raw in text.lines() {
        let line = raw.trim();
        if line.starts_with('[') {
            in_core = line.eq_ignore_ascii_case("[core]");
            continue;
        }
        if in_core {
            if let Some((k, v)) = line.split_once('=') {
                if k.trim().eq_ignore_ascii_case("bare") {
                    return v.trim().eq_ignore_ascii_case("true");
                }
            }
        }
    }
    false
}

fn interrupted(git_dir: &Path) -> Option<InterruptedOp> {
    if git_dir.join("MERGE_HEAD").exists() {
        return Some(InterruptedOp::Merge);
    }
    if git_dir.join("REBASE_HEAD").exists()
        || git_dir.join("rebase-merge").is_dir()
        || git_dir.join("rebase-apply").is_dir()
    {
        return Some(InterruptedOp::Rebase);
    }
    None
}

/// The stash reflog's entry count, or `None` when it could not be read.
///
/// **[p2-24b] R51: an unreadable stash reflog costs one field, not seven.** Before this, a
/// permission error here returned `Err` from the whole of `read_ref_state`, so `head_oid`,
/// `branch`, `tag_count`, `is_shallow`, `interrupted_op`, `fetch_head_at` and `reflog_tail_at`
/// were all lost to one unreadable file. They are independent facts and are now reported.
///
/// `None` is never collapsed to `0` anywhere downstream: `persist` writes NULL, and §24.8's
/// `stash_unreadable` blocker is what a deletion gate sees instead of a false all-clear.
/// **It cannot fail.** The `Result` it used to return was the mechanism by which one unreadable
/// file lost six other facts; removing it is what makes that impossible rather than merely
/// unlikely.
fn read_stash_count(common_dir: &Path) -> Option<usize> {
    let path = common_dir.join("logs").join("refs").join("stash");
    match std::fs::read_to_string(path) {
        Ok(text) => Some(text.lines().filter(|line| !line.trim().is_empty()).count()),
        // git creates the reflog on the first stash, so missing genuinely means none.
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Some(0),
        // Unreadable, by permission or by anything else. Not an error for the whole read, and
        // not a zero.
        Err(_) => None,
    }
}

/// §6's basis. Cheap: stats, not reads, for everything but `HEAD`.
///
/// The digest itself lives in [`crate::freshness`], which is the only place it is computed.
/// This function's job is to know that a linked worktree keeps `HEAD` and its operation markers
/// in `git_dir` while refs, config and the stash reflog live in `common_dir` — a fact the pure
/// digest has no business carrying.
///
/// # Errors
///
/// `GitError::PathGone` when the git dir is not there; `GitError::Unreadable` when the basis
/// inputs cannot be read.
pub fn ref_fingerprint(repo: &RepoHandle) -> GitResult<RefFingerprint> {
    Ok(crate::freshness::compute_basis(&basis_inputs(repo)?))
}

fn basis_inputs(repo: &RepoHandle) -> GitResult<crate::freshness::BasisInputs> {
    if !repo.git_dir.exists() {
        return Err(GitError::PathGone {
            detail: "git dir is not there".to_owned(),
        });
    }
    crate::freshness::collect_basis_inputs_split(&repo.git_dir, &repo.common_dir).map_err(|e| {
        GitError::Unreadable {
            detail: e.to_string(),
        }
    })
}

/// The basis plus the index, for §3.5's before/after comparison only.
///
/// Never stored, so its exact bytes do not matter — only that it moves whenever the basis or the
/// index does. It extends the one basis digest instead of restating its inputs.
///
/// # Errors
///
/// As [`ref_fingerprint`]; the index's own stat is best-effort and never fails it.
pub fn observation_fingerprint(repo: &RepoHandle) -> GitResult<ObservationFingerprint> {
    let mut h = crate::freshness::basis_hasher(&basis_inputs(repo)?);
    let index = repo.git_dir.join("index");
    h.update(b"index:");
    h.update(mtime_nanos(&index).unwrap_or(0).to_le_bytes());
    h.update(
        std::fs::metadata(&index)
            .map_or(0, |m| m.len())
            .to_le_bytes(),
    );
    Ok(ObservationFingerprint(format!("{:x}", h.finalize())))
}

/// Read every J1 fact from the filesystem.
///
/// # Errors
///
/// As [`ref_fingerprint`]. Every other file is read best-effort and never fails the read.
pub fn read_ref_state(repo: &RepoHandle, clock: &dyn Clock) -> GitResult<RefState> {
    let basis = ref_fingerprint(repo)?;
    let head_raw = read_trimmed(&repo.git_dir.join("HEAD")).unwrap_or_default();
    let packed = packed_refs(&repo.common_dir);

    let (branch, head_oid) = match head_raw.strip_prefix("ref:") {
        Some(target) => {
            let full = target.trim();
            let short = full.strip_prefix("refs/heads/").unwrap_or(full).to_owned();
            (Some(short), resolve_ref(&repo.common_dir, &packed, full))
        }
        None if head_raw.is_empty() => (None, None),
        None => (None, Some(head_raw.clone())),
    };

    let upstream = branch.as_ref().and_then(|b| {
        let (remote, merge) = config_upstream(&repo.common_dir, b)?;
        let short = merge.strip_prefix("refs/heads/").unwrap_or(&merge);
        let ref_name = if remote == "." {
            merge.clone()
        } else {
            format!("refs/remotes/{remote}/{short}")
        };
        let oid = resolve_ref(&repo.common_dir, &packed, &ref_name);
        Some(UpstreamRef {
            remote,
            ref_name,
            oid,
        })
    });

    let mut tag_names: Vec<String> = loose_refs(&repo.common_dir)
        .keys()
        .filter_map(|n| n.strip_prefix("refs/tags/").map(str::to_owned))
        .collect();
    tag_names.extend(
        packed
            .keys()
            .filter_map(|n| n.strip_prefix("refs/tags/").map(str::to_owned)),
    );
    tag_names.sort_unstable();
    tag_names.dedup();

    let stash_count = read_stash_count(&repo.common_dir);

    Ok(RefState {
        head_oid,
        branch,
        upstream,
        // R5: a graph walk, not a file read. `divergence` (Task 7) fills these in; leaving them
        // `None` here is the invariant, not a gap — unknown is never rendered as zero.
        ahead: None,
        behind: None,
        tag_count: u32::try_from(tag_names.len()).unwrap_or(u32::MAX),
        stash_count: stash_count.map(|n| u32::try_from(n).unwrap_or(u32::MAX)),
        is_shallow: repo.common_dir.join("shallow").exists(),
        is_bare: config_is_bare(&repo.common_dir),
        interrupted_op: interrupted(&repo.git_dir),
        fetch_head_at: mtime_secs(&repo.common_dir.join("FETCH_HEAD")),
        reflog_tail_at: mtime_secs(&repo.git_dir.join("logs").join("HEAD")),
        basis,
        observed_at: clock.now_unix(),
        push: read_push_record(&repo.common_dir),
    })
}

/// `location.ahead` / `location.behind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Divergence {
    /// Commits on the branch that the upstream does not have.
    pub ahead: u32,
    /// Commits on the upstream that the branch does not have.
    pub behind: u32,
}

/// Count divergence against the configured upstream.
///
/// `Ok(None)` means *not computed* — no branch, no upstream, no local tracking ref, or an
/// unborn HEAD. It is never rendered as a zero (§7.7, §1.10).
///
/// # Errors
///
/// The `rev-list` invocation's failure as [`GitExec::run_piped`] classifies it, or
/// `GitError::Internal` when its output is not two counts.
pub fn divergence(
    exec: &GitExec,
    repo: &RepoHandle,
    state: &RefState,
    limits: RunLimits,
    cancel: &CancelToken,
) -> GitResult<Option<Divergence>> {
    let (Some(head), Some(up)) = (state.head_oid.as_ref(), state.upstream.as_ref()) else {
        return Ok(None);
    };
    let Some(up_oid) = up.oid.as_ref() else {
        return Ok(None);
    };
    if head == up_oid {
        return Ok(Some(Divergence {
            ahead: 0,
            behind: 0,
        }));
    }

    let range = format!("{up_oid}...{head}");
    let out = exec.run(
        repo,
        &[
            OsStr::new("rev-list"),
            OsStr::new("--count"),
            OsStr::new("--left-right"),
            OsStr::new(&range),
        ],
        limits,
        cancel,
    )?;
    let text = String::from_utf8_lossy(&out.stdout);
    // `--left-right` prints "<left>\t<right>"; left is the upstream side, so left is `behind`.
    let Some((left, right)) = text.trim().split_once('\t') else {
        return Err(GitError::Internal {
            detail: format!("unparseable rev-list --count output: {}", text.trim()),
        });
    };
    let behind = left.trim().parse::<u32>().ok();
    let ahead = right.trim().parse::<u32>().ok();
    match (ahead, behind) {
        (Some(ahead), Some(behind)) => Ok(Some(Divergence { ahead, behind })),
        _ => Err(GitError::Internal {
            detail: format!("non-numeric rev-list --count output: {}", text.trim()),
        }),
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
mod tests {
    use super::*;

    const OLD: &str = "0000000000000000000000000000000000000000";
    const NEW: &str = "1111111111111111111111111111111111111111";

    #[test]
    fn an_identity_holding_spaces_still_yields_the_stamp() {
        let line = format!(
            "{OLD} {NEW} Jo Q. van Example <jo@example.invalid> 1790000000 +0530\tupdate by push"
        );
        assert_eq!(push_line_stamp(&line), Some((1_790_000_000, 330)));
    }

    #[test]
    fn a_crlf_ending_is_trimmed_from_the_message() {
        let line =
            format!("{OLD} {NEW} A <a@example.invalid> 1790000000 -0800\tupdate by push\r\n");
        assert_eq!(push_line_stamp(&line), Some((1_790_000_000, -480)));
    }

    #[test]
    fn a_malformed_line_is_skipped() {
        for line in [
            "update by push".to_owned(),
            format!("{OLD} {NEW} A <a@example.invalid> notatime +0000\tupdate by push"),
            format!("{OLD} {NEW} A <a@example.invalid> 1790000000 0530\tupdate by push"),
            String::new(),
        ] {
            assert_eq!(push_line_stamp(&line), None, "{line:?}");
        }
    }

    #[test]
    fn a_message_that_only_starts_with_the_push_message_is_not_a_push() {
        for message in [
            "update by push (forced)",
            "update by push2",
            "fetch: storing head",
        ] {
            let line = format!("{OLD} {NEW} A <a@example.invalid> 1790000000 +0000\t{message}");
            assert_eq!(push_line_stamp(&line), None, "{message}");
        }
    }

    #[test]
    fn the_config_reader_takes_the_three_facts_case_insensitively() {
        let config = parse_push_config(
            "[Core]\n\tLogAllRefUpdates = off ; a comment\n[Extensions]\n\trefStorage = reftable\n\
             [remote \"origin\"]\n\turl = x\n",
        );
        assert_eq!(config.log_all_ref_updates, Some(false));
        assert_eq!(config.ref_storage.as_deref(), Some("reftable"));
        assert!(config.has_remote);
        let bare_key = parse_push_config("[core]\n\tlogallrefupdates\n");
        assert_eq!(bare_key.log_all_ref_updates, Some(true));
        assert!(!bare_key.has_remote);
        assert_eq!(
            parse_push_config("[core]\n\tlogAllRefUpdates = always\n").log_all_ref_updates,
            Some(true)
        );
    }
}
