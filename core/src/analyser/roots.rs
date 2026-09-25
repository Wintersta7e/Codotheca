//! §45.6 steps 3 and 7: the roots `P(L)` holds, and the one walk that asks whether any of them
//! is uncovered.
//!
//! **The spawn count is independent of the ref count.** Every root goes into one `rev-list
//! --stdin` with `^t` for each covered tip; measured on 2,001 refs, a per-ref loop took 1,769 ms
//! and one walk 8 ms (D8 §3).

use crate::git::{GitBackend, HeadState, JobContext, RepoHandle, StashEntries};
use crate::protocol::UninstallBlocker;

/// The stash ref: its entries are read by row 3, never walked as a root of row 1.
const REFS_STASH: &str = "refs/stash";

/// §45.2 rows 1–4, as step 3 snapshots them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Roots {
    /// Every non-stash ref's object, and a detached `HEAD`'s commit: what step 7 walks.
    pub walked: Vec<String>,
    /// Annotated tag objects (row 4): each is covered only by its own advertised id.
    pub tags: Vec<String>,
    /// Every stash entry's commit, newest first (row 3).
    pub stash: Vec<String>,
}

impl Roots {
    /// Does anything here need an elsewhere? Stash entries never do: they are `stash_present`
    /// before the network step, which is what keeps step 6 from running for them (AC-P4-45-16).
    #[must_use]
    pub fn need_elsewhere(&self) -> bool {
        !self.walked.is_empty() || !self.tags.is_empty()
    }
}

/// Step 3: rows 1–4 through git. A listing that cannot be read is `refs_unreadable`; a stash
/// reflog that cannot be read is `stash_unreadable`. **Never a shorter list.**
///
/// # Errors
/// The unknown-class blocker of the input that could not be read.
pub fn read_roots(
    git: &dyn GitBackend,
    repo: &RepoHandle,
    ctx: &JobContext<'_>,
) -> Result<Roots, UninstallBlocker> {
    let listing = git
        .enumerate_refs(repo, ctx)
        .map_err(|_| UninstallBlocker::RefsUnreadable)?;
    let mut roots = Roots::default();
    for entry in listing.refs {
        if entry.name == REFS_STASH {
            continue;
        }
        if entry.object_type == "tag" {
            roots.tags.push(entry.oid.clone());
        }
        roots.walked.push(entry.oid);
    }
    if let HeadState::Detached(oid) = listing.head {
        roots.walked.push(oid);
    }
    roots.walked.sort_unstable();
    roots.walked.dedup();
    roots.tags.sort_unstable();
    roots.tags.dedup();
    match git.stash_entries(repo, ctx) {
        Ok(StashEntries::Entries(entries)) => roots.stash = entries,
        Ok(StashEntries::Unreadable) | Err(_) => return Err(UninstallBlocker::StashUnreadable),
    }
    Ok(roots)
}

/// What step 7 found uncovered by `covered` (§45.3's `T`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Uncovered {
    /// Some walked root reaches a commit no covered tip reaches.
    pub commit: bool,
    /// Some annotated tag object's own id is not in `covered`.
    pub tag: bool,
}

/// Step 7: **one walk** over the walked roots with `^t` for each `t` in `covered`, replace
/// objects and grafts off; a tag object is covered iff its own id is in `covered`.
///
/// # Errors
/// `refs_unreadable` when the walk fails — a missing object included.
pub fn uncovered(
    git: &dyn GitBackend,
    repo: &RepoHandle,
    roots: &Roots,
    covered: &[String],
    ctx: &JobContext<'_>,
) -> Result<Uncovered, UninstallBlocker> {
    let commit = git
        .any_uncovered(repo, &roots.walked, covered, ctx)
        .map_err(|_| UninstallBlocker::RefsUnreadable)?;
    let tag = roots.tags.iter().any(|tag| !covered.contains(tag));
    Ok(Uncovered { commit, tag })
}
