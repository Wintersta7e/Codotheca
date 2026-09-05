//! §22.3's inputs — what the matcher is given, and the two indexed reads that produce them.
//!
//! The matcher itself is pure, so everything that touches the database lives here. Both loaders
//! take a `&Transaction` from their caller and open none of their own.
//!
//! **The fold happens in Rust, never in SQL.** The rows are narrowed by index on the *stored*
//! spellings — one statement per declared alias host, **never a `LIKE`**, because a pattern over
//! a host segment is exactly the pattern-guessing §22.2 forbids — and the comparison form is then
//! computed here, on both sides, by the one implementation.

use rusqlite::{params, Transaction};

use super::alias::{fold_host, fold_key, HostAliases};
use super::match_listing::ListingEvidence;
use super::IdentityError;

/// The id read, exposed so a test can `EXPLAIN QUERY PLAN` **the statement that actually runs**
/// rather than a copy of it that may drift from it.
pub const LINK_CANDIDATES_BY_ID_SQL: &str =
    "SELECT id, provider, provider_repo_id, remote_key, created_at
   FROM project
  WHERE provider = ?1 AND provider_repo_id = ?2 AND merged_into IS NULL";

/// The key read, one execution per declared host spelling.
pub const LINK_CANDIDATES_BY_KEY_SQL: &str =
    "SELECT id, provider, provider_repo_id, remote_key, created_at
   FROM project
  WHERE remote_key = ?1 AND merged_into IS NULL";

/// The suppressor read (§22.6). **No index can serve it and none is added**: the rule compares a
/// *path component* against projects on an arbitrary other host, and the set of other hosts is
/// not bounded — an undeclared SSH-config alias is precisely the case it exists for. The scan is
/// narrowed to projects that carry a remote **and** at least one `location` row, which is the
/// clause §22.6 leaves implicit and the one that makes the rule correct.
const SUPPRESSORS_SQL: &str = "SELECT id, name, remote_key
   FROM project
  WHERE merged_into IS NULL
    AND remote_key IS NOT NULL
    AND EXISTS (SELECT 1 FROM location WHERE location.project_id = project.id)
  ORDER BY created_at, id";

/// Every non-tombstoned project this listing might be: equal on `(provider, provider_repo_id)`,
/// or equal on the folded `remote_key`.
///
/// Ordered `(created_at, id)`, and each project appears **once** — a project found by both reads
/// returned twice would read to the matcher as §22.5's ambiguity.
pub fn load_link_candidates(
    tx: &Transaction<'_>,
    ev: &ListingEvidence,
    aliases: &HostAliases,
) -> Result<Vec<LinkCandidate>, IdentityError> {
    let mut found: Vec<LinkCandidate> = Vec::new();

    let mut by_id = tx.prepare(LINK_CANDIDATES_BY_ID_SQL)?;
    for row in by_id.query_map(params![ev.provider, ev.provider_repo_id], read_candidate)? {
        push_once(&mut found, row?, aliases);
    }

    // The stored key may carry any declared spelling of the host, so the narrowing read runs once
    // per spelling. The listing's own key is always probed, which is what covers an undeclared
    // host — it folds to itself and belongs to no alias set.
    let mut by_key = tx.prepare(LINK_CANDIDATES_BY_KEY_SQL)?;
    for spelling in stored_spellings(ev, aliases) {
        for row in by_key.query_map(params![spelling], read_candidate)? {
            push_once(&mut found, row?, aliases);
        }
    }

    // The fold is applied to both sides here, by the one implementation, and a row a spelling
    // read reached whose folded key does not in fact match is dropped.
    found.retain(|c| {
        let by_key = c.folded_key.as_deref() == Some(ev.folded_key.as_str());
        let by_id = c.provider.as_deref() == Some(ev.provider.as_str())
            && c.provider_repo_id.as_deref() == Some(ev.provider_repo_id.as_str());
        by_key || by_id
    });
    found.sort_by_key(|c| (c.created_at, c.project_id));
    Ok(found)
}

/// §22.6 — the projects that withhold a `Create`.
///
/// The same folded **path component** — everything after the first `/`, so group paths compare
/// whole — on a **different** canonical host, with at least one `location` row.
pub fn load_suppressors(
    tx: &Transaction<'_>,
    ev: &ListingEvidence,
    aliases: &HostAliases,
) -> Result<Vec<Suppressor>, IdentityError> {
    let Some((listing_host, listing_path)) = ev.folded_key.split_once('/') else {
        return Ok(Vec::new());
    };

    let mut st = tx.prepare(SUPPRESSORS_SQL)?;
    let rows = st.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
        ))
    })?;

    let mut out = Vec::new();
    for row in rows {
        let (project_id, name, stored) = row?;
        let Some(folded) = fold_key(&stored, aliases) else {
            continue;
        };
        let Some((host, path)) = folded.split_once('/') else {
            continue;
        };
        if path == listing_path && fold_host(host, aliases) != listing_host {
            out.push(Suppressor {
                project_id,
                folded_key: folded,
                name,
            });
        }
    }
    Ok(out)
}

/// Every spelling of this listing's key that a stored `remote_key` could carry.
fn stored_spellings(ev: &ListingEvidence, aliases: &HostAliases) -> Vec<String> {
    let mut out = vec![ev.remote_key.clone()];
    let Some((host, path)) = ev.folded_key.split_once('/') else {
        return out;
    };
    if !aliases.contains(host) {
        return out;
    }
    for spelling in aliases.spellings() {
        let candidate = format!("{spelling}/{path}");
        if !out.contains(&candidate) {
            out.push(candidate);
        }
    }
    out
}

fn read_candidate(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawCandidate> {
    Ok(RawCandidate {
        project_id: row.get(0)?,
        provider: row.get(1)?,
        provider_repo_id: row.get(2)?,
        remote_key: row.get(3)?,
        created_at: row.get(4)?,
    })
}

fn push_once(found: &mut Vec<LinkCandidate>, raw: RawCandidate, aliases: &HostAliases) {
    if found.iter().any(|c| c.project_id == raw.project_id) {
        return;
    }
    found.push(LinkCandidate {
        project_id: raw.project_id,
        provider: raw.provider,
        provider_repo_id: raw.provider_repo_id,
        folded_key: raw
            .remote_key
            .as_deref()
            .and_then(|key| fold_key(key, aliases)),
        created_at: raw.created_at,
    });
}

/// The row as it comes off the statement, before the fold.
struct RawCandidate {
    project_id: i64,
    provider: Option<String>,
    provider_repo_id: Option<String>,
    remote_key: Option<String>,
    created_at: i64,
}

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
