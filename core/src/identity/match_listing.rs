//! §22.3 — the listing↔project matcher. A **second pure function beside** [`decide`], in the
//! same shape, with four outcomes.
//!
//! [`decide`](super::decide::decide) is not edited and every test in it stays true. This one runs
//! in **both** directions off one implementation: `ingest` calls it when a sync persists a
//! listing page, and `resolve_identity` reaches it through `hydrate` when a scan is about to
//! create a row.
//!
//! **Purity, asserted by construction.** The signature takes slices and returns a value — no
//! `&Transaction`, no `Clock`, no `GitBackend`, no `Provider` — so *"no IO, no network call, no
//! clock, and no git invocation"* is a property of the type rather than a promise.

use super::alias::{fold_key, HostAliases};
use super::candidates::{LinkCandidate, Suppressor};
use super::remote::canonical_remote_key;
use crate::protocol::RemoteLinkBasis;
use crate::provider::listing::RepoListing;

/// Everything the matcher may see. Four fields, both bases, nothing else.
///
/// **It carries no name, no fork parent, no description, no default branch, no language, no
/// visibility, no timestamp and no size.** §22.1 lists the repository name among the things that
/// are *not evidence* and §22.8 says the forge-declared parent is one *the matcher never reads*,
/// so a type that admitted them would turn a structural guarantee back into a review finding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListingEvidence {
    /// The forge the listing came from — half of the `provider_id` basis.
    pub provider: String,
    /// The forge's own id for the repository — the other half; meaningless without `provider`.
    pub provider_repo_id: String,
    /// `canonical_remote_key(<clone url>)` — the spelling git will actually contact, and what a
    /// created row stores.
    pub remote_key: String,
    /// §22.2's comparison form of the same key.
    pub folded_key: String,
}

/// Everything ingest writes and the matcher may not see.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListingFacts {
    /// The **bare** repository name — never `owner/name`. It becomes `seed_basename` on a row
    /// created from a listing (§7.4, §22.4).
    pub name: String,
    /// Whether the forge says the repository is a fork; written, never matched on (§22.8).
    pub is_fork: bool,
    /// §22.8's rendered forge fact, canonicalised by the one canonicaliser. Written to §25.7's
    /// facts row by the plan that owns it; the matcher never reads it.
    pub fork_parent_remote_key: Option<String>,
}

/// What a listing is to the library (§22.3): one of four outcomes, decided on evidence alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListingMatch {
    /// Exactly one candidate equal on the winning basis.
    Attach {
        /// The project the listing binds to.
        project_id: i64,
        /// Which basis decided it: `provider_id` first, `remote_key` only when that found none.
        basis: RemoteLinkBasis,
    },
    /// Two or more equal on the winning basis. Attaches nothing, creates nothing (§22.5).
    Ambiguous {
        /// Every candidate equal on that basis, ordered `(created_at, id)`.
        candidates: Vec<i64>,
    },
    /// No equal candidate, and a same-path-component project on another host (§22.6).
    Suppress {
        /// The first suppressor, ordered `(created_at, id)`.
        blocked_by: i64,
    },
    /// No equal candidate and no suppressor: a not-cloned project (§23).
    Create,
}

/// Take a listing apart, once, here.
///
/// The **one** place a [`RepoListing`] is split, so the evidence/facts boundary cannot be routed
/// around by a caller that builds a `ListingEvidence` of its own from softer fields. `None` when
/// the clone URL does not canonicalise — a URL that names no `<host>/<owner>/<name>` is not
/// evidence of anything, and an empty string would compare equal to every other one.
#[must_use]
pub fn listing_parts_from(
    listing: &RepoListing,
    aliases: &HostAliases,
) -> Option<(ListingEvidence, ListingFacts)> {
    let remote_key = canonical_remote_key(&listing.clone_url)?.key;
    let folded_key = fold_key(&remote_key, aliases)?;
    Some((
        ListingEvidence {
            provider: listing.provider.to_owned(),
            provider_repo_id: listing.provider_repo_id.clone(),
            remote_key,
            folded_key,
        },
        ListingFacts {
            name: listing.name.clone(),
            is_fork: listing.is_fork,
            fork_parent_remote_key: listing
                .fork_parent_clone_url
                .as_deref()
                .and_then(canonical_remote_key)
                .map(|parent| parent.key),
        },
    ))
}

/// §22.3, as one function. `provider_id` first, then `remote_key`, then suppression.
///
/// **The asymmetry is the part worth reading.** On the second comparison a candidate whose
/// `(provider, provider_repo_id)` is unknown is **not** excluded, while one that is known and
/// different **is**. Unknown is not different — the same distinction §1.1 draws for lineage, and
/// the one that keeps the ordinary first sync from minting a second tile for every project the
/// scan indexed before any id existed.
#[must_use]
pub fn match_listing(
    listing: &ListingEvidence,
    candidates: &[LinkCandidate],
    suppressors: &[Suppressor],
) -> ListingMatch {
    let by_id: Vec<i64> = candidates
        .iter()
        .filter(|c| binds_to(c, listing) == Some(true))
        .map(|c| c.project_id)
        .collect();
    if let Some(outcome) = decide_on(&by_id, RemoteLinkBasis::ProviderId) {
        return outcome;
    }

    let by_key: Vec<i64> = candidates
        .iter()
        .filter(|c| c.folded_key.as_deref() == Some(listing.folded_key.as_str()))
        .filter(|c| binds_to(c, listing) != Some(false))
        .map(|c| c.project_id)
        .collect();
    if let Some(outcome) = decide_on(&by_key, RemoteLinkBasis::RemoteKey) {
        return outcome;
    }

    suppressors
        .first()
        .map_or(ListingMatch::Create, |blocker| ListingMatch::Suppress {
            blocked_by: blocker.project_id,
        })
}

/// `Some(true)` this candidate is bound to this forge repository, `Some(false)` it is bound to a
/// different one, `None` it is not bound yet.
///
/// The basis is the **pair**, never the id alone: two forges number their repositories
/// independently, so `("other-forge", "42")` and `("github", "42")` are different repositories
/// that happen to share a number.
fn binds_to(candidate: &LinkCandidate, listing: &ListingEvidence) -> Option<bool> {
    let provider = candidate.provider.as_deref()?;
    let repo_id = candidate.provider_repo_id.as_deref()?;
    Some(provider == listing.provider && repo_id == listing.provider_repo_id)
}

/// One candidate attaches, two or more are ambiguous, none falls through to the next basis.
fn decide_on(matched: &[i64], basis: RemoteLinkBasis) -> Option<ListingMatch> {
    match matched {
        [] => None,
        [project_id] => Some(ListingMatch::Attach {
            project_id: *project_id,
            basis,
        }),
        many => Some(ListingMatch::Ambiguous {
            candidates: many.to_vec(),
        }),
    }
}
