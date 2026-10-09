//! §46.14's two sidecar sections: every parcel with its ref set, and every removal record with
//! its log.
//!
//! Both belong to their project's subject and name the copy they hang off by its location key —
//! `(kind, distro, path_key)` — never by a `location_id`, which a rebuild reassigns. A parcel
//! travels with its refs and a record with its log, so each child restores with its parent; and
//! parcels restore before records, because a record may name a parcel.
//!
//! Ids are kept. A parcel's directory name ends in its id and a holding directory is named by its
//! record's id, so a row restored under a new id would name a directory that is not its own. A
//! restore therefore inserts the exported id and never overwrites a row that already holds it,
//! and the rebuild reserves every id a waiting row holds before anything new can take it.

use rusqlite::{params, Connection, OptionalExtension as _, Transaction};

use crate::index::sidecar::{hex, unhex, SidecarLocationKey};
use crate::index::IndexError;
use crate::protocol::ProjectId;

/// The section carrying every parcel with its ref set.
pub const PARCELS_SECTION: &str = "parcels";
/// The section carrying every removal record with its log.
pub const REMOVAL_RECORDS_SECTION: &str = "removal_records";

/// One `parcel_ref` row: a ref the sealed parcel holds, in one of the copy's repositories.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SidecarParcelRef {
    /// The repository's path below the copy's root as raw bytes, hex-encoded; empty for the
    /// top-level repository.
    pub repo_path_hex: String,
    /// The full ref name.
    pub ref_name: String,
    /// The object the ref named when the parcel was sealed.
    pub oid: String,
}

/// One `parcel` row with its ref set: every column but the project and location ids a rebuild
/// reassigns.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SidecarParcel {
    /// The parcel's id, kept: its directory name ends in it.
    pub id: i64,
    /// The key of the copy the parcel preserved.
    pub location: SidecarLocationKey,
    /// `preserving`, `sealed`, `abandoned` or `purged`.
    pub state: String,
    /// The folder the user chose for it, its path bytes hex-encoded.
    pub folder_hex: String,
    /// Its directory's name inside that folder; set once it is sealed.
    pub dir_name: Option<String>,
    /// The directory it was written in before sealing, its path bytes hex-encoded.
    pub staging_hex: String,
    /// The volume the folder is on, when it was known.
    pub volume_key: Option<String>,
    /// The kind of store the folder is on, when it was known.
    pub store_class: Option<String>,
    /// The lineage of the repository it preserves.
    pub lineage_key: String,
    /// The digest of the copy's state it was sealed over.
    pub state_digest: Option<String>,
    /// The SHA-256 of its manifest.
    pub manifest_sha256: Option<String>,
    /// The bytes it holds, once counted.
    pub total_bytes: Option<i64>,
    /// The git version that wrote it.
    pub git_version: Option<String>,
    /// The tar version that wrote it.
    pub tar_version: Option<String>,
    /// The nonce of the session that wrote it, hex-encoded.
    pub session_nonce_hex: String,
    /// When it was started, in Unix seconds.
    pub created_at: i64,
    /// When it was sealed, in Unix seconds.
    pub sealed_at: Option<i64>,
    /// When it was last checked, in Unix seconds.
    pub checked_at: Option<i64>,
    /// When it was last checked in full, in Unix seconds.
    pub full_checked_at: Option<i64>,
    /// The git version the full check ran with.
    pub full_checked_git: Option<String>,
    /// The last check's answer; `unchecked` until one ran.
    pub check_result: String,
    /// When it was purged, in Unix seconds.
    pub purged_at: Option<i64>,
    /// Its ref set, by repository path, then ref name.
    pub refs: Vec<SidecarParcelRef>,
}

/// One `removal_log` row: the copy's full commit log, captured before any byte moved.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SidecarRemovalLog {
    /// The layout its lines are in.
    pub format: i64,
    /// The log itself.
    pub log: String,
}

/// One `removal_record` row with its log: every column but the project and location ids a
/// rebuild reassigns.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SidecarRemovalRecord {
    /// The record's id, kept: its holding directory is named by it.
    pub id: i64,
    /// The key of the copy it removed.
    pub location: SidecarLocationKey,
    /// `uninstall` or `remove`.
    pub kind: String,
    /// How far the removal got: `journaled`, `disposing`, `done`, `abandoned`, `refused` or
    /// `interrupted`.
    pub state: String,
    /// The copy's path bytes, hex-encoded.
    pub path_hex: String,
    /// Where its bytes were to go: `trash` or `hard_delete`.
    pub planned: String,
    /// The directory its bytes were held in, its path bytes hex-encoded, once one was made.
    pub holding_hex: Option<String>,
    /// The lineage of the repository it removed.
    pub lineage_key: String,
    /// The digest of the copy's state when it was journaled.
    pub state_digest: String,
    /// What brings the copy back: `remote` or `parcel`.
    pub recovery: String,
    /// The remotes it was verified against, as JSON text.
    pub remotes_json: Option<String>,
    /// When the remotes were verified, in Unix seconds.
    pub remote_verified_at: Option<i64>,
    /// The parcel it recovers through; set exactly when `recovery` is `parcel`.
    pub parcel_id: Option<i64>,
    /// Where its bytes went: `trashed`, `hard_deleted` or `gone_unconfirmed`.
    pub disposal: Option<String>,
    /// The README's file name, when one was kept.
    pub readme_name: Option<String>,
    /// The README's text, when one was kept.
    pub readme_text: Option<String>,
    /// Whether the kept README was cut short: 1 when it was, 0 when not.
    pub readme_truncated: Option<i64>,
    /// The nonce of the session that journaled it, hex-encoded.
    pub session_nonce_hex: String,
    /// When it started, in Unix seconds.
    pub started_at: i64,
    /// When it ended, in Unix seconds.
    pub ended_at: Option<i64>,
    /// The copy's commit log, captured before any byte moved.
    pub log: Option<SidecarRemovalLog>,
}

fn key_of(kind: String, distro: String, path_key: &[u8]) -> SidecarLocationKey {
    SidecarLocationKey {
        kind,
        distro,
        path_key: hex(path_key),
    }
}

/// Every parcel of `project`, by id, each with its refs by repository path and ref name.
///
/// # Errors
/// [`IndexError::Sqlite`] when a read fails.
pub fn export_parcels(
    conn: &Connection,
    project: ProjectId,
) -> Result<Vec<SidecarParcel>, IndexError> {
    let mut parcels: Vec<SidecarParcel> = conn
        .prepare(
            "SELECT p.id, l.kind, l.distro, l.path_key, p.state, p.folder_bytes, p.dir_name,
                    p.staging_bytes, p.volume_key, p.store_class, p.lineage_key, p.state_digest,
                    p.manifest_sha256, p.total_bytes, p.git_version, p.tar_version,
                    p.session_nonce, p.created_at, p.sealed_at, p.checked_at, p.full_checked_at,
                    p.full_checked_git, p.check_result, p.purged_at
               FROM parcel p JOIN location l ON l.id = p.location_id
              WHERE p.project_id = ?1 ORDER BY p.id",
        )?
        .query_map([project.0], |r| {
            Ok(SidecarParcel {
                id: r.get(0)?,
                location: key_of(r.get(1)?, r.get(2)?, &r.get::<_, Vec<u8>>(3)?),
                state: r.get(4)?,
                folder_hex: hex(&r.get::<_, Vec<u8>>(5)?),
                dir_name: r.get(6)?,
                staging_hex: hex(&r.get::<_, Vec<u8>>(7)?),
                volume_key: r.get(8)?,
                store_class: r.get(9)?,
                lineage_key: r.get(10)?,
                state_digest: r.get(11)?,
                manifest_sha256: r.get(12)?,
                total_bytes: r.get(13)?,
                git_version: r.get(14)?,
                tar_version: r.get(15)?,
                session_nonce_hex: hex(&r.get::<_, Vec<u8>>(16)?),
                created_at: r.get(17)?,
                sealed_at: r.get(18)?,
                checked_at: r.get(19)?,
                full_checked_at: r.get(20)?,
                full_checked_git: r.get(21)?,
                check_result: r.get(22)?,
                purged_at: r.get(23)?,
                refs: Vec::new(),
            })
        })?
        .collect::<Result<_, _>>()?;
    let mut refs = conn.prepare(
        "SELECT repo_path, ref_name, oid FROM parcel_ref WHERE parcel_id = ?1
          ORDER BY repo_path, ref_name",
    )?;
    for parcel in &mut parcels {
        parcel.refs = refs
            .query_map([parcel.id], |r| {
                Ok(SidecarParcelRef {
                    repo_path_hex: hex(&r.get::<_, Vec<u8>>(0)?),
                    ref_name: r.get(1)?,
                    oid: r.get(2)?,
                })
            })?
            .collect::<Result<_, _>>()?;
    }
    Ok(parcels)
}

/// Every removal record of `project`, by id, each with its log.
///
/// # Errors
/// [`IndexError::Sqlite`] when a read fails.
pub fn export_removal_records(
    conn: &Connection,
    project: ProjectId,
) -> Result<Vec<SidecarRemovalRecord>, IndexError> {
    let mut records: Vec<SidecarRemovalRecord> = conn
        .prepare(
            "SELECT r.id, l.kind, l.distro, l.path_key, r.kind, r.state, r.path_bytes, r.planned,
                    r.holding_bytes, r.lineage_key, r.state_digest, r.recovery, r.remotes_json,
                    r.remote_verified_at, r.parcel_id, r.disposal, r.readme_name, r.readme_text,
                    r.readme_truncated, r.session_nonce, r.started_at, r.ended_at
               FROM removal_record r JOIN location l ON l.id = r.location_id
              WHERE r.project_id = ?1 ORDER BY r.id",
        )?
        .query_map([project.0], |r| {
            Ok(SidecarRemovalRecord {
                id: r.get(0)?,
                location: key_of(r.get(1)?, r.get(2)?, &r.get::<_, Vec<u8>>(3)?),
                kind: r.get(4)?,
                state: r.get(5)?,
                path_hex: hex(&r.get::<_, Vec<u8>>(6)?),
                planned: r.get(7)?,
                holding_hex: r.get::<_, Option<Vec<u8>>>(8)?.map(|b| hex(&b)),
                lineage_key: r.get(9)?,
                state_digest: r.get(10)?,
                recovery: r.get(11)?,
                remotes_json: r.get(12)?,
                remote_verified_at: r.get(13)?,
                parcel_id: r.get(14)?,
                disposal: r.get(15)?,
                readme_name: r.get(16)?,
                readme_text: r.get(17)?,
                readme_truncated: r.get(18)?,
                session_nonce_hex: hex(&r.get::<_, Vec<u8>>(19)?),
                started_at: r.get(20)?,
                ended_at: r.get(21)?,
                log: None,
            })
        })?
        .collect::<Result<_, _>>()?;
    let mut log = conn.prepare("SELECT format, log FROM removal_log WHERE removal_id = ?1")?;
    for record in &mut records {
        record.log = log
            .query_row([record.id], |r| {
                Ok(SidecarRemovalLog {
                    format: r.get(0)?,
                    log: r.get(1)?,
                })
            })
            .optional()?;
    }
    Ok(records)
}

/// The one copy of `project` at `key`; `None` when it has none there, or more than one.
fn own_copy(
    conn: &Connection,
    project: ProjectId,
    key: &SidecarLocationKey,
) -> Result<Option<i64>, IndexError> {
    let ids: Vec<i64> = conn
        .prepare(
            "SELECT id FROM location
              WHERE project_id = ?1 AND kind = ?2 AND distro = ?3 AND path_key = ?4",
        )?
        .query_map(
            params![project.0, key.kind, key.distro, unhex(&key.path_key)?],
            |r| r.get(0),
        )?
        .collect::<Result<_, _>>()?;
    Ok(match ids.as_slice() {
        [only] => Some(*only),
        _ => None,
    })
}

fn held(conn: &Connection, table: &str, id: i64) -> Result<bool, IndexError> {
    Ok(conn
        .query_row(
            &format!("SELECT 1 FROM {table} WHERE id = ?1"),
            [id],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

fn unhex_opt(text: Option<&str>) -> Result<Option<Vec<u8>>, IndexError> {
    text.map(unhex).transpose()
}

/// Write `record` back as `project`'s, under its own id, with its refs. Answers `true` once
/// written and `false` — having written nothing — when the record must stay pending.
///
/// It stays pending, never guessed at, when `project` has no copy at the record's key, or more
/// than one; and when a parcel already holds its id, which is left exactly as it is and named
/// on stderr. Writes no ledger row and announces nothing. SQLite raises the table's sequence to
/// an explicit id above it, so no row made later can take this one.
///
/// # Errors
/// [`IndexError::Sidecar`] when a hex field is malformed; [`IndexError::Sqlite`] when a read or
/// an insert fails.
pub fn restore_parcel(
    tx: &Transaction<'_>,
    project: ProjectId,
    record: &SidecarParcel,
) -> Result<bool, IndexError> {
    let Some(location) = own_copy(tx, project, &record.location)? else {
        return Ok(false);
    };
    if held(tx, "parcel", record.id)? {
        eprintln!(
            "sidecar: parcel {} is already in the index, so its record stays pending",
            record.id
        );
        return Ok(false);
    }
    tx.execute(
        "INSERT INTO parcel (id, project_id, location_id, state, folder_bytes, dir_name,
                             staging_bytes, volume_key, store_class, lineage_key, state_digest,
                             manifest_sha256, total_bytes, git_version, tar_version,
                             session_nonce, created_at, sealed_at, checked_at, full_checked_at,
                             full_checked_git, check_result, purged_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18,
                 ?19, ?20, ?21, ?22, ?23)",
        params![
            record.id,
            project.0,
            location,
            record.state,
            unhex(&record.folder_hex)?,
            record.dir_name,
            unhex(&record.staging_hex)?,
            record.volume_key,
            record.store_class,
            record.lineage_key,
            record.state_digest,
            record.manifest_sha256,
            record.total_bytes,
            record.git_version,
            record.tar_version,
            unhex(&record.session_nonce_hex)?,
            record.created_at,
            record.sealed_at,
            record.checked_at,
            record.full_checked_at,
            record.full_checked_git,
            record.check_result,
            record.purged_at,
        ],
    )?;
    for r in &record.refs {
        tx.execute(
            "INSERT INTO parcel_ref (parcel_id, repo_path, ref_name, oid) VALUES (?1, ?2, ?3, ?4)",
            params![record.id, unhex(&r.repo_path_hex)?, r.ref_name, r.oid],
        )?;
    }
    Ok(true)
}

/// The states a copy holds at most one record in (`removal_record_one_open`), as an SQL list.
const OPEN_STATES: &str = "('journaled','disposing','interrupted')";

/// Write `record` back as `project`'s, under its own id, with its log. Answers `true` once
/// written and `false` — having written nothing — when the record must stay pending.
///
/// It stays pending as [`restore_parcel`]'s does, while the parcel it names is not in the index
/// (the parcel's own record may not have matched yet), and while it is open and its copy already
/// holds another open record, which the index cannot take beside it.
///
/// # Errors
/// [`IndexError::Sidecar`] when a hex field is malformed; [`IndexError::Sqlite`] when a read or
/// an insert fails.
pub fn restore_removal_record(
    tx: &Transaction<'_>,
    project: ProjectId,
    record: &SidecarRemovalRecord,
) -> Result<bool, IndexError> {
    let Some(location) = own_copy(tx, project, &record.location)? else {
        return Ok(false);
    };
    if held(tx, "removal_record", record.id)? {
        eprintln!(
            "sidecar: removal record {} is already in the index, so its record stays pending",
            record.id
        );
        return Ok(false);
    }
    let open_beside = tx
        .query_row(
            &format!(
                "SELECT 1 FROM removal_record
                  WHERE location_id = ?1 AND state IN {OPEN_STATES} AND ?2 IN {OPEN_STATES}"
            ),
            params![location, record.state],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    if open_beside {
        eprintln!(
            "sidecar: removal record {}'s copy already holds an open record, so its record stays \
             pending",
            record.id
        );
        return Ok(false);
    }
    if let Some(parcel) = record.parcel_id {
        if !held(tx, "parcel", parcel)? {
            return Ok(false);
        }
    }
    tx.execute(
        "INSERT INTO removal_record (id, project_id, location_id, kind, state, path_bytes, planned,
                                     holding_bytes, lineage_key, state_digest, recovery,
                                     remotes_json, remote_verified_at, parcel_id, disposal,
                                     readme_name, readme_text, readme_truncated, session_nonce,
                                     started_at, ended_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18,
                 ?19, ?20, ?21)",
        params![
            record.id,
            project.0,
            location,
            record.kind,
            record.state,
            unhex(&record.path_hex)?,
            record.planned,
            unhex_opt(record.holding_hex.as_deref())?,
            record.lineage_key,
            record.state_digest,
            record.recovery,
            record.remotes_json,
            record.remote_verified_at,
            record.parcel_id,
            record.disposal,
            record.readme_name,
            record.readme_text,
            record.readme_truncated,
            unhex(&record.session_nonce_hex)?,
            record.started_at,
            record.ended_at,
        ],
    )?;
    if let Some(log) = &record.log {
        tx.execute(
            "INSERT INTO removal_log (removal_id, format, log) VALUES (?1, ?2, ?3)",
            params![record.id, log.format, log.log],
        )?;
    }
    Ok(true)
}

/// Raise the `parcel` and `removal_record` sequences to at least the highest id a waiting record
/// holds, so a row made before that record matches can never take its id. Never lowers either.
///
/// # Errors
/// [`IndexError::Sqlite`] when the sequence table refuses a write.
pub fn reserve_ids(
    tx: &Transaction<'_>,
    parcel_max: Option<i64>,
    removal_max: Option<i64>,
) -> Result<(), IndexError> {
    for (table, max) in [("parcel", parcel_max), ("removal_record", removal_max)] {
        let Some(max) = max else {
            continue;
        };
        // A table no row was ever written to has no sequence row yet.
        tx.execute(
            "INSERT INTO sqlite_sequence (name, seq) SELECT ?1, ?2
              WHERE NOT EXISTS (SELECT 1 FROM sqlite_sequence WHERE name = ?1)",
            params![table, max],
        )?;
        tx.execute(
            "UPDATE sqlite_sequence SET seq = ?2 WHERE name = ?1 AND seq < ?2",
            params![table, max],
        )?;
    }
    Ok(())
}
