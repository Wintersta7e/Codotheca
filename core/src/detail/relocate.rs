//! RELOCATE (§8.5.2, §17, criterion 44). **Rewrites one `location` row's path and nothing
//! else**: no file is moved, deleted, created or opened for writing, no git command mutates
//! anything, and no row is removed. `presence`, `scan_generation`, `volume_key`, `store_key`
//! and every observed fact stay exactly as the last scan wrote them — the row has been
//! *pointed* at a folder, not *observed* there, and §6 does not let a pointer claim currency.
//!
//! **The renderer may never originate a filesystem path** (§2.4). The new path arrives as
//! `pathBytes` — tagged bytes (§2.5) produced by the shell's native folder dialog. Two
//! independent things enforce that: the command carries `"privileged": true` in the schema, so
//! the bridge refuses it on the renderer channel; and the core takes `Bytes`, so a
//! renderer-typed string cannot deserialise and fails before this handler runs.

use std::time::Duration;

use crate::detail::DetailCtx;
use crate::git::GitError;
use crate::index::path::{PathPlatform, StoredPath};
use crate::proto::dispatch::{parse_args, CommandFailure};
use crate::protocol::{ErrorCode, LocationDetail, LocationsRelocateArgs};

/// The probe runs on the loop thread while a user waits, so it carries a deadline rather than
/// blocking the core on an unresponsive store.
const RELOCATE_PROBE_SECS: u64 = 10;

/// Maps §3.5's failures into §2.4's closed enum, so an untrusted or unreadable target refuses
/// with a code §11.1 already has prose for instead of `INTERNAL`.
fn git_failure(e: &GitError) -> CommandFailure {
    let code = e
        .protocol_code()
        .and_then(crate::projects::rows::enum_from_column::<ErrorCode>)
        .unwrap_or(ErrorCode::Internal);
    CommandFailure {
        code,
        message: e.to_string(),
        outcome: None,
    }
}

fn internal(e: impl std::fmt::Display) -> CommandFailure {
    CommandFailure::internal(e.to_string())
}

/// # Errors
/// `PROTOCOL` for an argument shape the schema does not admit or an id that names no location;
/// `REPO_UNREADABLE` when the target's root set disagrees with the project's lineage; the
/// git-derived code for an untrusted, gone or unreadable target; `INTERNAL` for an index fault.
pub fn handle_location_relocate(
    ctx: &DetailCtx<'_>,
    args: serde_json::Value,
) -> Result<LocationDetail, CommandFailure> {
    let a: LocationsRelocateArgs = parse_args(args)?;
    let conn = ctx.index.conn();
    let (project_id, kind, expected): (i64, String, Option<String>) = conn
        .query_row(
            "SELECT l.project_id, l.kind, p.lineage_key
               FROM location l JOIN project p ON p.id = l.project_id
              WHERE l.id = ?1",
            [a.location_id.0],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => {
                CommandFailure::protocol(format!("no location {}", a.location_id.0))
            }
            other => internal(other),
        })?;

    // The path exists only because the shell's dialog produced it. Nothing derives it from a
    // renderer string, and nothing here writes to it.
    let path = crate::paths::path_from_bytes(&a.path_bytes.0);
    let facts = ctx.mount.resolve(&path).map_err(internal)?;
    let repo = crate::git::RepoHandle::resolve(
        &path,
        crate::git::StoreKey::new(facts.store_key),
        facts.class,
    )
    .map_err(|e| git_failure(&e))?;

    let cancel = crate::cancel::CancelToken::new();
    let jc = crate::git::JobContext::new(
        crate::git::JobClass::Interactive,
        &cancel,
        Some(Duration::from_secs(RELOCATE_PROBE_SECS)),
    );
    let is_shallow = ctx
        .git
        .repo_facts(&repo, &jc)
        .map_err(|e| git_failure(&e))?
        .is_shallow;
    let oids: Vec<String> = ctx
        .git
        .root_commits(&repo, &jc)
        .map_err(|e| git_failure(&e))?
        .into_iter()
        .map(|c| c.oid)
        .collect();
    let found = crate::identity::lineage::lineage_key(&oids, is_shallow);

    // §8.5.2: the core refuses a target whose root set disagrees with the project's lineage.
    // Relocating must never silently re-identify one project as another.
    if found.is_none() || found != expected {
        return Err(CommandFailure {
            code: ErrorCode::RepoUnreadable,
            message: format!("lineage mismatch for location {}", a.location_id.0),
            outcome: None,
        });
    }

    let platform = if kind == "win" {
        PathPlatform::Windows
    } else {
        PathPlatform::Unix
    };
    let stored = StoredPath::from_bytes(a.path_bytes.0.clone(), platform);
    let (bytes, key, display) = stored.as_params();
    // Three columns, one row, by id. Everything else is left alone deliberately — plan 08's
    // `upsert_location` remains the only writer that creates or re-identifies a location row,
    // and this touches no identity column, which is why it does not go through it.
    conn.execute(
        "UPDATE location SET path_bytes = ?2, path_key = ?3, path_display = ?4 WHERE id = ?1",
        rusqlite::params![a.location_id.0, bytes, key, display],
    )
    .map_err(internal)?;

    crate::detail::emit_upserted(ctx, crate::protocol::ProjectId(project_id));
    crate::detail::get::location_detail(conn, a.location_id)
}
