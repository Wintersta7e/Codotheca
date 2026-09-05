use std::sync::Arc;

use serde::de::DeserializeOwned;
use serde::Deserialize;

use crate::accounts::keychain::SecretToken;
use crate::http::{HttpRequest, HttpResponse, HttpTransport, ACCOUNT_LIMITS};
use crate::provider::listing::{
    OrgListing, Page, RepoListing, Verified, Viewer, GITHUB_CANONICAL_HOST, GITHUB_HOST_ALIASES,
};
use crate::provider::{Observed, Provider, ProviderError, ProviderResult};

const GITHUB_PROVIDER_ID: &str = "github";
const GITHUB_API_BASE: &str = "https://api.github.com";
const API_VERSION: &str = "2022-11-28";

#[derive(Debug, Clone)]
pub struct GitHubProvider {
    transport: Arc<dyn HttpTransport>,
    /// The host this provider speaks to, **already folded to its canonical spelling**.
    ///
    /// Folding at construction is what makes `www.github.com` and `github.com` one provider
    /// rather than two: without it `canonical_host` would answer whichever spelling the caller
    /// happened to hold, `host_aliases` would be empty for every alias but one, and `api_base`
    /// would build `https://www.github.com/api/v3` for a host that is not an Enterprise install.
    host: String,
}

impl GitHubProvider {
    #[must_use]
    pub fn new(transport: Arc<dyn HttpTransport>, host: String) -> Self {
        let host = if GITHUB_HOST_ALIASES.contains(&host.as_str()) {
            GITHUB_CANONICAL_HOST.to_owned()
        } else {
            host
        };
        Self { transport, host }
    }

    fn api_base(&self) -> String {
        if self.host == GITHUB_CANONICAL_HOST {
            return GITHUB_API_BASE.to_owned();
        }
        format!("https://{}/api/v3", self.host)
    }

    fn user_url(&self) -> String {
        format!("{}/user", self.api_base())
    }

    fn orgs_url(&self) -> String {
        format!("{}/user/orgs?per_page=100", self.api_base())
    }

    fn repos_url(&self) -> String {
        format!(
            "{}/user/repos?per_page=100&affiliation=owner,collaborator,organization_member",
            self.api_base()
        )
    }

    fn get(&self, token: &SecretToken, url: String) -> ProviderResult<HttpResponse> {
        let request = HttpRequest {
            method: "GET",
            url,
            headers: request_headers(token),
            body: None,
            limits: ACCOUNT_LIMITS,
        };
        let response = self
            .transport
            .send(&request)
            .map_err(ProviderError::Transport)?;
        success(response)
    }
}

impl Provider for GitHubProvider {
    fn verify_token(&self, t: &SecretToken) -> ProviderResult<Observed<Verified>> {
        let response = self.get(t, self.user_url())?;
        let granted_scopes = observed_scopes(&response);
        let user: GitHubUser = decode(&response)?;
        Ok(Observed {
            value: Verified {
                login: user.login,
                display_name: user.name,
                granted_scopes: granted_scopes.clone().unwrap_or_default(),
            },
            granted_scopes,
        })
    }

    fn viewer(&self, t: &SecretToken) -> ProviderResult<Observed<Viewer>> {
        let response = self.get(t, self.user_url())?;
        let granted_scopes = observed_scopes(&response);
        let user: GitHubUser = decode(&response)?;
        Ok(Observed {
            value: Viewer {
                login: user.login,
                display_name: user.name,
            },
            granted_scopes,
        })
    }

    fn list_orgs(
        &self,
        t: &SecretToken,
        cur: Option<&str>,
    ) -> ProviderResult<Observed<Page<OrgListing>>> {
        let url = cur.map_or_else(|| self.orgs_url(), str::to_owned);
        let response = self.get(t, url.clone())?;
        let granted_scopes = observed_scopes(&response);
        let next_cursor = next_link_cursor_for(response.header("link"), &url);
        let orgs: Vec<GitHubOrg> = decode(&response)?;
        let items = orgs
            .into_iter()
            .map(|org| OrgListing {
                login: org.login,
                repo_count_seen: org.public_repos,
            })
            .collect();
        Ok(Observed {
            value: Page { items, next_cursor },
            granted_scopes,
        })
    }

    fn list_repos(
        &self,
        t: &SecretToken,
        cur: Option<&str>,
    ) -> ProviderResult<Observed<Page<RepoListing>>> {
        let url = cur.map_or_else(|| self.repos_url(), str::to_owned);
        let response = self.get(t, url.clone())?;
        let granted_scopes = observed_scopes(&response);
        let next_cursor = next_link_cursor_for(response.header("link"), &url);
        let repos: Vec<GitHubRepo> = decode(&response)?;
        let items = repos.into_iter().map(RepoListing::from).collect();
        Ok(Observed {
            value: Page { items, next_cursor },
            granted_scopes,
        })
    }

    fn canonical_host(&self) -> &str {
        &self.host
    }

    fn host_aliases(&self) -> &[&str] {
        if self.host == GITHUB_CANONICAL_HOST {
            return GITHUB_HOST_ALIASES;
        }
        &[]
    }
}

fn request_headers(token: &SecretToken) -> Vec<(String, String)> {
    vec![
        (
            "authorization".to_owned(),
            format!("Bearer {}", token.expose()),
        ),
        (
            "accept".to_owned(),
            "application/vnd.github+json".to_owned(),
        ),
        ("x-github-api-version".to_owned(), API_VERSION.to_owned()),
        ("user-agent".to_owned(), "codotheca".to_owned()),
    ]
}

fn success(response: HttpResponse) -> ProviderResult<HttpResponse> {
    if (200..300).contains(&response.status) {
        return Ok(response);
    }
    Err(ProviderError::Http {
        status: response.status,
        headers: response.headers,
    })
}

fn observed_scopes(response: &HttpResponse) -> Option<Vec<String>> {
    response.header("x-oauth-scopes").map(split_scope_header)
}

fn split_scope_header(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|scope| !scope.is_empty())
        .map(str::to_owned)
        .collect()
}

fn decode<T>(response: &HttpResponse) -> ProviderResult<T>
where
    T: DeserializeOwned,
{
    serde_json::from_slice(&response.body).map_err(|error| ProviderError::Decode(error.to_string()))
}

/// The `rel="next"` URL, **bounded to the host that produced it**.
///
/// A `Link` header is attacker-influenceable in exactly the way a `Location` is: it names a URL
/// this provider will then fetch **with the account's token attached**. Following one off-host
/// would hand the token to whatever the header named, so a cross-host next page is dropped and
/// pagination simply ends — the alternative is sending a credential somewhere the user never
/// authorised.
fn next_link_cursor_for(header: Option<&str>, from: &str) -> Option<String> {
    let next = next_link_cursor(header)?;
    match crate::http::same_host(from, &next) {
        Ok(true) => Some(next),
        // A next page on another host is dropped, and so is one whose URL will not parse: an
        // unparseable next page is not one to guess about either.
        Ok(false) | Err(_) => None,
    }
}

/// The `rel="next"` URL out of an RFC 8288 `Link` header, or `None`.
///
/// **It walks `<...>` pairs; it does not split the header on `,`.** The repos endpoint's own URL
/// carries `affiliation=owner,collaborator,organization_member`, so a comma split tears the URL
/// into pieces, finds no `rel="next"` in any of them, and paginates exactly once — silently, and
/// only against the real API, because a fixture without a comma passes either way.
fn next_link_cursor(header: Option<&str>) -> Option<String> {
    let header = header?;
    let bytes = header.as_bytes();
    let mut at = 0_usize;
    while at < bytes.len() {
        let open = header.get(at..)?.find('<')? + at;
        let close = header.get(open..)?.find('>')? + open;
        let url = header.get(open + 1..close)?;
        // Parameters run to the next link's `<`, or to the end of the header.
        let rest = header.get(close + 1..)?;
        let params_end = rest.find('<').unwrap_or(rest.len());
        let params = rest.get(..params_end)?;
        if params
            .split(';')
            .map(str::trim)
            .any(|param| param == r#"rel="next""# || param == "rel=next")
        {
            return Some(url.to_owned());
        }
        at = close + 1 + params_end;
    }
    None
}

#[derive(Debug, Deserialize)]
struct GitHubUser {
    login: String,
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GitHubOrg {
    login: String,
    public_repos: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct GitHubOwner {
    login: String,
    #[serde(rename = "type")]
    kind: String,
}

/// `push` is optional, so a `permissions` object that does not carry it is **unknown** rather
/// than a decode failure that loses the whole page. An absent permission is never `false`.
#[derive(Debug, Deserialize)]
struct GitHubPermissions {
    push: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct GitHubParent {
    clone_url: String,
}

#[derive(Debug, Deserialize)]
struct GitHubRepo {
    id: u64,
    clone_url: String,
    owner: GitHubOwner,
    name: String,
    permissions: Option<GitHubPermissions>,
    fork: bool,
    parent: Option<GitHubParent>,
    archived: bool,
    private: bool,
}

impl From<GitHubRepo> for RepoListing {
    fn from(repo: GitHubRepo) -> Self {
        let owner = repo.owner.login;
        let in_org = if repo.owner.kind == "Organization" {
            Some(owner.clone())
        } else {
            None
        };
        Self {
            provider: GITHUB_PROVIDER_ID,
            provider_repo_id: repo.id.to_string(),
            clone_url: repo.clone_url,
            owner,
            name: repo.name,
            can_push: repo.permissions.and_then(|permissions| permissions.push),
            is_fork: repo.fork,
            fork_parent_clone_url: repo.parent.map(|parent| parent.clone_url),
            is_archived: repo.archived,
            is_private: repo.private,
            in_org,
        }
    }
}
