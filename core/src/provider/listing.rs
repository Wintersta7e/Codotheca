//! Provider listing DTOs.

/// GitHub.com's canonical repository host.
pub const GITHUB_CANONICAL_HOST: &str = "github.com";

/// Finite GitHub host aliases recognised by the provider.
///
/// `gist.github.com` is deliberately excluded: a gist is not a repository.
pub const GITHUB_HOST_ALIASES: &[&str] = &["github.com", "www.github.com", "ssh.github.com"];

/// Who a token authenticates as.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Viewer {
    /// The account's login on the forge.
    pub login: String,
    /// The name the user chose to display; `None` when they set none.
    pub display_name: Option<String>,
}

/// One organisation the token's user belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrgListing {
    /// The organisation's login on the forge.
    pub login: String,
    /// The organisation's public repository count as the listing reported it; `None` when unsent.
    pub repo_count_seen: Option<u32>,
}

/// One page of a paginated listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page<T> {
    /// The page's entries, in the order the forge listed them.
    pub items: Vec<T>,
    /// Where the next page is — its URL, on the host that answered this one; `None` on the last
    /// page.
    pub next_cursor: Option<String>,
}

/// One repository as the forge reports it in a listing page or a single lookup.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoListing {
    /// The provider's id — the `provider` column a binding is keyed by.
    pub provider: &'static str,
    /// The forge's own id for the repository, as text.
    pub provider_repo_id: String,
    /// The clone URL the forge reports.
    pub clone_url: String,
    /// The owning user's or organisation's login.
    pub owner: String,
    /// The repository's name.
    pub name: String,
    /// Whether the account can push, from the listing's permission object; `None` when the entry
    /// carried no push permission, which is unknown rather than `false`.
    pub can_push: Option<bool>,
    /// Whether the forge reports the repository as a fork.
    pub is_fork: bool,
    /// The parent's clone URL, when the listing names a parent.
    pub fork_parent_clone_url: Option<String>,
    /// Whether the forge marks the repository archived.
    pub is_archived: bool,
    /// Whether the repository is private.
    pub is_private: bool,
    /// The owning organisation's login when an organisation owns it; `None` for a user's own.
    pub in_org: Option<String>,
}
