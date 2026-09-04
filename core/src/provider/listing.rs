//! Provider listing DTOs.

/// GitHub.com's canonical repository host.
pub const GITHUB_CANONICAL_HOST: &str = "github.com";

/// Finite GitHub host aliases recognised by the provider.
///
/// `gist.github.com` is deliberately excluded: a gist is not a repository.
pub const GITHUB_HOST_ALIASES: &[&str] = &["github.com", "www.github.com", "ssh.github.com"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verified {
    pub login: String,
    pub display_name: Option<String>,
    pub granted_scopes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Viewer {
    pub login: String,
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrgListing {
    pub login: String,
    pub repo_count_seen: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub next_cursor: Option<String>,
}

#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoListing {
    pub provider: &'static str,
    pub provider_repo_id: String,
    pub clone_url: String,
    pub owner: String,
    pub name: String,
    pub can_push: Option<bool>,
    pub is_fork: bool,
    pub fork_parent_clone_url: Option<String>,
    pub is_archived: bool,
    pub is_private: bool,
    pub in_org: Option<String>,
}
