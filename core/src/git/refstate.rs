//! §3.3's first row and §6's basis: ref state read straight off the filesystem.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use sha2::{Digest as _, Sha256};

use crate::cancel::CancelToken;
use crate::clock::Clock;

use super::error::{GitError, GitResult};
use super::exec::{GitExec, RunLimits};
use super::repo::RepoHandle;

// [superseded by R5] a 128-bit FNV-1a hasher used to live here and both fingerprints were
// `u128`. `location.refstate_basis` is a `TEXT` column, and `serde_json` cannot carry a `u128`
// outside `u64` range without `arbitrary_precision` — so the digest is SHA-256 and the value is
// its lowercase hex. Plan 09's `BasisInputs` feeds the same computation.

/// Feed one labelled `u128` — an mtime in nanoseconds, or a length — into the digest.
fn write_u128(h: &mut Sha256, v: u128) {
    h.update(v.to_le_bytes());
}

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

/// The ref basis plus the index, used only by §3.5's torn-read guard. Never stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservationFingerprint(String);

/// An operation git left half-finished. §3.3 reads only `MERGE_HEAD` and `REBASE_HEAD`, so
/// phase 1 stores only these two (§5.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterruptedOp {
    /// A merge is open.
    Merge,
    /// A rebase is open.
    Rebase,
}

impl InterruptedOp {
    /// The lowercase word §5.6 puts in its sentence.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Merge => "merge",
            Self::Rebase => "rebase",
        }
    }
}

/// The branch's configured upstream and where it currently points.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpstreamRef {
    /// `branch.<name>.remote`.
    pub remote: String,
    /// The full remote-tracking ref name.
    pub ref_name: String,
    /// Its tip, or `None` when the tracking ref does not exist locally.
    pub oid: Option<String>,
}

/// Everything J1 produces from file reads.
#[derive(Debug, Clone)]
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
    pub stash_count: u32,
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

fn packed_refs(common_dir: &Path) -> BTreeMap<String, String> {
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

/// The §6 tuple, fed into a SHA-256 that the caller finishes (**R5**).
fn base_digest(repo: &RepoHandle) -> Sha256 {
    let mut h = Sha256::new();
    h.update(b"head:");
    h.update(
        read_trimmed(&repo.git_dir.join("HEAD"))
            .unwrap_or_default()
            .as_bytes(),
    );
    h.update(b"packed:");
    write_u128(
        &mut h,
        mtime_nanos(&repo.common_dir.join("packed-refs")).unwrap_or(0),
    );
    h.update(b"refs:");
    for (name, (nanos, len)) in loose_refs(&repo.common_dir) {
        h.update(name.as_bytes());
        write_u128(&mut h, nanos);
        write_u128(&mut h, u128::from(len));
    }
    h.update(b"fetchhead:");
    write_u128(
        &mut h,
        mtime_nanos(&repo.common_dir.join("FETCH_HEAD")).unwrap_or(0),
    );
    h.update(b"config:");
    write_u128(
        &mut h,
        mtime_nanos(&repo.common_dir.join("config")).unwrap_or(0),
    );
    h.update(b"shallow:");
    h.update(if repo.common_dir.join("shallow").exists() {
        b"1"
    } else {
        b"0"
    });
    h.update(b"markers:");
    for marker in [
        "MERGE_HEAD",
        "REBASE_HEAD",
        "CHERRY_PICK_HEAD",
        "REVERT_HEAD",
        "BISECT_LOG",
    ] {
        h.update(if repo.git_dir.join(marker).exists() {
            b"1"
        } else {
            b"0"
        });
    }
    for dir in ["rebase-merge", "rebase-apply"] {
        h.update(if repo.git_dir.join(dir).is_dir() {
            b"1"
        } else {
            b"0"
        });
    }
    h.update(b"stash:");
    write_u128(
        &mut h,
        mtime_nanos(&repo.common_dir.join("logs").join("refs").join("stash")).unwrap_or(0),
    );
    h
}

/// §6's basis. Cheap: stats, not reads, for everything but `HEAD`.
pub fn ref_fingerprint(repo: &RepoHandle) -> GitResult<RefFingerprint> {
    if !repo.git_dir.exists() {
        return Err(GitError::PathGone {
            detail: "git dir is not there".to_owned(),
        });
    }
    Ok(RefFingerprint(format!(
        "{:x}",
        base_digest(repo).finalize()
    )))
}

/// The basis plus the index, for §3.5's before/after comparison only.
pub fn observation_fingerprint(repo: &RepoHandle) -> GitResult<ObservationFingerprint> {
    if !repo.git_dir.exists() {
        return Err(GitError::PathGone {
            detail: "git dir is not there".to_owned(),
        });
    }
    let mut h = base_digest(repo);
    let index = repo.git_dir.join("index");
    h.update(b"index:");
    write_u128(&mut h, mtime_nanos(&index).unwrap_or(0));
    write_u128(
        &mut h,
        u128::from(std::fs::metadata(&index).map_or(0, |m| m.len())),
    );
    Ok(ObservationFingerprint(format!("{:x}", h.finalize())))
}

/// Read every J1 fact from the filesystem.
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

    let stash_count =
        std::fs::read_to_string(repo.common_dir.join("logs").join("refs").join("stash"))
            .map_or(0, |t| t.lines().filter(|l| !l.trim().is_empty()).count());

    Ok(RefState {
        head_oid,
        branch,
        upstream,
        // R5: a graph walk, not a file read. `divergence` (Task 7) fills these in; leaving them
        // `None` here is the invariant, not a gap — unknown is never rendered as zero.
        ahead: None,
        behind: None,
        tag_count: u32::try_from(tag_names.len()).unwrap_or(u32::MAX),
        stash_count: u32::try_from(stash_count).unwrap_or(u32::MAX),
        is_shallow: repo.common_dir.join("shallow").exists(),
        is_bare: config_is_bare(&repo.common_dir),
        interrupted_op: interrupted(&repo.git_dir),
        fetch_head_at: mtime_secs(&repo.common_dir.join("FETCH_HEAD")),
        reflog_tail_at: mtime_secs(&repo.git_dir.join("logs").join("HEAD")),
        basis,
        observed_at: clock.now_unix(),
    })
}

/// `location.ahead` / `location.behind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
