//! §22.7 — one bounded, additive listing read per unmatched `remote_key`.
//!
//! A repository that was renamed or transferred keeps its stable forge id and loses its path, so
//! the listing's key no longer equals the local clone's and §22.1's second basis fails. The
//! repair asks the forge what the stored path resolves to **once**, and writes the id it gets
//! back.
//!
//! | | |
//! |---|---|
//! | When | **after a sync completes** — on the terminal `Done` outcome only, never on a `NextPage` and never on a throttled park, because *"no listing matched"* is not knowable until the listing ends |
//! | Bounded | one request per unmatched `remote_key`, once. A project that resolves carries an id afterwards and is never asked again |
//! | Budget | **§21's.** These requests are accounted against `sync_budget` by the caller; this module **implements no budget of its own and holds no budget row** — a second accounting of one pool is R12's shape against the value that decides whether the app burns an account's allowance |
//! | Additive | on 403, 429 or offline the fields stay **unknown**, never `failed`; no retry loop, and the sync outcome is not an error |
//! | Reads | no git object, no history, no working copy. A **listing read, not a fetch** |
//! | Writes | `provider`, `provider_repo_id`, `remote_link_basis = 'provider_id'`, `updated_at`. **Never `remote_key`** |
//!
//! **The lock is held only around the writes.** Every lookup happens with no guard at all, on
//! `Route::Scan`'s and R75's precedent: the process has one `rusqlite::Connection` behind one
//! `std::sync::Mutex`, and holding it across a network call stalls every command.

use std::sync::{Arc, Mutex};

use crate::accounts::keychain::SecretToken;
use crate::identity::alias::HostAliases;
use crate::identity::binding::{write_binding, RemoteBinding};
use crate::identity::remote::owner_of;
use crate::identity::IdentityError;
use crate::index::{Index, IndexError};
use crate::protocol::RemoteLinkBasis;
use crate::provider::Provider;

/// What one repair pass did. **Counts, never a failure**: an unresolved project is unknown.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RepairReport {
    /// Lookups issued. Never more than the number of unmatched projects.
    pub attempted: usize,
    /// Bindings written.
    pub resolved: usize,
    /// Lookups that answered *not found*, or that the forge refused or the network lost.
    pub unknown: usize,
}

/// One project the repair may ask about, and the two segments it asks with.
struct Unmatched {
    project_id: i64,
    owner: String,
    name: String,
}

pub fn repair_renames(
    index: &Arc<Mutex<Index>>,
    provider: &dyn Provider,
    token: &SecretToken,
    aliases: &HostAliases,
    now: i64,
) -> Result<RepairReport, IdentityError> {
    let unmatched = read_unmatched(index, aliases)?;

    // No guard is held here, and that is the point.
    let mut report = RepairReport::default();
    let mut resolved: Vec<(i64, RemoteBinding)> = Vec::new();
    for candidate in &unmatched {
        report.attempted += 1;
        match provider.lookup_repo(token, &candidate.owner, &candidate.name) {
            Ok(observed) => match observed.value {
                Some(listing) => resolved.push((
                    candidate.project_id,
                    RemoteBinding {
                        provider: listing.provider.to_owned(),
                        provider_repo_id: listing.provider_repo_id,
                        // The id **is** the evidence, so the basis is `provider_id`. This is the
                        // one place a binding is promoted from the key basis.
                        remote_link_basis: Some(RemoteLinkBasis::ProviderId),
                    },
                )),
                // Not found, and not an error: the fields stay unknown.
                None => report.unknown += 1,
            },
            // A 403, a 429 and an offline lookup are the same answer here — unknown. There is no
            // retry loop, and the sync outcome is not an error.
            Err(_) => report.unknown += 1,
        }
    }

    report.resolved = resolved.len();
    if !resolved.is_empty() {
        let mut guard = index.lock().map_err(|_| poisoned())?;
        guard
            .with_tx(|tx| {
                for (project_id, binding) in &resolved {
                    write_binding(tx, *project_id, binding, now).map_err(as_index_error)?;
                }
                Ok(())
            })
            .map_err(IdentityError::Index)?;
    }
    Ok(report)
}

/// Every non-tombstoned project on a host this provider declares that carries a `remote_key` and
/// **no** `provider_repo_id`.
///
/// No id after the listing settled *is* "no listing matched": the ingest writes one on every
/// attach and on every create, so a project without one was reached by no entry on the page.
fn read_unmatched(
    index: &Arc<Mutex<Index>>,
    aliases: &HostAliases,
) -> Result<Vec<Unmatched>, IdentityError> {
    let guard = index.lock().map_err(|_| poisoned())?;
    let mut st = guard.conn().prepare(
        "SELECT id, remote_key FROM project
          WHERE remote_key IS NOT NULL AND provider_repo_id IS NULL AND merged_into IS NULL
          ORDER BY created_at, id",
    )?;
    let rows = st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;

    let mut out = Vec::new();
    for row in rows {
        let (project_id, key) = row?;
        let Some((host, _)) = key.split_once('/') else {
            continue;
        };
        if !aliases.contains(host) {
            continue;
        }
        // **From the stored `remote_key`, never from `project.owner` and `project.name`** — those
        // are a display name and a directory basename, and neither is a forge path.
        let (Some(owner), Some(name)) = (owner_of(&key), key.rsplit('/').next()) else {
            continue;
        };
        out.push(Unmatched {
            project_id,
            owner: owner.to_owned(),
            name: name.to_owned(),
        });
    }
    Ok(out)
}

fn poisoned() -> IdentityError {
    IdentityError::Index(IndexError::Corrupt {
        detail: "the index mutex is poisoned".to_owned(),
    })
}

fn as_index_error(e: IdentityError) -> IndexError {
    match e {
        IdentityError::Sqlite(e) => IndexError::Sqlite(e),
        IdentityError::Index(e) => e,
        other => IndexError::Corrupt {
            detail: format!("identity: {other:?}"),
        },
    }
}
