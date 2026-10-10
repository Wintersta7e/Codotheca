//! The archive reachability observation: is every local commit on some remote?
//!
//! One `rev-list` from the local tips with every remote-tracking tip and the caller's extra tips
//! excluded. Nothing printed is `true`; any commit printed is `false`; a walk that fails — a
//! missing object included — is `Err`, never `true`.

use std::ffi::OsStr;

use crate::cancel::CancelToken;

use super::analyse::{self, RefListing};
use super::error::GitResult;
use super::exec::{GitExec, RunLimits};
use super::repo::RepoHandle;

/// The local tips: every branch and tag, by the object its ref names, plus `detached_head`.
///
/// A tag is passed as the tag object: the walk peels it, and passes over one that peels to a
/// tree or a blob. A stash is not a local tip.
fn local_tips(listing: &RefListing, detached_head: Option<&str>) -> Vec<String> {
    let mut tips: Vec<String> = listing
        .refs
        .iter()
        .filter(|r| r.name.starts_with("refs/heads/") || r.name.starts_with("refs/tags/"))
        // §45.2a's product-tag exclusion is this function's one filter point; it is not applied
        // yet, so the product's own tag still counts as a local tip.
        .map(|r| r.oid.clone())
        .collect();
    tips.extend(detached_head.map(str::to_owned));
    tips.sort_unstable();
    tips.dedup();
    tips
}

/// Whether every commit reachable from a local tip is reachable from a `refs/remotes/*` tip or
/// from `extra_excluded`. No local tip is `true` without a walk.
///
/// # Errors
/// The enumeration's failure, an id that is not an object id, or a failed walk — a missing
/// object included.
pub(crate) fn archive_reach(
    exec: &GitExec,
    repo: &RepoHandle,
    detached_head: Option<&str>,
    extra_excluded: &[String],
    limits: RunLimits,
    cancel: &CancelToken,
) -> GitResult<bool> {
    let listing = analyse::enumerate_refs(exec, repo, limits, cancel)?;
    let tips = local_tips(&listing, detached_head);
    let uncovered = analyse::any_uncovered_also(
        exec,
        repo,
        &tips,
        extra_excluded,
        &[OsStr::new("--not"), OsStr::new("--remotes")],
        limits,
        cancel,
    )?;
    Ok(!uncovered)
}
