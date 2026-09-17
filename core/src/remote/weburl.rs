//! §25.2 — the host allowlist, stated once, and the one place a URL is constructed.
//!
//! `remote_key` is derived from repository content the user may not have written — a cloned
//! repository's config, a submodule's — so a renderer-supplied or renderer-relayed URL is hostile
//! input aimed at the process that owns the dialogs. **No URL crosses IPC inbound**: the renderer
//! names `{ projectId, kind }`, the core reads the stored key and builds the string, and the
//! shell re-asserts the same allowlist on the answer before it opens anything.
//!
//! A9's derivation rule is narrow and is testable: the URL derives from **this project's**
//! `remote_key` only. `fork_parent_remote_key` is a rendered string and reaches no construction
//! here, so a fork parent on an allowlisted host cannot make its off-allowlist child linkable.

use rusqlite::{Connection, OptionalExtension as _};

use crate::proto::dispatch::{parse_args, CommandFailure};
use crate::protocol::{RemoteLinkKind, RemoteWebUrlArgs};
use crate::remote::RemoteCtx;

/// The hosts every install trusts. The Enterprise host of a connected account joins it at
/// runtime and is never a literal on either side of the process boundary.
pub const ALLOWLIST_BASE: &[&str] = &["github.com"];

/// Whether `host` may produce a link.
///
/// `enterprise` is the host column of the account rows this install holds. An account row exists
/// because the user connected it, so its host is configured by definition; a disabled account is
/// still a configured host, and dropping it would make a link disappear for a reason the user
/// never asked for.
#[must_use]
pub fn is_allowlisted_host(host: &str, enterprise: &[String]) -> bool {
    ALLOWLIST_BASE.iter().any(|base| base == &host)
        || enterprise.iter().any(|known| known.as_str() == host)
}

/// The path suffix for each link kind. `repository` has none.
const fn suffix_of(kind: RemoteLinkKind) -> &'static str {
    match kind {
        RemoteLinkKind::Repository => "",
        RemoteLinkKind::Issues => "/issues",
        RemoteLinkKind::Pulls => "/pulls",
        RemoteLinkKind::Actions => "/actions",
        RemoteLinkKind::Releases => "/releases",
    }
}

/// `https://<host>/<owner>/<name>` plus `kind`'s suffix, or `None`.
///
/// `None` for an unlisted host and for a key that is not **exactly** three segments. §1.1's key
/// is `<host>/<owner>/<name>`; `canonical_remote_key` will keep a longer path for a forge that
/// nests groups, and an https form guessed from one is a URL the product invented.
#[must_use]
pub fn web_url(remote_key: &str, kind: RemoteLinkKind, enterprise: &[String]) -> Option<String> {
    let mut segments = remote_key.split('/');
    let host = segments.next()?;
    let owner = segments.next()?;
    let name = segments.next()?;
    if segments.next().is_some() {
        return None;
    }
    if host.is_empty() || owner.is_empty() || name.is_empty() {
        return None;
    }
    if !is_allowlisted_host(host, enterprise) {
        return None;
    }
    Some(format!("https://{host}/{owner}/{name}{}", suffix_of(kind)))
}

/// The hosts of every account row this install holds.
///
/// One source, read here and mirrored nowhere: §20 owns the account row, and a second copy of a
/// host — in a settings field, in the shell — is R12's shape on the value that decides whether a
/// URL opens.
///
/// # Errors
/// Fails when the account table cannot be read.
pub fn enterprise_hosts(conn: &Connection) -> Result<Vec<String>, rusqlite::Error> {
    let mut stmt = conn.prepare("SELECT DISTINCT host FROM account")?;
    let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
    rows.collect()
}

fn remote_key_of(
    conn: &Connection,
    project: crate::protocol::ProjectId,
) -> Result<Option<String>, rusqlite::Error> {
    conn.query_row(
        "SELECT remote_key FROM project WHERE id = ?1 AND merged_into IS NULL",
        rusqlite::params![project.0],
        |r| r.get::<_, Option<String>>(0),
    )
    .optional()
    .map(Option::flatten)
}

/// `remote.webUrl`.
///
/// # Errors
/// `PROTOCOL` for an argument shape the schema does not admit; `INTERNAL` for an index fault.
/// An id that names no live project, a NULL `remote_key` and an unlistable host are all the same
/// answer — `null` — because each of them means *this project produces no link*.
pub fn handle_web_url(
    ctx: &RemoteCtx<'_>,
    args: serde_json::Value,
) -> Result<serde_json::Value, CommandFailure> {
    let args: RemoteWebUrlArgs = parse_args(args)?;
    let conn = ctx.index.conn();
    let internal = |e: rusqlite::Error| CommandFailure::internal(e.to_string());
    let Some(key) = remote_key_of(conn, args.project_id).map_err(internal)? else {
        return Ok(serde_json::Value::Null);
    };
    let hosts = enterprise_hosts(conn).map_err(internal)?;
    serde_json::to_value(web_url(&key, args.kind, &hosts))
        .map_err(|e| CommandFailure::internal(e.to_string()))
}
