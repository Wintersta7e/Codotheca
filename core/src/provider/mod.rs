//! Typed provider seam over the raw HTTP transport.

pub mod admit;
mod github;
pub mod listing;
pub mod scopes;

use crate::accounts::keychain::SecretToken;
use crate::http::TransportError;
use crate::provider::listing::{OrgListing, Page, RepoListing, Verified, Viewer};

pub use github::GitHubProvider;

/// The request-issuing provider methods.
///
/// This is deliberately a module-level constant, not an associated const on [`Provider`]:
/// a trait with an associated const is not dyn-compatible (`E0038`), and real consumers store
/// `Arc<dyn Provider>`. The tripwire still enumerates request methods rather than grepping `fn`
/// declarations, so helpers such as [`Provider::canonical_host`] cannot be miscounted.
pub const PROVIDER_REQUEST_METHODS: [&str; 4] =
    ["verify_token", "viewer", "list_orgs", "list_repos"];

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
    /// Verifies the token by reading the current user.
    ///
    /// The token response has no body field for grants, so `Verified.granted_scopes` is copied
    /// from the response's `X-OAuth-Scopes` header. If the header is absent,
    /// `Verified.granted_scopes` is empty but [`Observed::granted_scopes`] is `None`; callers act
    /// on the `None`, because an absent header is unknown and writing an empty grant would render
    /// the account as having no scopes at all.
    fn verify_token(&self, t: &SecretToken) -> ProviderResult<Observed<Verified>>;
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
    fn canonical_host(&self) -> &str;
    fn host_aliases(&self) -> &[&str];
}
