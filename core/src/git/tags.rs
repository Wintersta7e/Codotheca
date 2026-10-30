//! §38.8's tag facts: every tag that peels to a commit, with the identity and the date a
//! `release` row is judged and dated by.
//!
//! One `for-each-ref` over `refs/tags` names every field the facts need and **no body atom**
//! (`contents`, `body`, `subject`, `trailers`, `raw`), so what git prints grows with the number of
//! tags and never with a tag message. A nested tag is peeled by `cat-file --batch-check`, which
//! reads headers only. A read that fails is `Err` — never a shorter list, because *absent* is not
//! *unreadable*: git exits 0 when it skips a broken ref, so a word on its diagnostic stream fails
//! the read, and under the files backend an error-strict walk must name the same refs, which
//! catches the ones git skips without a word.

use std::collections::BTreeSet;
use std::ffi::{OsStr, OsString};

use crate::cancel::CancelToken;

use super::analyse::{batch_check, is_object_id, quiet, unreadable, walked_tag_names};
use super::error::GitResult;
use super::exec::{GitExec, GitOutput, RunLimits};
use super::history::{parse_tz_offset_min, tz_offset_from_iso_date};
use super::repo::RepoHandle;

/// The namespace the release read lists.
const REFS_TAGS: &str = "refs/tags";

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

/// One line of the listing. The tagger field is empty for a lightweight tag and the commit
/// fields for an annotated one.
struct Listed<'a> {
    refname: &'a str,
    oid: &'a str,
    object_type: &'a str,
    peeled_oid: &'a str,
    peeled_type: &'a str,
    tagger: &'a [u8],
    committer_email: &'a [u8],
    author_date: &'a str,
}

/// Every tag of `repo` that peels to a commit, in ref-name byte order.
///
/// A tag that peels to a tree or a blob is not a release and is left out; so is an annotated
/// tag written without a tagger line or with one that does not parse, and a lightweight tag whose
/// committer email is not UTF-8, which name nobody they could be judged by.
///
/// # Errors
/// The listing's failure or any word on its diagnostic stream; under the files backend a ref the
/// walk cannot read or a name set that differs from git's; a tag whose peeled object is missing,
/// a commit git did not date, or output in any other shape.
pub(crate) fn release_tags(
    exec: &GitExec,
    repo: &RepoHandle,
    limits: RunLimits,
    cancel: &CancelToken,
) -> GitResult<Vec<TagFact>> {
    let out = listing(exec, repo, limits, cancel)?;
    quiet(&out, "for-each-ref")?;
    let listed = parse_listing(&out.stdout)?;
    if !repo.common_dir.join("reftable").is_dir() {
        let walked = walked_tag_names(&repo.common_dir)?;
        let named: BTreeSet<String> = listed.iter().map(|l| l.refname.to_owned()).collect();
        if walked != named {
            return Err(unreadable(format!(
                "git listed {} tags and the walk found {}; they differ by {:?}",
                named.len(),
                walked.len(),
                walked.symmetric_difference(&named).collect::<Vec<_>>()
            )));
        }
    }

    // A nested tag's `*objecttype` is the inner tag's. `^{}` rather than `^{commit}`: git answers
    // `missing` for a tag on a tree and for a commit that is gone alike, and only the second is
    // unreadable.
    let no_replace = [("GIT_NO_REPLACE_OBJECTS", OsString::from("1"))];
    let queries: Vec<String> = listed
        .iter()
        .filter(|l| l.object_type == "tag" && l.peeled_type == "tag")
        .map(|l| format!("{}^{{}}", l.refname))
        .collect();
    let mut peeled = batch_check(exec, repo, &queries, &no_replace, limits, cancel)?.into_iter();

    let mut facts = Vec::with_capacity(listed.len());
    for l in listed {
        if let Some(fact) = fact_of(&l, &mut peeled)? {
            facts.push(fact);
        }
    }
    Ok(facts)
}

/// One listed tag's fact, or `None` for a tag that is not a release or names nobody. `peeled`
/// holds the answers for the nested tags, in listing order.
fn fact_of(
    l: &Listed<'_>,
    peeled: &mut impl Iterator<Item = Option<(String, String)>>,
) -> GitResult<Option<TagFact>> {
    let name = l
        .refname
        .strip_prefix("refs/tags/")
        .ok_or_else(|| unreadable(format!("for-each-ref listed {:?}", l.refname)))?;
    let (commit_oid, kind) = match l.object_type {
        "tag" => {
            let commit = match l.peeled_type {
                "commit" => Some(l.peeled_oid.to_owned()),
                "tag" => match peeled.next().flatten() {
                    Some((oid, kind)) => (kind == "commit").then_some(oid),
                    None => {
                        return Err(unreadable(format!("tag {name} peels to a missing object")))
                    }
                },
                "tree" | "blob" => None,
                other => return Err(unreadable(format!("tag {name} peels to {other:?}"))),
            };
            let (Some(commit), Some(tagger)) = (commit, parse_tagger(l.tagger)) else {
                return Ok(None);
            };
            (
                commit,
                TagKind::Annotated {
                    tagger_email: tagger.email,
                    tagged_at: tagger.at,
                    tz_offset_min: tagger.tz_offset_min,
                },
            )
        }
        "commit" => {
            let Ok(email) = std::str::from_utf8(l.committer_email) else {
                return Ok(None);
            };
            let (at, tz_offset_min) = commit_date(l.author_date)
                .ok_or_else(|| unreadable(format!("git did not date {}", l.oid)))?;
            (
                l.oid.to_owned(),
                TagKind::Lightweight {
                    committer_email: email.to_owned(),
                    authored_at: at,
                    tz_offset_min,
                },
            )
        }
        "tree" | "blob" => return Ok(None),
        other => return Err(unreadable(format!("tag {name} names a {other:?}"))),
    };
    Ok(Some(TagFact {
        name: name.to_owned(),
        object_oid: l.oid.to_owned(),
        commit_oid,
        kind,
    }))
}

/// The release read's own `for-each-ref`, as git printed it — for a test to weigh.
///
/// # Errors
/// The spawn's failure or a non-zero exit.
#[cfg(feature = "testkit")]
pub fn release_listing(exec: &GitExec, repo: &RepoHandle) -> GitResult<GitOutput> {
    listing(exec, repo, RunLimits::none(), &CancelToken::new())
}

/// One `for-each-ref` over `refs/tags`, one line per ref, each field closed by a NUL — the one
/// byte no header field can hold, so a TAB in an email stays inside its field. `%(tagger)` is the
/// raw tagger line, read as [`parse_tagger`] reads it; `%(authordate:raw)` is the author time and
/// zone the history job dates a commit-day by.
fn listing(
    exec: &GitExec,
    repo: &RepoHandle,
    limits: RunLimits,
    cancel: &CancelToken,
) -> GitResult<GitOutput> {
    exec.run(
        repo,
        &[
            OsStr::new("for-each-ref"),
            OsStr::new("--format=%(refname)%00%(objectname)%00%(objecttype)%00%(*objectname)%00%(*objecttype)%00%(tagger)%00%(committeremail:trim)%00%(authordate:raw)%00"),
            OsStr::new(REFS_TAGS),
        ],
        limits,
        cancel,
    )
}

/// The listing, line by line, in git's ref-name order.
fn parse_listing(stdout: &[u8]) -> GitResult<Vec<Listed<'_>>> {
    fn text(field: &[u8]) -> GitResult<&str> {
        std::str::from_utf8(field).map_err(|_| unreadable("for-each-ref printed non-UTF-8"))
    }
    let mut listed = Vec::new();
    for line in stdout.split(|&b| b == b'\n').filter(|l| !l.is_empty()) {
        let fields: Vec<&[u8]> = line.split(|&b| b == 0).collect();
        let [refname, oid, object_type, peeled_oid, peeled_type, tagger, committer_email, author_date, []] =
            fields.as_slice()
        else {
            return Err(unreadable(format!(
                "for-each-ref printed {:?}",
                String::from_utf8_lossy(line)
            )));
        };
        let oid = text(oid)?;
        if !is_object_id(oid) {
            return Err(unreadable(format!(
                "for-each-ref printed the object {oid:?}"
            )));
        }
        listed.push(Listed {
            refname: text(refname)?,
            oid,
            object_type: text(object_type)?,
            peeled_oid: text(peeled_oid)?,
            peeled_type: text(peeled_type)?,
            tagger,
            committer_email,
            author_date: text(author_date)?,
        });
    }
    Ok(listed)
}

/// `<epoch> <+HHMM>`, the author date as `%(authordate:raw)` prints it.
fn commit_date(raw: &str) -> Option<(i64, i32)> {
    let (at, _) = raw.trim().split_once(' ')?;
    Some((at.parse().ok()?, tz_offset_from_iso_date(raw)?))
}

/// `Name <email> <epoch> <+HHMM>`. The name is never read, so it need not be UTF-8. A line that
/// does not parse names nobody, as a missing one does: one malformed historic tag must not make
/// every other tag of the repository unreadable.
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
