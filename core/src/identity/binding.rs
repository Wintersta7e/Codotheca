//! §22.11 — the identity binding, in one shape.
//!
//! *"The identity binding — `provider`, `provider_repo_id`, `remote_link_basis` — is what says
//! this project row is that forge repository."* It is declared here because this is where a
//! binding is first written, and because two other plans consume it while no manifest declared
//! it — R16's shape, which `registry-check.py` is structurally blind to.
//!
//! **`remote_link_basis` is the matcher's and never reaches a facts row.** §25.7's three tables
//! are keyed `(provider, provider_repo_id)` and are a fact cache with no identity role: they
//! **reference** the binding and are not a second home for it. A `remote_*` table that grew a
//! basis column would put identity on a row §22.9 forbids keying identity on — which is why the
//! facts writers take a `&RemoteBinding` and call [`RemoteBinding::key`], and why the migration
//! this plan owns asserts no `remote_*` table carries the column.

use rusqlite::{params, OptionalExtension as _, Transaction};

use super::IdentityError;
use crate::protocol::RemoteLinkBasis;

impl RemoteLinkBasis {
    /// The stored form, matching `remote_link_basis`'s CHECK character for character.
    ///
    /// **The schema's variant strings are the stored strings.** Codegen emits
    /// `#[serde(rename = "<schema literal>")]` per variant, so the wire form is the stored form
    /// and `providerId` — the generated Rust identifier — is never a value.
    #[must_use]
    pub const fn slug(self) -> &'static str {
        match self {
            Self::ProviderId => "provider_id",
            Self::RemoteKey => "remote_key",
        }
    }

    /// `slug`'s inverse. `None` for anything else: a basis this build does not know is a row from
    /// a newer schema, and guessing would name evidence the app does not have.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "provider_id" => Some(Self::ProviderId),
            "remote_key" => Some(Self::RemoteKey),
            _ => None,
        }
    }
}

/// Which forge repository a `project` row **is**.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteBinding {
    pub provider: String,
    pub provider_repo_id: String,
    /// Which evidence established the link. `None` on a binding that has not been written yet.
    pub remote_link_basis: Option<RemoteLinkBasis>,
}

impl RemoteBinding {
    /// The facts-row key, and the **only** part of a binding a `remote_*` table may see.
    #[must_use]
    pub fn key(&self) -> (&str, &str) {
        (&self.provider, &self.provider_repo_id)
    }
}

/// The **only** writer of the three binding columns.
///
/// It sets `updated_at` and touches nothing else — not `remote_key`, which recomputes from
/// `git config` on every scan and is the git-derived `xp_events` key's component (§1.7, §22.7),
/// and not `association_kind`, which records how a *location* came to be under a project and has
/// nothing to say about a forge listing (§22.11).
pub fn write_binding(
    tx: &Transaction<'_>,
    project_id: i64,
    binding: &RemoteBinding,
    now: i64,
) -> Result<(), IdentityError> {
    tx.execute(
        "UPDATE project
            SET provider = ?2, provider_repo_id = ?3, remote_link_basis = ?4, updated_at = ?5
          WHERE id = ?1",
        params![
            project_id,
            binding.provider,
            binding.provider_repo_id,
            binding.remote_link_basis.map(RemoteLinkBasis::slug),
            now,
        ],
    )?;
    Ok(())
}

/// The binding a project carries, or `None`.
///
/// `None` when `provider` or `provider_repo_id` is NULL, which is *not yet resolved* and is the
/// honest value — an empty string would compare equal to every other project that has none.
pub fn load_binding(
    tx: &Transaction<'_>,
    project_id: i64,
) -> Result<Option<RemoteBinding>, IdentityError> {
    let row: Option<(Option<String>, Option<String>, Option<String>)> = tx
        .query_row(
            "SELECT provider, provider_repo_id, remote_link_basis FROM project WHERE id = ?1",
            params![project_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;

    let Some((Some(provider), Some(provider_repo_id), basis)) = row else {
        return Ok(None);
    };
    Ok(Some(RemoteBinding {
        provider,
        provider_repo_id,
        remote_link_basis: basis.as_deref().and_then(RemoteLinkBasis::parse),
    }))
}
