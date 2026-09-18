//! Typed provider seam over the raw HTTP transport.

pub mod admit;
mod github;
pub mod listing;
pub mod scopes;

use crate::accounts::keychain::SecretToken;
use crate::http::TransportError;
use crate::protocol::Ecosystem;
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
///
/// [p3] §32.4 appends a **seventh**, `advisories` — the first entry whose method takes no token.
/// The census is about *which methods issue a request*, and an unauthenticated one issues exactly
/// as many as an authenticated one.
pub const PROVIDER_REQUEST_METHODS: &[&str] = &[
    "viewer",
    "list_orgs",
    "list_repos",
    "lookup_repo",
    "repo_facts",
    "ci_runs",
    "advisories",
];

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
    /// §25.1's repository facts, as **one conditional read**.
    ///
    /// It takes the caller's validator and returns the response's, per §21.7's
    /// one-pair-per-resource rule, and there is **no `etag_observed_at`** (A15): an `ETag` is never
    /// rendered as a time, so §6 grants it no column of its own.
    ///
    /// A `403` and a `404` are **answers, not errors**: they observe the *access* state and
    /// nothing else, and the writer turns them into `permitted = 0` while dating nothing. Only a
    /// transport failure, an unexpected status or an unparseable body is an `Err`.
    fn repo_facts(
        &self,
        t: &SecretToken,
        owner: &str,
        name: &str,
        etag: Option<&str>,
    ) -> ProviderResult<Observed<RepoFactsRead>>;

    /// §25.1's `LATEST CI` — the Actions runs, as one conditional read with **its own**
    /// validator and therefore its own clock.
    ///
    /// **A8: this requests no scope this project does not hold.** There is no `workflow:read`;
    /// the real scope is `workflow`, a *write* scope granting the addition and update of workflow
    /// files, and this project must never request it. Runs on a public repository need no scope
    /// at all, and on a private one they ride `repo`, which §20 already requests.
    fn ci_runs(
        &self,
        t: &SecretToken,
        owner: &str,
        name: &str,
        etag: Option<&str>,
    ) -> ProviderResult<Observed<CiRunsRead>>;

    /// [p3] §32.1's advisory read — **the first unauthenticated forge read in the product.**
    ///
    /// **It takes no token parameter at all**, and not `Option<&SecretToken>`: the refusal is
    /// structural rather than conditional, so a later author has nowhere to put one. The endpoint
    /// is a global, public advisory database; sending a credential would attribute a
    /// library-wide sweep to whichever account happened to be connected and would spend that
    /// account's allowance on work that is nobody's in particular.
    ///
    /// It returns `ProviderResult<Observed<_>>` like the rest, and that is part of the seam's
    /// contract (R76): `AC-P2-25-5`'s census classifies a request method by exactly that return
    /// shape, so a request method returning anything else would pass the census unseen.
    /// [`Observed::granted_scopes`] is `None` here for ever, which already means *unknown* and
    /// not *an empty grant*.
    ///
    /// **The request key is exactly the `(ecosystem, name, version)` triple and version matching
    /// is server-side.** This product implements no per-ecosystem semver comparison.
    ///
    /// **A package name may appear at most once in `affects`.** The endpoint answers with the
    /// advisories matching *any* of the asked pairs and names the affected **package** rather than
    /// the pair that matched, so two versions of one package in one request would be
    /// indistinguishable in the answer. The batcher is what keeps that true; this method does not
    /// re-check it, because a caller that got it wrong would get a wrong answer and not an error.
    fn advisories(
        &self,
        ecosystem: Ecosystem,
        affects: &[PackageVersion],
        cur: Option<&str>,
    ) -> ProviderResult<Observed<Page<AdvisoryPayload>>>;

    fn canonical_host(&self) -> &str;
    fn host_aliases(&self) -> &[&str];
}

/// One half of the advisory request key: a package and the version this library resolves it at.
///
/// The **ecosystem is the call's own parameter** rather than a third field, because the endpoint
/// keys on it: one request asks about one ecosystem's packages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageVersion {
    pub name: String,
    pub version: String,
}

/// One advisory, as parsed.
///
/// `severity` is a nullable `String` and **not** an enum: the vocabulary belongs to the forge, and
/// a closed mirror of a third party's vocabulary is R26 by construction — the same ruling
/// [`CiRunPayload::conclusion`] already carries. `cve_ids` is a **list** because one advisory
/// carries several CVE ids or **none**; unreviewed and malware advisories have none, and the
/// notifiable unit is the advisory rather than the CVE.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdvisoryPayload {
    pub advisory_id: String,
    pub severity: Option<String>,
    pub cve_ids: Vec<String>,
    pub withdrawn_at: Option<i64>,
    pub summary: String,
    pub url: String,
    pub affects: Vec<AffectedPackage>,
}

/// One package an advisory affects, **with that package's own fix**.
///
/// **Fix availability varies per package within one advisory** — the source reports a first
/// patched version per affected package — and a debt item's `scoring` follows it per
/// `(ecosystem, package_name, advisory_id)`. Carried on the advisory instead, an advisory fixed in
/// one package and not another would flip both items together and one of them would be wrong.
///
/// There is **no version here**: the response names the affected package and the vulnerable
/// *range*, and the version that matched is the one the request asked about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AffectedPackage {
    pub name: String,
    pub fix_available: bool,
    pub fixed_version: Option<String>,
}

/// One conditional repo-facts read: what the server said, and what it said it with.
///
/// `facts` is `None` for a `304` (the caller's copy is current), and for a `403`/`404` (the
/// token may not see it). `status` is what lets the caller tell those apart without a second
/// vocabulary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoFactsRead {
    pub status: u16,
    pub etag: Option<String>,
    pub facts: Option<RepoFactsPayload>,
}

/// One conditional Actions read. Same shape, its own validator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CiRunsRead {
    pub status: u16,
    pub etag: Option<String>,
    pub runs: Option<Vec<CiRunPayload>>,
}

/// The **parse** shape of §25.7's `remote_repo` row: column-shaped, rendered nowhere.
///
/// It is deliberately not `RemoteFacts`, which is the **render** shape keyed to §25.1's blocks.
/// Collapsing them would make one struct answer two questions and would put `permitted` — which
/// has no wire field, and can only be written by the store — onto a wire type.
///
/// **Four counts this read cannot separate are `None`, and that is the invariant rather than a
/// shortfall.** `GET /repos/{owner}/{name}` carries `open_issues_count`, which is issues **plus**
/// pull requests, so rendering it under a block labelled `OPEN ISSUES` would be a wrong number.
/// Unobserved renders `—`; a wrong number renders as a fact.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RepoFactsPayload {
    pub visibility: Option<String>,
    pub description: Option<String>,
    pub fork_parent_remote_key: Option<String>,
    pub stars: Option<u32>,
    pub open_issues: Option<u32>,
    pub good_first_issues: Option<u32>,
    pub open_prs: Option<u32>,
    pub open_prs_from_user: Option<u32>,
    pub topics: Vec<String>,
}

/// One `remote_ci_run` row, as parsed. `conclusion` is a nullable `String` and **not** an enum:
/// the vocabulary belongs to the forge, and a closed mirror of a third party's vocabulary is R26
/// by construction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CiRunPayload {
    pub run_id: i64,
    pub workflow_name: String,
    pub conclusion: Option<String>,
    pub branch: String,
    pub run_number: u32,
    pub started_at: Option<i64>,
}
