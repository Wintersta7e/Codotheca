//! §22.10 — applying one listing entry, inside the transaction that persists its page.
//!
//! One function the sync calls once per listing. It opens no transaction of its own and reaches
//! no network: everything it needs was read by [`super::candidates`] and decided by
//! [`super::match_listing`], both of which are given the same `&Transaction`.

use rusqlite::{params, Transaction};

use super::alias::HostAliases;
use super::binding::{write_binding, RemoteBinding};
use super::candidates::{load_link_candidates, load_suppressors};
use super::match_listing::{listing_parts_from, match_listing, ListingMatch};
use super::IdentityError;
use crate::protocol::RemoteLinkBasis;
use crate::provider::listing::RepoListing;

/// What one listing entry did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListingIngest {
    /// What the matcher decided for the entry.
    pub outcome: ListingMatch,
    /// The row this entry reached, `None` for `Ambiguous` and `Suppress`, which reach none.
    pub project_id: Option<i64>,
    /// True only for `Create` — a new not-cloned project row was inserted.
    pub created: bool,
    /// The listing's own canonical key.
    ///
    /// **Not in the plan's declared shape, and it has to be here**: R62 requires a suppression to
    /// name the listing as well as the project that blocked it, and
    /// [`ListingIngestReport::push`] takes a `ListingIngest` and nothing else, so the key has
    /// nowhere else to travel.
    pub listing_key: String,
}

/// One entry that was withheld, and the project that withheld it (R62).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suppression {
    /// The withheld listing's canonical key.
    pub listing_key: String,
    /// The project whose copy on another host withheld it.
    pub blocked_by: i64,
}

/// The per-page tally §21.10's summary is copied from.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ListingIngestReport {
    /// Every entry pushed, whatever its outcome.
    pub listed: usize,
    /// Entries that reached a row — attached to one or created one.
    pub admitted: usize,
    /// The **projects** involved in an ambiguity, deduplicated and in first-seen order: this is
    /// the list §11.1's eighth group needs. §21.10's summary carries a count and p2-21 renders
    /// `ambiguous.len()`; a count and a list are different values and neither side re-derives
    /// the other.
    pub ambiguous: Vec<i64>,
    /// **The source `SyncListingSummary.suppressedBy` is copied from, never re-derived** (R62).
    /// Positionally aligned with the suppressions and **not deduplicated**: one project blocking
    /// two entries reads as two, and an empty list means *nothing was suppressed* — never *not
    /// computed*.
    pub suppressed: Vec<Suppression>,
}

impl ListingIngestReport {
    /// Tally one entry's outcome into the page's counts and lists.
    pub fn push(&mut self, ingest: ListingIngest) {
        let ListingIngest {
            outcome,
            listing_key,
            ..
        } = ingest;
        self.listed += 1;
        match outcome {
            ListingMatch::Attach { .. } | ListingMatch::Create => self.admitted += 1,
            ListingMatch::Ambiguous { candidates } => {
                for id in candidates {
                    if !self.ambiguous.contains(&id) {
                        self.ambiguous.push(id);
                    }
                }
            }
            ListingMatch::Suppress { blocked_by } => self.suppressed.push(Suppression {
                listing_key,
                blocked_by,
            }),
        }
    }
}

/// Apply one listing entry (§22.10).
///
/// | Outcome | Writes |
/// |---|---|
/// | `Attach` | the three binding columns and `updated_at`; `is_fork = 1` when the listing says so and **never cleared** (§22.8). `remote_key` is never rewritten by a listing (§22.7) and `association_kind` is not touched (§22.11) |
/// | `Ambiguous` | `ambiguous_lineage = 1` and `updated_at` on each candidate, **and nothing else**. §22.3's *"attaches nothing, creates nothing"* is kept literally: a flag is neither, and without this write §22.5's state has no producer at all |
/// | `Suppress` | **nothing.** The blocking project and the listing's key go into the report |
/// | `Create` | one `project` row, seeded on the listing's **bare** name |
///
/// # Errors
/// Fails with [`IdentityError::ListingNotCanonical`] when the clone URL names no
/// `<host>/<owner>/<name>`, and [`IdentityError::Sqlite`] when a read or write is refused.
pub fn ingest_listing(
    tx: &Transaction<'_>,
    listing: &RepoListing,
    aliases: &HostAliases,
    now: i64,
) -> Result<ListingIngest, IdentityError> {
    let Some((evidence, facts)) = listing_parts_from(listing, aliases) else {
        // Not silently dropped: §21.10's summary must account for every entry, and an entry that
        // vanished between the page and the tally is the silent suppression §11.1 forbids.
        return Err(IdentityError::ListingNotCanonical {
            provider: listing.provider.to_owned(),
            provider_repo_id: listing.provider_repo_id.clone(),
        });
    };

    let candidates = load_link_candidates(tx, &evidence, aliases)?;
    let suppressors = load_suppressors(tx, &evidence, aliases)?;
    let outcome = match_listing(&evidence, &candidates, &suppressors);

    let (project_id, created) = match &outcome {
        ListingMatch::Attach { project_id, basis } => {
            write_binding(
                tx,
                *project_id,
                &RemoteBinding {
                    provider: evidence.provider.clone(),
                    provider_repo_id: evidence.provider_repo_id.clone(),
                    remote_link_basis: Some(*basis),
                },
                now,
            )?;
            if facts.is_fork {
                tx.execute(
                    "UPDATE project SET is_fork = 1, updated_at = ?2 WHERE id = ?1",
                    params![project_id, now],
                )?;
            }
            (Some(*project_id), false)
        }
        ListingMatch::Ambiguous { candidates: tied } => {
            for id in tied {
                tx.execute(
                    "UPDATE project SET ambiguous_lineage = 1, updated_at = ?2 WHERE id = ?1",
                    params![id, now],
                )?;
            }
            (None, false)
        }
        ListingMatch::Suppress { .. } => (None, false),
        ListingMatch::Create => {
            let id = create_from_listing(tx, &evidence, &facts.name, facts.is_fork, now)?;
            (Some(id), true)
        }
    };

    Ok(ListingIngest {
        outcome,
        project_id,
        created,
        listing_key: evidence.remote_key,
    })
}

/// A not-cloned project (§23). It writes **no `location` row and no facts row**: §23 owns what a
/// zero-location project renders, and §21 and §25 own the facts.
///
/// **`remote_link_basis` is `remote_key`, and §22 does not say so.** AC-P2-22-1 requires
/// *identical* basis values in both ingest orders. In scan-then-sync the local row carries no id,
/// so the listing matches on `remote_key` and `Attach` writes `remote_key`; for sync-then-scan to
/// agree, `Create` must write the same value. The binding was established by the clone URL's
/// canonical key, and only a later id-equal comparison promotes it. Writing NULL, or writing
/// `provider_id`, makes the two orders disagree and the criterion unpassable. **Recorded as a
/// ruling of this plan, not as an inference from §22.**
fn create_from_listing(
    tx: &Transaction<'_>,
    evidence: &super::match_listing::ListingEvidence,
    bare_name: &str,
    is_fork: bool,
    now: i64,
) -> Result<i64, IdentityError> {
    tx.execute(
        "INSERT INTO project
            (name, seed_basename, remote_key, provider, provider_repo_id, remote_link_basis,
             is_fork, created_at, updated_at)
         VALUES (?1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
        params![
            bare_name,
            // The UNFOLDED key: what git will contact, and what a later scan of a clone of this
            // repository will recompute. The fold is a comparison form and is never stored.
            evidence.remote_key,
            evidence.provider,
            evidence.provider_repo_id,
            RemoteLinkBasis::RemoteKey.slug(),
            i64::from(is_fork),
            now,
        ],
    )?;
    Ok(tx.last_insert_rowid())
}
