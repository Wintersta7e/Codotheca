//! `account_repos` — the scheduled task, and the only scheduled one.
//!
//! **The page is the transaction.** For each page, in one transaction: mirror the budget, run
//! p2-20's admission pass, persist through p2-22's ingest, recompute the derived values the write
//! changed, and move the clock. §21.9's rule 2 is what that is for — *a listing whose values
//! landed and whose clock did not is the currency invariant broken*.
//!
//! **R94's second side.** This runs on the sync worker thread, so it takes
//! `&Mutex<Index>` and locks it itself: nothing above the worker holds the guard. The same
//! signature in a route arm would deadlock.

use std::collections::BTreeSet;
use std::sync::Mutex;

use crate::identity::ingest::{ingest_listing, ListingIngestReport};
use crate::index::Index;
use crate::protocol::{AccountId, ProjectId};
use crate::provider::admit::admit;
use crate::provider::listing::{Page, RepoListing};
use crate::provider::{declared_host_aliases, Observed};
use crate::sync::budget::mirror;
use crate::sync::outcome::SyncOutcome;
use crate::sync::{observe_one, token_for, SyncDeps, SyncError};

/// §21.10's settle-time count set, accumulated across a listing's pages.
///
/// **Three sources, one owner.** `listed`, `admitted`, `ambiguous` and `suppressed_by` come from
/// p2-22's `ListingIngestReport`; `skipped_unknown_permission` comes from **p2-20's admission
/// pass**, which runs *before* ingest and is the only place a listing entry with no permission
/// object is seen; the totals accumulate here.
///
/// **`suppressed_by` is positionally aligned with the suppressions and is not deduplicated** —
/// one entry per suppression, so `suppressed_by.len() == suppressed` is an invariant a test can
/// hold, and one project blocking two entries is visible as two. It is copied from p2-22's report
/// and never re-derived: the decision is p2-22's and the transport is this plan's.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ListingSummary {
    /// Entries the admission pass let through to ingest.
    pub listed: i64,
    /// Entries that reached a row — attached to a project or created one.
    pub admitted: i64,
    /// Entries the admission pass skipped because they carried no permission object.
    pub skipped_unknown_permission: i64,
    /// Projects caught in an ambiguous match, deduplicated within each page and summed across
    /// pages.
    pub ambiguous: i64,
    /// Entries withheld because an existing project blocked them.
    pub suppressed: i64,
    /// The blocking project of each suppression, one entry per suppression.
    pub suppressed_by: Vec<ProjectId>,
}

impl ListingSummary {
    /// Fold one page's tally into the listing's.
    pub fn merge(&mut self, page: &Self) {
        self.listed = self.listed.saturating_add(page.listed);
        self.admitted = self.admitted.saturating_add(page.admitted);
        self.skipped_unknown_permission = self
            .skipped_unknown_permission
            .saturating_add(page.skipped_unknown_permission);
        self.ambiguous = self.ambiguous.saturating_add(page.ambiguous);
        self.suppressed = self.suppressed.saturating_add(page.suppressed);
        self.suppressed_by.extend_from_slice(&page.suppressed_by);
    }

    /// The wire shape §21.13 declares.
    #[must_use]
    pub fn payload(&self) -> crate::protocol::SyncListingSummary {
        crate::protocol::SyncListingSummary {
            listed: self.listed,
            admitted: self.admitted,
            skipped_unknown_permission: self.skipped_unknown_permission,
            ambiguous: self.ambiguous,
            suppressed: self.suppressed,
            suppressed_by: self.suppressed_by.clone(),
        }
    }
}

/// One page of one account's admissible repository listing.
///
/// Returns the outcome **this page** produced and the tally it contributed. A `NextPage` carries
/// the cursor the runner hands back on the next call; every other outcome settles the task.
///
/// **No `If-None-Match` is sent and `account.listing_etag` is neither read nor written here.**
/// `Provider::list_repos` (`core/src/provider/mod.rs:93-97`) takes no validator and returns none,
/// unlike `repo_facts` (`:122-128`) and `ci_runs` (`:137-143`), which both do. §21.7 and R67 put
/// the column on `0008` for this read; widening the method is p2-20's seam under R49 and R76, not
/// this plan's to hand-write. Recorded in `.dev/reports/p2-21.md` rather than filled silently.
///
/// # Errors
/// Fails when the index or the keychain refuses. A forge **refusal** is not an error: it is an
/// outcome, classified from the headers by the observing transport.
pub fn run_account_repos(
    deps: &SyncDeps,
    index: &Mutex<Index>,
    account: AccountId,
    cursor: Option<&str>,
) -> Result<(SyncOutcome, ListingSummary), SyncError> {
    let now = deps.clock.now_unix();
    let (token_ref, viewer, tier, enabled_orgs) = {
        let guard = index
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let identity = crate::accounts::store::account_identity(guard.conn(), account)
            .map_err(|e| SyncError::Account(e.to_string()))?;
        let orgs = crate::accounts::store::list_orgs(guard.conn(), account)
            .map_err(|e| SyncError::Account(e.to_string()))?
            .unwrap_or_default();
        drop(guard);
        let enabled: BTreeSet<String> = orgs
            .into_iter()
            .filter(|o| o.enabled)
            .map(|o| o.login)
            .collect();
        (
            identity.token_ref,
            identity.login,
            identity.scope_tier,
            enabled,
        )
    };
    // **The keychain read happens outside the guard** (R75), which is why the block above yields
    // a `token_ref` rather than a token: reading a secret is a call into the OS credential store,
    // and the process's one SQLite mutex may not be held across it. `rename.rs` and `remote.rs`
    // were already this shape. A p2-20 test caught this one the first time a listing actually
    // ran — `disconnect_holds_no_index_lock_while_it_deletes_the_keychain_entry` probes whether
    // the mutex is free at the moment the keychain is reached, and nothing had ever reached it
    // from this path before.
    let token = token_for(deps, &token_ref)?;

    let answer = deps.provider.list_repos(&token, cursor);
    let observation = observe_one(deps, &answer);
    let page: Option<Page<RepoListing>> = answer.ok().map(|Observed { value, .. }| value);

    let mut summary = ListingSummary::default();
    let mut outcome = observation.outcome.clone();

    {
        let mut guard = index
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.with_tx(|tx| {
            // §21.6 first, and **whatever the outcome**: an error response's headers are the ones
            // that decide whether the next request is even issued.
            mirror(tx, Some(account), &observation.rate, observation.at)?;

            if let Some(page) = &page {
                // p2-20's predicate. Its `skipped_unknown_permission` is the only place an entry
                // with no permission object is seen, and §11.1 forbids dropping it silently.
                let admission = admit(&page.items, &viewer, tier, &enabled_orgs);
                summary.skipped_unknown_permission =
                    i64::try_from(admission.skipped_unknown_permission).unwrap_or(i64::MAX);

                let aliases = declared_host_aliases();
                let mut report = ListingIngestReport::default();
                let mut touched: BTreeSet<i64> = BTreeSet::new();
                for admitted in admission.admitted.values() {
                    // `IdentityError` carries no `Display` — §2.4 keeps its user-facing prose
                    // in the shell — so it travels as its `Debug` form, which is diagnostic and
                    // is never rendered.
                    let ingest =
                        ingest_listing(tx, &admitted.listing, &aliases, now).map_err(|e| {
                            crate::index::IndexError::Corrupt {
                                detail: format!("{e:?}"),
                            }
                        })?;
                    if let Some(project) = ingest.project_id {
                        touched.insert(project);
                    }
                    report.push(ingest);
                }

                summary.listed = i64::try_from(report.listed).unwrap_or(i64::MAX);
                summary.admitted = i64::try_from(report.admitted).unwrap_or(i64::MAX);
                summary.ambiguous = i64::try_from(report.ambiguous.len()).unwrap_or(i64::MAX);
                summary.suppressed = i64::try_from(report.suppressed.len()).unwrap_or(i64::MAX);
                // Copied, never re-derived: naming the blocker is p2-22's decision (R62).
                summary.suppressed_by = report
                    .suppressed
                    .iter()
                    .map(|s| ProjectId(s.blocked_by))
                    .collect();

                // §21.9: a sync write that changes an input to a derived value recomputes it
                // **before this transaction commits**. Safe for a zero-location project —
                // `all_offline` is guarded by `!locations.is_empty()`
                // (`core/src/derive/persist.rs:126-129`).
                for project in touched {
                    crate::derive::persist::recompute(tx, ProjectId(project), now)?;
                }
            }
            Ok(())
        })?;
    }

    // §21.8 step 9's second half, applied where the cursor exists.
    outcome = outcome.with_next_page(page.and_then(|p| p.next_cursor));
    Ok((outcome, summary))
}
