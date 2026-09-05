//! §22.3's inputs — what the matcher is given, and the two indexed reads that produce them.
//!
//! The matcher itself is pure, so everything that touches the database lives here. Both loaders
//! take a `&Transaction` from their caller and open none of their own.

/// A project the listing might be. Narrowed by index before the matcher sees it, and ordered
/// `(created_at, id)` so the outcome never depends on the order a walk happened to reach a row
/// in — the same discipline `store::load_candidates` already uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkCandidate {
    pub project_id: i64,
    /// `None` means *not yet resolved*, which is not the same as a different forge.
    pub provider: Option<String>,
    /// `None` means *not yet resolved*. §22.3: unknown does not exclude a candidate, and
    /// known-and-different does.
    pub provider_repo_id: Option<String>,
    /// §22.2's comparison form of `project.remote_key`. `None` for a project with no remote.
    pub folded_key: Option<String>,
    pub created_at: i64,
}

/// A project that withholds a `Create` (§22.6): the same path component on a **different**
/// canonical host, and at least one `location` row.
///
/// Drawing a `NOT CLONED` tile is a positive claim about the user's disk, and a copy the app
/// cannot fold into this listing is exactly the doubt that makes that claim unsafe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suppressor {
    pub project_id: i64,
    pub folded_key: String,
    pub name: String,
}
