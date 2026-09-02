//! §3.3's history rows.
//!
//! The root-set walk traverses the reachable graph and is the expensive one; the authorship
//! walk measured ~30 ms across a real corpus, which is why §4.1a runs it before J2 and J3.
//! Authorship is keyed on the **committer** (`%ce`) over the **full** history, with no cap:
//! v1's `%ae` and `-1000` misclassified long-lived repositories as Reference.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::io::BufRead;

use crate::cancel::CancelToken;

use super::error::{GitError, GitResult};
use super::exec::{GitExec, RunLimits};
use super::repo::RepoHandle;

/// A root commit and the local day it happened on.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RootCommit {
    /// Its object id.
    pub oid: String,
    /// `project.first_commit_at`, epoch seconds.
    pub committed_at: i64,
    /// `project.first_commit_tz_offset_min` — a day is the local day, and epoch seconds alone
    /// cannot preserve it.
    pub tz_offset_min: i32,
}

/// One committer's contribution to a repository.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CommitterTally {
    /// The committer email as git recorded it.
    pub email: String,
    /// `project_committer.commits` (§1.4). Nothing renders this; no XP is derived from it.
    pub commits: u32,
    /// Distinct local days this committer committed on — J4's actual output.
    pub days: BTreeSet<i64>,
    /// The newest commit this address made, epoch seconds.
    ///
    /// §5.1's `last_user_commit_at` is "the last commit by a *user identity*", which is not
    /// `last_commit_at` and cannot be recovered from `days`: a day number is a day, and the
    /// column is a second. The walk already parses the timestamp, so carrying it costs nothing.
    pub last_commit_at: i64,
}

/// The full-history committer walk.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Authorship {
    /// One entry per committer email, ordered by email.
    pub committers: Vec<CommitterTally>,
}

/// One commit's subject line.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CommitSubject {
    /// Its object id.
    pub oid: String,
    /// Committer date, epoch seconds. `last_commit_at` is the newest of these (§5.1).
    pub committed_at: i64,
    /// The subject, which may itself contain tabs.
    pub subject: String,
}

/// Parse git's `%az` / `%cz` form, `+HHMM`, into minutes east of UTC.
#[must_use]
pub fn parse_tz_offset_min(tz: &str) -> Option<i32> {
    let tz = tz.trim();
    let (sign, digits) = match tz.strip_prefix('+') {
        Some(rest) => (1, rest),
        None => (-1, tz.strip_prefix('-')?),
    };
    if digits.len() != 4 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let hours = digits.get(0..2)?.parse::<i32>().ok()?;
    let minutes = digits.get(2..4)?.parse::<i32>().ok()?;
    Some(sign * (hours * 60 + minutes))
}

/// The day number in the committer's own timezone.
#[must_use]
pub fn local_day(epoch_seconds: i64, tz_offset_min: i32) -> i64 {
    (epoch_seconds + i64::from(tz_offset_min) * 60).div_euclid(86_400)
}

/// The `+HHMM` offset out of git's `%ai` form, `YYYY-MM-DD HH:MM:SS +HHMM`.
///
/// **git has no `%az` or `%cz` placeholder** — it emits the literal text `%az`, which parses as
/// no offset at all, so every root date and every commit-day silently disappeared. The offset
/// has to be taken off the ISO-like date instead. A format string that quietly prints itself is
/// why this is a named helper rather than a literal at three call sites.
fn tz_offset_from_iso_date(iso: &str) -> Option<i32> {
    parse_tz_offset_min(iso.trim().rsplit(' ').next()?)
}

/// True when git's complaint means "this repository has no commits yet".
fn is_empty_history(err: &GitError) -> bool {
    let GitError::Unreadable { detail } = err else {
        return false;
    };
    let low = detail.to_ascii_lowercase();
    low.contains("does not have any commits yet")
        || low.contains("unknown revision or path not in the working tree")
        || low.contains("ambiguous argument 'head'")
        || low.contains("bad revision")
}

/// `rev-list --max-parents=0 HEAD`, then one `show` for their dates.
pub fn root_commits(
    exec: &GitExec,
    repo: &RepoHandle,
    limits: RunLimits,
    cancel: &CancelToken,
) -> GitResult<Vec<RootCommit>> {
    let listed = match exec.run(
        repo,
        &[
            OsStr::new("rev-list"),
            OsStr::new("--max-parents=0"),
            OsStr::new("HEAD"),
        ],
        limits,
        cancel,
    ) {
        Ok(out) => out,
        Err(e) if is_empty_history(&e) => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let text = String::from_utf8_lossy(&listed.stdout);
    let oids: Vec<String> = text
        .lines()
        .map(|l| l.trim().to_owned())
        .filter(|l| !l.is_empty())
        .collect();
    if oids.is_empty() {
        return Ok(Vec::new());
    }

    let mut args: Vec<&OsStr> = vec![
        OsStr::new("show"),
        OsStr::new("--no-patch"),
        OsStr::new("--format=%H%x09%at%x09%ai"),
    ];
    args.extend(oids.iter().map(|o| OsStr::new(o.as_str())));
    let dated = exec.run(repo, &args, limits, cancel)?;

    let text = String::from_utf8_lossy(&dated.stdout);
    let mut out = Vec::new();
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let mut parts = line.split('\t');
        let (Some(oid), Some(at), Some(tz)) = (parts.next(), parts.next(), parts.next()) else {
            continue;
        };
        let (Some(committed_at), Some(tz_offset_min)) =
            (at.trim().parse::<i64>().ok(), tz_offset_from_iso_date(tz))
        else {
            continue;
        };
        out.push(RootCommit {
            oid: oid.trim().to_owned(),
            committed_at,
            tz_offset_min,
        });
    }
    Ok(out)
}

/// The full committer walk, streamed rather than buffered: a million-commit history is ~70 MB
/// of output and nothing needs it in memory at once.
pub fn authorship(
    exec: &GitExec,
    repo: &RepoHandle,
    limits: RunLimits,
    cancel: &CancelToken,
) -> GitResult<Authorship> {
    let tallies = exec.run_piped(
        repo,
        &[
            OsStr::new("log"),
            OsStr::new("--no-merges"),
            OsStr::new("--format=%H%x09%ce%x09%at%x09%ai"),
        ],
        limits,
        cancel,
        |_stdin| Ok(()),
        |stdout: &mut dyn BufRead| {
            let mut acc: BTreeMap<String, (u32, BTreeSet<i64>, i64)> = BTreeMap::new();
            let mut line = String::new();
            loop {
                line.clear();
                if stdout.read_line(&mut line)? == 0 {
                    break;
                }
                let mut parts = line.trim_end_matches('\n').split('\t');
                let (Some(_oid), Some(email), Some(at), Some(tz)) =
                    (parts.next(), parts.next(), parts.next(), parts.next())
                else {
                    continue;
                };
                let (Some(at), Some(tz)) =
                    (at.trim().parse::<i64>().ok(), tz_offset_from_iso_date(tz))
                else {
                    continue;
                };
                let entry = acc
                    .entry(email.to_owned())
                    .or_insert((0, BTreeSet::new(), i64::MIN));
                entry.0 = entry.0.saturating_add(1);
                entry.1.insert(local_day(at, tz));
                entry.2 = entry.2.max(at);
            }
            Ok(acc)
        },
    );

    let tallies = match tallies {
        Ok(t) => t,
        Err(e) if is_empty_history(&e) => return Ok(Authorship::default()),
        Err(e) => return Err(e),
    };
    Ok(Authorship {
        committers: tallies
            .into_iter()
            .map(|(email, (commits, days, last_commit_at))| CommitterTally {
                email,
                commits,
                days,
                last_commit_at,
            })
            .collect(),
    })
}

/// `log --format=%H%x09%ct%x09%s -n <limit>`, newest first.
pub fn commit_subjects(
    exec: &GitExec,
    repo: &RepoHandle,
    limit: u32,
    limits: RunLimits,
    cancel: &CancelToken,
) -> GitResult<Vec<CommitSubject>> {
    let count = format!("-n{limit}");
    let out = match exec.run(
        repo,
        &[
            OsStr::new("log"),
            OsStr::new(&count),
            OsStr::new("--format=%H%x09%ct%x09%s"),
        ],
        limits,
        cancel,
    ) {
        Ok(out) => out,
        Err(e) if is_empty_history(&e) => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let mut subjects = Vec::new();
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let Some((oid, rest)) = line.split_once('\t') else {
            continue;
        };
        let Some((at, subject)) = rest.split_once('\t') else {
            continue;
        };
        let Ok(committed_at) = at.trim().parse::<i64>() else {
            continue;
        };
        subjects.push(CommitSubject {
            oid: oid.trim().to_owned(),
            committed_at,
            subject: subject.to_owned(),
        });
    }
    Ok(subjects)
}
