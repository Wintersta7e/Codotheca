//! OAuth scope tiers for account connection.
//!
//! The public tier is structurally read-only for repository data: it can identify the user and
//! read public identity metadata, but cannot ask the forge for private repositories. The private
//! tier adds the forge's repository scope and organisation read scope because the forge exposes no
//! read-only private repository variant; that split is the product decision the UI must surface.
//!
//! No destructive, workflow, snippet, notification, administration or write-prefixed scope belongs
//! in either tier.

/// Public repository access: identity only, with no private repository grant.
pub const SCOPES_PUBLIC: &[&str] = &["read:user", "user:email"];

/// Private repository access.
///
/// The `repo` grant is both read and write on the forge, because no read-only private repository
/// scope exists. Keeping it out of [`SCOPES_PUBLIC`] is what makes the default tier incapable of
/// writing anything.
pub const SCOPES_PRIVATE: &[&str] = &["read:user", "user:email", "repo", "read:org"];

/// The one scope whose presence separates the public tier from the private one.
///
/// It is declared here rather than anywhere else, so this module stays the only place in the
/// core that writes a scope string and `core/tests/provider_scope_audit.rs` keeps meaning what it
/// says. A test asserts it is a member of [`SCOPES_PRIVATE`] and absent from [`SCOPES_PUBLIC`],
/// which is the pair of facts that makes it *the* separator rather than a third opinion.
pub const PRIVATE_TIER_SCOPE: &str = "repo";
