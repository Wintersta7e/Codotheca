//! Typed provider seam over the raw HTTP transport.

pub mod admit;
mod github;
pub mod listing;
pub mod scopes;

use crate::accounts::keychain::SecretToken;
use crate::http::TransportError;
use crate::provider::listing::{OrgListing, Page, RepoListing, Viewer};

pub use github::GitHubProvider;

/// The request-issuing provider methods — R73's census.
///
/// A module-level constant, not an associated const on [`Provider`]: a trait carrying one is not
/// dyn-compatible (`E0038`), and the seam is held as `Arc<dyn Provider>`. R49's substance is
/// unchanged and is the part that matters — the list is **enumerated**, never grepped out of the
/// source, so helpers such as [`Provider::canonical_host`] cannot be miscounted.
///
/// A slice rather than a fixed-size array, per R73: p2-22 appends `lookup_repo` and p2-25 appends
/// `repo_facts` and `ci_runs`, and an array's length would make each of those a second edit to
/// this same line. **Six by the end of phase 2, three of them this plan's** — R79 collapsed
/// `verify_token` into `viewer`, so two entries became one and only the arithmetic changed.
pub const PROVIDER_REQUEST_METHODS: &[&str] = &["viewer", "list_orgs", "list_repos", "lookup_repo"];

/// §22.2's host-alias set for a caller that has **no account and no transport in hand**.
///
/// A scan folds a clone's remote key to compare it against a listing's, and it holds no
/// `Provider`: the alias sets are *declarations*, static per adapter, so the scan reads the
/// declaration rather than constructing an adapter with a transport it does not need.
///
/// **One adapter, so one set.** A second forge makes this a slice and every caller a loop; it is
/// a function rather than a constant so that change is one signature rather than a search.
#[must_use]
pub fn declared_host_aliases() -> crate::identity::alias::HostAliases {
    crate::identity::alias::HostAliases::declared(
        listing::GITHUB_CANONICAL_HOST,
        listing::GITHUB_HOST_ALIASES,
    )
}

/// A typed provider value plus the scopes observed on that response.
///
/// `None` means the response carried no `X-OAuth-Scopes` header, which is unknown. `Some(vec![])`
/// means the header was present and empty, a real observed empty grant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Observed<T> {
    pub value: T,
    pub granted_scopes: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProviderError {
    #[error("provider returned HTTP status {status}")]
    Http {
        status: u16,
        headers: Vec<(String, String)>,
    },
    #[error("provider transport failed: {0}")]
    Transport(TransportError),
    #[error("provider response could not be decoded: {0}")]
    Decode(String),
}

pub type ProviderResult<T> = Result<T, ProviderError>;

pub trait Provider: Send + Sync + std::fmt::Debug {
    /// Who this token is, and what it was granted — **in one request** (R79).
    ///
    /// It was two methods, `verify_token` and `viewer`, which resolved to the same `GET /user`
    /// and decoded the same body: two forge requests to learn one thing, against a budgeted and
    /// rate-limited resource whose exhaustion §21 exists to handle. R48 ruled the same shape one
    /// level up — two names for one shape is R15 inverted.
    ///
    /// **Verification is what a caller does with the failure**, not a second call: a 401 here is
    /// a token that does not authenticate. The grant travels on [`Observed::granted_scopes`],
    /// which is `None` when the response carried no `X-OAuth-Scopes` header — unknown, and not an
    /// empty grant, which would render the account as having no scopes at all.
    fn viewer(&self, t: &SecretToken) -> ProviderResult<Observed<Viewer>>;
    fn list_orgs(
        &self,
        t: &SecretToken,
        cur: Option<&str>,
    ) -> ProviderResult<Observed<Page<OrgListing>>>;
    fn list_repos(
        &self,
        t: &SecretToken,
        cur: Option<&str>,
    ) -> ProviderResult<Observed<Page<RepoListing>>>;
    /// One listing read of a stored `owner`/`name` (§22.7's rename repair).
    ///
    /// **A listing read, not a fetch**: no git object, no history, no working copy. `Ok(None)` is
    /// *no such repository for this token* — a 404 answers "not found" and "not visible to you"
    /// identically, and neither is an error, so the fields stay **unknown** rather than `failed`.
    ///
    /// It returns `ProviderResult<Observed<_>>` like the rest, and that is now part of the seam's
    /// contract (R76): the census tripwire classifies a request method by exactly that return
    /// shape, so a request method returning anything else would pass the census unseen.
    fn lookup_repo(
        &self,
        t: &SecretToken,
        owner: &str,
        name: &str,
    ) -> ProviderResult<Observed<Option<RepoListing>>>;
    fn canonical_host(&self) -> &str;
    fn host_aliases(&self) -> &[&str];
}
