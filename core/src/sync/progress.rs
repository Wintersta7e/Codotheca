//! §21.11's listing counter, and the denominator that must not be guessed.
//!
//! A scan may show no percentage because its denominator is unknown (§10.2). A listing is not
//! that case and must not inherit the rule by accident — but §10.2's ban has **two** clauses, and
//! the second is about **retreat** (A10).
//!
//! - Where the response supplied a total, progress renders `<n> of <total>`.
//! - Where it did not, progress renders a **bare count**. A denominator inferred from page
//!   numbers is a guess, and a guessed denominator is an unknown rendered as a fact.
//! - **Never a percentage, and never a figure that retreats.**
//!
//! **No forge listing in phase 2 supplies a total**, and that is by design rather than by
//! omission: p2-20's `Page<T>` (`core/src/provider/listing.rs:24-27`) carries `items` and
//! `next_cursor` and nothing else, because the only denominator the endpoint could offer is
//! `pages × per_page` off the `Link` header — which is exactly the guess §21.11 bans. So
//! `total` is `None` on every event this phase emits. It stays on the wire because the type has
//! to be able to say *unknown*, and because a later section that does observe one writes into a
//! field rather than into a migration.

use crate::protocol::{AccountId, SyncListingProgress};

/// A monotone count of entries listed, with an optional denominator.
///
/// **Monotone by construction.** [`ListingProgress::add`] is the only mutator and it saturates
/// upward; there is no setter, so a later page that reports fewer entries cannot lower the
/// figure. That is R100's shape rather than an assertion: a retreat is not a state this type can
/// reach.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ListingProgress {
    account: AccountId,
    listed: i64,
    total: Option<i64>,
}

impl ListingProgress {
    /// Start a listing.
    ///
    /// `total` is what the **response** supplied, and `None` is *not observed* — never a
    /// denominator this process worked out for itself.
    #[must_use]
    pub const fn new(account: AccountId, total: Option<i64>) -> Self {
        Self {
            account,
            listed: 0,
            total,
        }
    }

    /// Add one page's entries. A negative or absent count adds nothing rather than subtracting.
    pub fn add(&mut self, n: i64) {
        self.listed = self.listed.saturating_add(n.max(0));
    }

    /// Entries counted so far across every page of this listing.
    #[must_use]
    pub const fn listed(&self) -> i64 {
        self.listed
    }

    /// The wire shape, for the `listing_progress` event and for `sync.status`.
    #[must_use]
    pub const fn payload(&self) -> SyncListingProgress {
        SyncListingProgress {
            account_id: self.account,
            listed: self.listed,
            total: self.total,
        }
    }
}
