//! Admission predicate for remote provider listings.

use std::collections::{BTreeMap, BTreeSet};

use crate::protocol::{Affiliation, ScopeTier};
use crate::provider::listing::RepoListing;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Admission {
    /// Keyed by `(provider, provider_repo_id)`, so two accounts' listings of ONE repository
    /// collapse to one key BEFORE anything reaches a row - which is where the duplicate would
    /// otherwise be born.
    pub admitted: BTreeMap<(String, String), AdmittedRepo>,
    pub skipped_unknown_permission: usize,
    pub skipped_no_push: usize,
    pub skipped_private_under_public_tier: usize,
    pub skipped_org_not_enabled: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmittedRepo {
    pub listing: RepoListing,
    pub affiliation: Affiliation,
}

/// Admits provider listings whose own permission object says the connected account can push.
///
/// `viewer` is part of this predicate because the listing endpoint does not tag entries with an
/// affiliation. Without the viewer login, an owned repository and a collaborator repository are
/// indistinguishable. Viewer and owner logins are compared case-insensitively because forge
/// logins are case-insensitive.
///
/// Evaluation is ordered so every non-duplicate listing has one visible outcome: disabled orgs
/// are counted before any other gate, private repositories are counted under a public tier before
/// the permission object is examined, and only then is `can_push` treated as the admission
/// predicate. A missing permission object is unknown, not a false push permission.
#[must_use]
pub fn admit(
    listings: &[RepoListing],
    viewer: &str,
    tier: ScopeTier,
    enabled_orgs: &BTreeSet<String>,
) -> Admission {
    let mut admission = Admission::default();

    for listing in listings {
        // Case-insensitively, for the same reason the viewer comparison below is: forge logins
        // are case-insensitive, and a case difference here would silently exclude every
        // repository in an org the user had actually enabled.
        if listing
            .in_org
            .as_ref()
            .is_some_and(|org| !enabled_orgs.iter().any(|e| e.eq_ignore_ascii_case(org)))
        {
            admission.skipped_org_not_enabled += 1;
            continue;
        }

        if tier == ScopeTier::Public && listing.is_private {
            admission.skipped_private_under_public_tier += 1;
            continue;
        }

        match listing.can_push {
            Some(true) => {
                let key = (
                    listing.provider.to_owned(),
                    listing.provider_repo_id.clone(),
                );
                let admitted = AdmittedRepo {
                    listing: listing.clone(),
                    affiliation: affiliation_for(listing, viewer),
                };
                admission.admitted.insert(key, admitted);
            }
            Some(false) => {
                admission.skipped_no_push += 1;
            }
            None => {
                admission.skipped_unknown_permission += 1;
            }
        }
    }

    admission
}

#[must_use]
fn affiliation_for(listing: &RepoListing, viewer: &str) -> Affiliation {
    if listing.in_org.is_some() {
        Affiliation::OrganizationMember
    } else if listing.owner.eq_ignore_ascii_case(viewer) {
        Affiliation::Owner
    } else {
        Affiliation::Collaborator
    }
}
