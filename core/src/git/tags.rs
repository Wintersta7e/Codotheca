//! §38.8's tag facts: every tag that peels to a commit, with the identity and the date a
//! `release` row is judged and dated by.
//!
//! Three reads after the ref enumeration, each one spawn whatever the number of tags: the peel,
//! the annotated tags' tagger lines, and the lightweight tags' commits. A read that fails is
//! `Err` — never a shorter list, because *absent* is not *unreadable*.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{OsStr, OsString};

use crate::cancel::CancelToken;

use super::analyse::{self, batch_check, text_of, unreadable};
use super::error::GitResult;
use super::exec::{GitExec, RunLimits};
use super::history::{parse_tz_offset_min, tz_offset_from_iso_date};
use super::repo::RepoHandle;

/// One tag that peels to a commit.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TagFact {
    /// The tag's name, without `refs/tags/`.
    pub name: String,
    /// The object the ref names: the tag object when annotated, the commit when lightweight.
    pub object_oid: String,
    /// The commit the tag peels to.
    pub commit_oid: String,
    /// Whose tag it is and when, by its kind.
    pub kind: TagKind,
}

/// A tag's identity and date: an annotated tag's own tagger, or a lightweight tag's commit.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum TagKind {
    /// A tag object, dated by its tagger in the tagger's zone.
    Annotated {
        /// The tagger line's email.
        tagger_email: String,
        /// The tagger date, epoch seconds.
        tagged_at: i64,
        /// The tagger's zone, minutes east of UTC.
        tz_offset_min: i32,
    },
    /// A ref straight to a commit, dated as the history job dates that commit's commit-day.
    Lightweight {
        /// The commit's committer email.
        committer_email: String,
        /// The commit's author date, epoch seconds.
        authored_at: i64,
        /// The author's zone, minutes east of UTC.
        tz_offset_min: i32,
    },
}

/// A tagger line's three fields.
struct Tagger {
    email: String,
    at: i64,
    tz_offset_min: i32,
}

/// Every tag of `repo` that peels to a commit, in ref-name byte order.
///
/// A tag that peels to a tree or a blob is not a release and is left out; so is an annotated
/// tag written without a tagger line or with one that does not parse, which names nobody it
/// could be judged by.
///
/// # Errors
/// The enumeration's failure, a tag whose peeled object is missing, a commit `show` did not date,
/// or output in any other shape.
pub(crate) fn release_tags(
    exec: &GitExec,
    repo: &RepoHandle,
    limits: RunLimits,
    cancel: &CancelToken,
) -> GitResult<Vec<TagFact>> {
    let listing = analyse::enumerate_refs(exec, repo, limits, cancel)?;
    let tags: Vec<_> = listing
        .refs
        .into_iter()
        .filter_map(|r| {
            let name = r.name.strip_prefix("refs/tags/")?.to_owned();
            Some((name, r))
        })
        .collect();

    // `^{}` rather than `^{commit}`: git answers `missing` for a tag on a tree and for a commit
    // that is gone alike, and only the second is unreadable.
    let no_replace = [("GIT_NO_REPLACE_OBJECTS", OsString::from("1"))];
    let queries: Vec<String> = tags
        .iter()
        .map(|(_, r)| format!("{}^{{}}", r.name))
        .collect();
    let peeled = batch_check(exec, repo, &queries, &no_replace, limits, cancel)?;
    let mut commits = Vec::new();
    for ((name, entry), answer) in tags.into_iter().zip(peeled) {
        let Some((oid, kind)) = answer else {
            return Err(unreadable(format!("tag {name} peels to a missing object")));
        };
        if kind == "commit" {
            commits.push((name, entry, oid));
        }
    }

    let annotated: Vec<String> = commits
        .iter()
        .filter(|(_, entry, _)| entry.object_type == "tag")
        .map(|(_, entry, _)| entry.oid.clone())
        .collect();
    let taggers = taggers(exec, repo, &annotated, &no_replace, limits, cancel)?;
    let lightweight: BTreeSet<String> = commits
        .iter()
        .filter(|(_, entry, _)| entry.object_type != "tag")
        .map(|(_, _, commit)| commit.clone())
        .collect();
    let dated = commit_dates(exec, repo, &lightweight, limits, cancel)?;

    let mut facts = Vec::with_capacity(commits.len());
    let mut taggers = taggers.into_iter();
    for (name, entry, commit_oid) in commits {
        let kind = if entry.object_type == "tag" {
            let Some(Tagger {
                email,
                at,
                tz_offset_min,
            }) = taggers.next().flatten()
            else {
                continue;
            };
            TagKind::Annotated {
                tagger_email: email,
                tagged_at: at,
                tz_offset_min,
            }
        } else {
            let Some((email, at, tz_offset_min)) = dated.get(&commit_oid).cloned() else {
                return Err(unreadable(format!("show did not date {commit_oid}")));
            };
            TagKind::Lightweight {
                committer_email: email,
                authored_at: at,
                tz_offset_min,
            }
        };
        facts.push(TagFact {
            name,
            object_oid: entry.oid,
            commit_oid,
            kind,
        });
    }
    Ok(facts)
}

/// `cat-file --batch` over tag objects: each one's tagger, or `None` for a tag with no tagger
/// line or one that does not parse. Read as bytes, sized by git's own header, because a tag message need not be UTF-8.
fn taggers(
    exec: &GitExec,
    repo: &RepoHandle,
    oids: &[String],
    env: &[(&str, OsString)],
    limits: RunLimits,
    cancel: &CancelToken,
) -> GitResult<Vec<Option<Tagger>>> {
    if oids.is_empty() {
        return Ok(Vec::new());
    }
    let mut stdin = Vec::new();
    for oid in oids {
        stdin.extend_from_slice(oid.as_bytes());
        stdin.push(b'\n');
    }
    let out = exec.run_with_stdin(
        repo,
        &[OsStr::new("cat-file"), OsStr::new("--batch")],
        stdin,
        env,
        limits,
        cancel,
    )?;
    let short = || unreadable("cat-file --batch ended before its last answer");
    let mut rest = out.stdout.as_slice();
    let mut found = Vec::with_capacity(oids.len());
    for oid in oids {
        let end = rest.iter().position(|&b| b == b'\n').ok_or_else(short)?;
        let (header, after) = rest.split_at(end);
        let header = text_of(header, "cat-file --batch")?;
        let size = match header.split(' ').collect::<Vec<_>>().as_slice() {
            [got, "tag", size] if got == oid => size.parse::<usize>().ok(),
            _ => None,
        }
        .ok_or_else(|| unreadable(format!("cat-file --batch answered {header:?} for {oid}")))?;
        let after = after.strip_prefix(b"\n").ok_or_else(short)?;
        let body = after.get(..size).ok_or_else(short)?;
        found.push(tagger_of(body));
        rest = after
            .get(size..)
            .and_then(|r| r.strip_prefix(b"\n"))
            .ok_or_else(short)?;
    }
    if !rest.is_empty() {
        return Err(unreadable(
            "cat-file --batch answered more than it was asked",
        ));
    }
    Ok(found)
}

/// The `tagger` line of a tag object's header, which ends at the first empty line. A line that
/// does not parse names nobody, as a missing one does: one malformed historic tag must not make
/// every other tag of the repository unreadable.
fn tagger_of(body: &[u8]) -> Option<Tagger> {
    body.split(|&b| b == b'\n')
        .take_while(|l| !l.is_empty())
        .find_map(|line| line.strip_prefix(b"tagger "))
        .and_then(parse_tagger)
}

/// `Name <email> <epoch> <+HHMM>`. The name is never read, so it need not be UTF-8.
fn parse_tagger(value: &[u8]) -> Option<Tagger> {
    let (ident, when) = value.split_at(value.iter().rposition(|&b| b == b'>')?);
    let (_, email) = ident.split_at(ident.iter().rposition(|&b| b == b'<')?);
    let email = std::str::from_utf8(email.strip_prefix(b"<")?).ok()?;
    let when = std::str::from_utf8(when.strip_prefix(b">")?).ok()?;
    let mut parts = when.split_whitespace();
    let at = parts.next()?.parse::<i64>().ok()?;
    let tz_offset_min = parse_tz_offset_min(parts.next()?)?;
    if parts.next().is_some() {
        return None;
    }
    Some(Tagger {
        email: email.to_owned(),
        at,
        tz_offset_min,
    })
}

/// `show` over `oids` in one spawn: each commit's committer email, author time and author zone,
/// the fields and the format the history job dates a commit-day by.
///
/// Signatures off and the output pinned to UTF-8: a repository's `log.showSignature` prints
/// verification lines into the output, and its `i18n.logOutputEncoding` re-codes the email.
fn commit_dates(
    exec: &GitExec,
    repo: &RepoHandle,
    oids: &BTreeSet<String>,
    limits: RunLimits,
    cancel: &CancelToken,
) -> GitResult<BTreeMap<String, (String, i64, i32)>> {
    if oids.is_empty() {
        return Ok(BTreeMap::new());
    }
    let mut stdin = Vec::new();
    for oid in oids {
        stdin.extend_from_slice(oid.as_bytes());
        stdin.push(b'\n');
    }
    let out = exec.run_with_stdin(
        repo,
        &[
            OsStr::new("show"),
            OsStr::new("--stdin"),
            OsStr::new("--no-patch"),
            OsStr::new("--no-show-signature"),
            OsStr::new("--encoding=UTF-8"),
            OsStr::new("--format=%H%x09%ce%x09%at%x09%ai"),
        ],
        stdin,
        &[],
        limits,
        cancel,
    )?;
    let text = text_of(&out.stdout, "show")?;
    let mut dated = BTreeMap::new();
    for line in text.lines().filter(|l| !l.is_empty()) {
        let parsed = match line.split('\t').collect::<Vec<_>>().as_slice() {
            [oid, email, at, iso] if oids.contains(*oid) => at
                .parse::<i64>()
                .ok()
                .zip(tz_offset_from_iso_date(iso))
                .map(|(at, tz)| ((*oid).to_owned(), ((*email).to_owned(), at, tz))),
            _ => None,
        };
        let (oid, date) = parsed.ok_or_else(|| unreadable(format!("show printed {line:?}")))?;
        dated.insert(oid, date);
    }
    if dated.len() != oids.len() {
        return Err(unreadable(format!(
            "show dated {} of {} commits",
            dated.len(),
            oids.len()
        )));
    }
    Ok(dated)
}
