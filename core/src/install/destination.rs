//! §24.3a and §24.3d: where a clone lands, and the refusals that are replies rather than failures.
//!
//! **The renderer names a destination by `RootId` and nothing else.** The subpath is the project's
//! stored `seed_basename` — write-once, the directory basename recorded at first index, and the
//! only art-seed input (§7.4) — so the basename the scanner reads back after the clone is
//! identical to the one the blueprint was seeded on.
//!
//! **The core refuses; it never transforms.** A `-2` suffix is never generated. A clone landing in
//! `myapp-2` for remote `myapp` is the exact failure §7.4 records: a transformed basename is not
//! `seed_basename`, so the art re-rolls mid-scan for a project the user did not touch.

use std::io::ErrorKind;
use std::path::PathBuf;

use rusqlite::{OptionalExtension as _, Transaction};

use crate::accounts::store::account_identity;
use crate::paths::{path_display, path_from_bytes, path_key};
use crate::protocol::{
    AccountId, InstallDestination, InstallRefusal, ProjectId, RemoteLinkKind, RootId, ScopeTier,
};
use crate::remote::weburl::{enterprise_hosts, web_url};
use crate::scan::run::platform_of;
use crate::sync::tasks::remote::account_for_project;

/// Windows reserved device names, which are refused **whatever their case and whatever extension
/// follows** — `aux`, `AUX`, `Aux.txt` and `aux.tar.gz` all name the same device.
///
/// These are refused on **every** target, not only on Windows. A library is a portable artefact:
/// a repository cloned into `com1/` on Linux is one that cannot be checked out, scanned or removed
/// on Windows, and the failure would arrive on a different machine from the one that caused it.
const RESERVED_DEVICE_NAMES: &[&str] = &[
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
    "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

/// §24.3d's whole list: is this a legal single path segment on both targets?
///
/// Refuses `.`, `..`, anything containing a path separator or one of `: * ? " < > |`, any control
/// character, a Windows reserved device name, and — beyond the spec's list, because they are the
/// same class of hazard — the empty string, a leading `-`, and a trailing space or dot.
///
/// **A trailing space or dot is silently stripped by the Windows filesystem**, so `widget.` and
/// `widget ` both become `widget`. Admitting one would mean the directory created is not the
/// directory named, which breaks the `seed_basename` identity this module exists to preserve just
/// as surely as a `-2` suffix does.
#[must_use]
pub fn is_safe_path_segment(segment: &str) -> bool {
    if segment.is_empty() || segment == "." || segment == ".." {
        return false;
    }
    // A segment beginning with `-` is a path that argv reads as an option.
    if segment.starts_with('-') {
        return false;
    }
    if segment.ends_with(' ') || segment.ends_with('.') {
        return false;
    }
    if segment.chars().any(|c| {
        c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
    }) {
        return false;
    }
    // `aux`, `AUX`, `aux.txt` and `aux.tar.gz` all name the device: the stem is what matters.
    let stem = segment.split('.').next().unwrap_or(segment);
    !RESERVED_DEVICE_NAMES
        .iter()
        .any(|name| stem.eq_ignore_ascii_case(name))
}

/// The refusal for a name that cannot be a directory.
#[must_use]
pub fn refuse_unsafe_name(segment: &str) -> Option<InstallRefusal> {
    (!is_safe_path_segment(segment)).then_some(InstallRefusal::UnsafeName)
}

#[derive(Debug)]
struct RootRow {
    kind: String,
    distro: String,
    path: PathBuf,
    enabled: bool,
}

fn selected_root(tx: &Transaction<'_>, root: RootId) -> rusqlite::Result<Option<RootRow>> {
    tx.query_row(
        "SELECT kind, distro, path_bytes, enabled FROM scan_root WHERE id = ?1",
        [root.0],
        |row| {
            Ok(RootRow {
                kind: row.get(0)?,
                distro: row.get(1)?,
                path: path_from_bytes(&row.get::<_, Vec<u8>>(2)?),
                enabled: row.get::<_, i64>(3)? != 0,
            })
        },
    )
    .optional()
}

fn project_seed(tx: &Transaction<'_>, project: ProjectId) -> rusqlite::Result<Option<String>> {
    tx.query_row(
        "SELECT seed_basename FROM project WHERE id = ?1 AND merged_into IS NULL",
        [project.0],
        |row| row.get(0),
    )
    .optional()
}

/// The HTTPS clone URL for one live project.
///
/// It comes from the same constructor as every rendered repository URL. `None` means the key is
/// absent, malformed or outside the configured host allowlist; Install never invents a second
/// spelling for the endpoint Git will contact.
///
/// # Errors
/// Fails when the project or account-host tables cannot be read.
pub fn clone_url(tx: &Transaction<'_>, project: ProjectId) -> rusqlite::Result<Option<String>> {
    let remote_key = tx
        .query_row(
            "SELECT remote_key FROM project WHERE id = ?1 AND merged_into IS NULL",
            [project.0],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()?
        .flatten();
    let Some(remote_key) = remote_key else {
        return Ok(None);
    };
    let hosts = enterprise_hosts(tx)?;
    Ok(web_url(&remote_key, RemoteLinkKind::Repository, &hosts))
}

fn remote_visibility(tx: &Transaction<'_>, project: ProjectId) -> rusqlite::Result<Option<String>> {
    tx.query_row(
        "SELECT remote_repo.visibility
           FROM project
           JOIN remote_repo
             ON remote_repo.provider = project.provider
            AND remote_repo.provider_repo_id = project.provider_repo_id
          WHERE project.id = ?1 AND project.merged_into IS NULL",
        [project.0],
        |row| row.get::<_, Option<String>>(0),
    )
    .optional()
    .map(Option::flatten)
}

fn scope_tier(tx: &Transaction<'_>, account: AccountId) -> rusqlite::Result<ScopeTier> {
    account_identity(tx, account)
        .map(|identity| identity.scope_tier)
        .map_err(|error| match error {
            crate::accounts::store::AccountError::Sqlite(error) => error,
            other => rusqlite::Error::ToSqlConversionFailure(Box::new(other)),
        })
}

fn private_needs_upgrade(tx: &Transaction<'_>, project: ProjectId) -> rusqlite::Result<bool> {
    // NULL is unknown, not public. Only an explicit private observation can earn this refusal.
    if remote_visibility(tx, project)?.as_deref() != Some("private") {
        return Ok(false);
    }
    let Some(account) = account_for_project(tx, project) else {
        return Ok(false);
    };
    Ok(scope_tier(tx, account)? == ScopeTier::Public)
}

fn is_scan_root(
    tx: &Transaction<'_>,
    root: &RootRow,
    destination_key: &[u8],
) -> rusqlite::Result<bool> {
    tx.query_row(
        "SELECT EXISTS (
             SELECT 1 FROM scan_root
              WHERE kind = ?1 AND distro = ?2 AND path_key = ?3
         )",
        rusqlite::params![root.kind, root.distro, destination_key],
        |row| row.get::<_, i64>(0),
    )
    .map(|exists| exists != 0)
}

fn is_known_location(
    tx: &Transaction<'_>,
    project: ProjectId,
    root: &RootRow,
    destination_key: &[u8],
) -> rusqlite::Result<bool> {
    tx.query_row(
        "SELECT EXISTS (
             SELECT 1 FROM location
              WHERE project_id = ?1 AND kind = ?2 AND distro = ?3 AND path_key = ?4
                AND removed_at IS NULL
         )",
        rusqlite::params![project.0, root.kind, root.distro, destination_key],
        |row| row.get::<_, i64>(0),
    )
    .map(|exists| exists != 0)
}

/// Compose the destination while preserving database failures for the command surface.
///
/// Refusals are evaluated in this order: the named root must exist, be enabled and be a usable
/// directory; the live project must have a URL from the shared HTTPS constructor; its exact
/// stored `seed_basename` must be safe; an explicitly private repository must have a private-tier
/// account; the composed path must not itself be any scan root; then an occupied path is classified
/// solely from a non-removed persisted location for this project. No Git process is spawned and no
/// repository metadata is read. `NULL` visibility is unknown and passes the privacy check.
pub(crate) fn compose_destination_checked(
    tx: &Transaction<'_>,
    project: ProjectId,
    root_id: RootId,
) -> rusqlite::Result<Result<InstallDestination, InstallRefusal>> {
    let Some(root) = selected_root(tx, root_id)? else {
        return Ok(Err(InstallRefusal::RootUnavailable));
    };
    if !root.enabled || !std::fs::metadata(&root.path).is_ok_and(|metadata| metadata.is_dir()) {
        return Ok(Err(InstallRefusal::RootUnavailable));
    }
    if clone_url(tx, project)?.is_none() {
        return Ok(Err(InstallRefusal::NoCloneUrl));
    }
    // An absent row is not a refusal and must not borrow one: `no_clone_url` would state a
    // reason that is not the reason. `clone_url` above reads the same row under the same
    // `merged_into IS NULL` guard, so reaching here with no row means the project vanished
    // between the two reads — a storage fault for the command surface, not an answer.
    let Some(seed_basename) = project_seed(tx, project)? else {
        return Err(rusqlite::Error::QueryReturnedNoRows);
    };
    if refuse_unsafe_name(&seed_basename).is_some() {
        return Ok(Err(InstallRefusal::UnsafeName));
    }
    if private_needs_upgrade(tx, project)? {
        return Ok(Err(InstallRefusal::PrivateNeedsUpgrade));
    }

    let destination = root.path.join(&seed_basename);
    let destination_key = path_key(&destination, platform_of(&root.kind));
    if is_scan_root(tx, &root, &destination_key)? {
        return Ok(Err(InstallRefusal::DestinationExists));
    }

    match std::fs::symlink_metadata(&destination) {
        Ok(_) if is_known_location(tx, project, &root, &destination_key)? => {
            Ok(Err(InstallRefusal::AlreadyInstalled))
        }
        Ok(_) => Ok(Err(InstallRefusal::DestinationExists)),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(Ok(InstallDestination {
            root_id,
            seed_basename,
            display: path_display(&destination),
        })),
        // A stat that fails for any other reason — a permission wall, a parent that stopped
        // being a directory — establishes NOTHING about the destination. §24.3d conditions
        // `destination_exists` on the path *existing*, and claiming it here would be claiming
        // currency we do not have: we could not look. What we did observe is that the root is
        // not usable for this composition, which is what `root_unavailable` says.
        Err(_) => Ok(Err(InstallRefusal::RootUnavailable)),
    }
}

/// Compose `<root>/<stored seed_basename>` or return one of Task 10's six refusals.
///
/// The supplied [`RootId`] is authoritative for this preview. `NoInstallRootChosen` is the
/// chooser's no-selection state and is owned by Task 19, so this function cannot return it.
/// A storage fault is conservatively refused as `root_unavailable`; [`super::handle_preview`]
/// uses the checked form above and reports the same fault as `INTERNAL` instead.
///
/// # Errors
/// The first refusal that applies, in the checked form's order: `RootUnavailable`, `NoCloneUrl`,
/// `UnsafeName`, `PrivateNeedsUpgrade`, then `DestinationExists` or `AlreadyInstalled` — and
/// `RootUnavailable` for a storage fault.
pub fn compose_destination(
    tx: &Transaction<'_>,
    project: ProjectId,
    root: RootId,
) -> Result<InstallDestination, InstallRefusal> {
    compose_destination_checked(tx, project, root).unwrap_or(Err(InstallRefusal::RootUnavailable))
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use super::is_safe_path_segment;

    #[test]
    fn an_ordinary_basename_is_safe() {
        for name in [
            "widget",
            "my-tool",
            "a.b.c",
            "Widget2",
            "_private",
            "ünïcode",
        ] {
            assert!(is_safe_path_segment(name), "{name} must be a legal segment");
        }
    }

    #[test]
    fn a_relative_component_is_refused() {
        for name in ["", ".", ".."] {
            assert!(!is_safe_path_segment(name), "{name:?} must be refused");
        }
    }

    #[test]
    fn a_separator_or_a_windows_illegal_character_is_refused() {
        for name in [
            "a/b", "a\\b", "a:b", "a*b", "a?b", "a\"b", "a<b", "a>b", "a|b", "a\tb", "a\0b",
        ] {
            assert!(!is_safe_path_segment(name), "{name:?} must be refused");
        }
    }

    /// Case-varied, and with an extension, because all of these name the same device.
    #[test]
    fn a_reserved_device_name_is_refused_in_every_spelling() {
        for name in [
            "aux",
            "AUX",
            "Aux",
            "con",
            "CON",
            "prn",
            "nul",
            "NUL",
            "com1",
            "COM9",
            "lpt1",
            "LPT9",
            "aux.txt",
            "COM1.tar.gz",
            "nul.md",
        ] {
            assert!(
                !is_safe_path_segment(name),
                "{name:?} names a reserved device and must be refused on every target"
            );
        }
    }

    /// `com0` and `lpt0` are **not** reserved, and refusing them would be a transformation of a
    /// legal name — the thing this module exists not to do.
    #[test]
    fn a_name_that_merely_resembles_a_device_is_allowed() {
        for name in ["com0", "lpt0", "com10", "console", "auxiliary", "nullable"] {
            assert!(
                is_safe_path_segment(name),
                "{name:?} is not a reserved device and must be allowed"
            );
        }
    }

    /// The Windows filesystem strips these, so the directory created would not be the directory
    /// named — the same identity break as a `-2` suffix, arriving by a different route.
    #[test]
    fn a_trailing_space_or_dot_is_refused_because_the_filesystem_would_strip_it() {
        for name in ["widget ", "widget.", "widget..", "widget . "] {
            assert!(!is_safe_path_segment(name), "{name:?} must be refused");
        }
    }

    #[test]
    fn a_leading_dash_is_refused_because_argv_reads_it_as_an_option() {
        assert!(!is_safe_path_segment("-rf"));
        assert!(is_safe_path_segment("a-rf"));
    }
}
