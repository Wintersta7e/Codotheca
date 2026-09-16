use std::sync::Arc;

use serde::de::DeserializeOwned;
use serde::Deserialize;

use crate::accounts::keychain::SecretToken;
use crate::http::{HttpRequest, HttpResponse, HttpTransport, ACCOUNT_LIMITS};
use crate::provider::listing::{
    OrgListing, Page, RepoListing, Viewer, GITHUB_CANONICAL_HOST, GITHUB_HOST_ALIASES,
};
use crate::provider::{
    CiRunPayload, CiRunsRead, Observed, Provider, ProviderError, ProviderResult, RepoFactsPayload,
    RepoFactsRead,
};

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

    /// One GET, **whatever the status**. `success` is applied by the caller, because §25's two
    /// conditional reads need a `304`, a `403` and a `404` as answers rather than as errors.
    fn get_raw(
        &self,
        token: &SecretToken,
        url: String,
        etag: Option<&str>,
    ) -> ProviderResult<HttpResponse> {
        let mut headers = request_headers(token);
        if let Some(etag) = etag {
            headers.push(("if-none-match".to_owned(), etag.to_owned()));
        }
        let request = HttpRequest {
            method: "GET",
            url,
            headers,
            body: None,
            limits: ACCOUNT_LIMITS,
        };
        self.transport
            .send(&request)
            .map_err(ProviderError::Transport)
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

    fn lookup_repo(
        &self,
        t: &SecretToken,
        owner: &str,
        name: &str,
    ) -> ProviderResult<Observed<Option<RepoListing>>> {
        // `owner` and `name` come from a stored `remote_key`, whose segments are already the
        // path segments of a URL this build canonicalised. A key with more than two path
        // segments loses its middle here, which is §22.7's own rule — the repair asks for the
        // stored owner and the stored name and nothing else.
        let request = HttpRequest {
            method: "GET",
            url: format!("{}/repos/{owner}/{name}", self.api_base()),
            headers: request_headers(t),
            body: None,
            limits: ACCOUNT_LIMITS,
        };
        let response = self
            .transport
            .send(&request)
            .map_err(ProviderError::Transport)?;
        let granted_scopes = observed_scopes(&response);
        if response.status == 404 {
            return Ok(Observed {
                value: None,
                granted_scopes,
            });
        }
        let response = success(response)?;
        let repo: GitHubRepo = decode(&response)?;
        Ok(Observed {
            value: Some(RepoListing::from(repo)),
            granted_scopes,
        })
    }

    fn repo_facts(
        &self,
        t: &SecretToken,
        owner: &str,
        name: &str,
        etag: Option<&str>,
    ) -> ProviderResult<Observed<RepoFactsRead>> {
        let url = format!("{}/repos/{owner}/{name}", self.api_base());
        let response = self.get_raw(t, url, etag)?;
        let granted_scopes = observed_scopes(&response);
        let status = response.status;
        // A read that observed the access state and nothing else. The caller writes
        // `permitted = 0` and dates nothing; dating the counts by a read that returned none of
        // them is the staleness marker lying.
        if status == 304 || status == 403 || status == 404 {
            return Ok(Observed {
                value: RepoFactsRead {
                    status,
                    // A 304 confirms the caller's validator, so it is carried forward rather
                    // than dropped; the other two observed no representation at all.
                    etag: if status == 304 {
                        etag.map(str::to_owned)
                    } else {
                        None
                    },
                    facts: None,
                },
                granted_scopes,
            });
        }
        let response = success(response)?;
        let observed_etag = response.header("etag").map(str::to_owned);
        let repo: GitHubRepo = decode(&response)?;
        Ok(Observed {
            value: RepoFactsRead {
                status,
                etag: observed_etag,
                facts: Some(repo_facts_of(repo)),
            },
            granted_scopes,
        })
    }

    fn ci_runs(
        &self,
        t: &SecretToken,
        owner: &str,
        name: &str,
        etag: Option<&str>,
    ) -> ProviderResult<Observed<CiRunsRead>> {
        // §25.1 renders at most `CI_RUN_LIMIT` and §25.7 stores at most `CI_RUN_LIMIT`, so
        // asking for more spends budget on rows the writer trims away. **Derived, not typed:**
        // the literal `5` here was a third copy of a bound whose other two are mirror-tested
        // against each other (`core/src/remote/facts.rs` and `ciCopy.ts`, via
        // `app/test/remoteAllowlist.test.ts`), so raising the limit would have left the request
        // fetching five for ever with nothing to say so — R12's one-owner rule.
        let url = format!(
            "{}/repos/{owner}/{name}/actions/runs?per_page={}",
            self.api_base(),
            crate::remote::facts::CI_RUN_LIMIT
        );
        let response = self.get_raw(t, url, etag)?;
        let granted_scopes = observed_scopes(&response);
        let status = response.status;
        if status == 304 || status == 403 || status == 404 {
            return Ok(Observed {
                value: CiRunsRead {
                    status,
                    etag: if status == 304 {
                        etag.map(str::to_owned)
                    } else {
                        None
                    },
                    runs: None,
                },
                granted_scopes,
            });
        }
        let response = success(response)?;
        let observed_etag = response.header("etag").map(str::to_owned);
        let body: GitHubRuns = decode(&response)?;
        Ok(Observed {
            value: CiRunsRead {
                status,
                etag: observed_etag,
                runs: Some(
                    body.workflow_runs
                        .into_iter()
                        .map(CiRunPayload::from)
                        .collect(),
                ),
            },
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

/// The repository object into §25.7's columns.
///
/// **`open_issues_count` is deliberately not read.** It counts issues *and* pull requests, so it
/// answers neither of the two blocks §25.1 draws, and a wrong number under a labelled block is
/// worse than `—`. Those two counts, and the two sub-line counts beside them, stay unobserved
/// until a read exists that can separate them.
fn repo_facts_of(repo: GitHubRepo) -> RepoFactsPayload {
    RepoFactsPayload {
        // **Read from `private`, deliberately, and the comment used to argue for the field this
        // does not use.** §25.3 admits exactly two words, and `private` is total over them: an
        // Enterprise `internal` repository is certainly not public, so it maps to `private`
        // rather than to a third word the surface cannot render or to nothing at all. The
        // `visibility` field would carry `internal` and then need collapsing here anyway, and a
        // value neither branch recognised would render an absence where access is actually
        // restricted — the worse of the two failures.
        visibility: Some(if repo.private { "private" } else { "public" }.to_owned()),
        description: repo.description,
        fork_parent_remote_key: repo
            .parent
            .as_ref()
            .and_then(|parent| crate::identity::remote::canonical_remote_key(&parent.clone_url))
            .map(|key| key.key),
        stars: repo.stargazers_count,
        open_issues: None,
        good_first_issues: None,
        open_prs: None,
        open_prs_from_user: None,
        topics: repo.topics.unwrap_or_default(),
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
    // §25.1's fields. Every one is optional: a forge that omits one has not observed it, and a
    // missing key must never cost the whole page the way a decode failure would.
    description: Option<String>,
    stargazers_count: Option<u32>,
    topics: Option<Vec<String>>,
}

/// The Actions listing. `total_count` is deliberately unread: §25.1 renders the runs, never a
/// count of them, and there is no aggregate anywhere in phase 2.
#[derive(Debug, Deserialize)]
struct GitHubRuns {
    workflow_runs: Vec<GitHubRun>,
}

#[derive(Debug, Deserialize)]
struct GitHubRun {
    id: i64,
    name: Option<String>,
    conclusion: Option<String>,
    head_branch: Option<String>,
    run_number: Option<u32>,
    run_started_at: Option<String>,
}

impl From<GitHubRun> for CiRunPayload {
    fn from(run: GitHubRun) -> Self {
        Self {
            run_id: run.id,
            // A run with no workflow name is not a run with an empty name; the column is NOT
            // NULL, so the id it is keyed on is the honest stand-in for a name nobody sent.
            workflow_name: run.name.unwrap_or_else(|| format!("run {}", run.id)),
            conclusion: run.conclusion,
            branch: run.head_branch.unwrap_or_default(),
            run_number: run.run_number.unwrap_or_default(),
            started_at: run.run_started_at.as_deref().and_then(parse_rfc3339_secs),
        }
    }
}

/// An RFC 3339 instant into unix seconds, or `None`.
///
/// Hand-rolled because the core carries no date crate and this is the only place a forge sends a
/// formatted time. A string this cannot read is **unknown**, never the epoch: a run dated
/// 1970 would sort to the bottom of a list headed *latest* and read as a fact.
fn parse_rfc3339_secs(text: &str) -> Option<i64> {
    let bytes = text.as_bytes();
    let at = |i: usize| bytes.get(i).copied();
    if bytes.len() < 20 || at(4) != Some(b'-') || at(7) != Some(b'-') || at(10) != Some(b'T') {
        return None;
    }
    let num = |from: usize, to: usize| text.get(from..to)?.parse::<i64>().ok();
    let (year, month, day) = (num(0, 4)?, num(5, 7)?, num(8, 10)?);
    let (hour, minute, second) = (num(11, 13)?, num(14, 16)?, num(17, 19)?);
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    if hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    Some(days_from_civil(year, month, day) * 86_400 + hour * 3600 + minute * 60 + second)
}

/// Days since 1970-01-01 from a proleptic Gregorian date — Howard Hinnant's `days_from_civil`,
/// which is the algorithm every date library uses and is exact for every year this can see.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let yoe = year - era * 400;
    let doy = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
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
