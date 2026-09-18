//! §29.3's blob cache and §29.5's per-project row — J7's store.
//!
//! **`blob_scan` exists because A10 re-grained `blob_finding`.** `blob_finding` holds one row per
//! occurrence, so a blob with no markers writes no row at all, and the occurrence table alone
//! cannot tell *read and clean* from *never read*. That is render-unknown-as-zero at the cache
//! layer, and it would make every clean blob re-read for ever. The outcome gets its own row: the
//! same *"a row saying I looked"* shape `peek_cache` already ships, one level down.
//!
//! **Marker counts are derived from these rows and are stored nowhere beside them** (A10).
//!
//! Neither cache table carries a `project_id`: the key is a **content address**, so two projects
//! holding a byte-identical blob share one row, and a merge leaves both tables alone (§29.10).

use rusqlite::{Connection, Transaction};

use super::markers::{scan_blob, BlobOccurrence, Marker, J7_BINARY_SNIFF_BYTES, J7_BLOB_BYTE_CAP};
use crate::git::BlobRead;
use crate::index::IndexError;

/// §29.2's rules 4 and 5, plus the ordinary outcome. `blob_scan.outcome`'s CHECK mirrors
/// [`BlobOutcome::slug`] character for character (R26).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlobOutcome {
    /// Read and passed through the marker pass. Findings, if any, are its children.
    Scanned,
    /// Over `J7_BLOB_BYTE_CAP`. **Recorded, never silently dropped** — the size comes from
    /// `cat-file --batch`'s own header, so the verdict carries its evidence.
    TooLarge,
    /// A NUL byte inside the first `J7_BINARY_SNIFF_BYTES`.
    Binary,
}

impl BlobOutcome {
    /// Every outcome, so a test can walk the vocabulary without restating it.
    pub const ALL: [BlobOutcome; 3] = [
        BlobOutcome::Scanned,
        BlobOutcome::TooLarge,
        BlobOutcome::Binary,
    ];

    /// The stored form.
    #[must_use]
    pub fn slug(self) -> &'static str {
        match self {
            BlobOutcome::Scanned => "scanned",
            BlobOutcome::TooLarge => "too_large",
            BlobOutcome::Binary => "binary",
        }
    }

    /// `slug`'s inverse. `None` for an outcome a newer build wrote.
    #[must_use]
    pub fn from_slug(s: &str) -> Option<BlobOutcome> {
        match s {
            "scanned" => Some(BlobOutcome::Scanned),
            "too_large" => Some(BlobOutcome::TooLarge),
            "binary" => Some(BlobOutcome::Binary),
            _ => None,
        }
    }
}

/// One cached blob-read outcome. **The row is the answer to *was this looked at*** — the
/// occurrence rows answer a different question and cannot answer this one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CachedScan {
    /// What the read concluded.
    pub outcome: BlobOutcome,
    /// The size git reported. **Diagnostic, read by no surface** (R129/F9): `too_large` without
    /// the size is a verdict with no evidence, and a later reader must not give this column a
    /// meaning its writer never promised.
    pub size_bytes: i64,
}

/// Record that a blob was looked at, at this scanner version.
///
/// # Errors
/// Fails when SQLite refuses the write.
pub fn record_blob_scan(
    tx: &Transaction<'_>,
    blob_oid: &str,
    scanner_version: i64,
    outcome: BlobOutcome,
    size_bytes: u64,
    now: i64,
) -> Result<(), IndexError> {
    tx.execute(
        "INSERT INTO blob_scan (blob_oid, scanner_version, outcome, size_bytes, scanned_at)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(blob_oid, scanner_version) DO UPDATE SET
             outcome = excluded.outcome,
             size_bytes = excluded.size_bytes,
             scanned_at = excluded.scanned_at",
        rusqlite::params![
            blob_oid,
            scanner_version,
            outcome.slug(),
            i64::try_from(size_bytes).unwrap_or(i64::MAX),
            now
        ],
    )?;
    Ok(())
}

/// Write one blob's occurrences as children of its [`record_blob_scan`] row.
///
/// The parent row must already exist: the foreign key is what stops an occurrence set outliving
/// the record of the read that produced it.
///
/// # Errors
/// Fails when SQLite refuses the write.
pub fn record_findings(
    tx: &Transaction<'_>,
    blob_oid: &str,
    scanner_version: i64,
    occurrences: &[BlobOccurrence],
) -> Result<(), IndexError> {
    tx.execute(
        "DELETE FROM blob_finding WHERE blob_oid = ?1 AND scanner_version = ?2",
        rusqlite::params![blob_oid, scanner_version],
    )?;
    for occurrence in occurrences {
        tx.execute(
            "INSERT INTO blob_finding
               (blob_oid, scanner_version, ordinal_in_blob, marker, salient_sha256,
                salient_text_capped, line, \"column\")
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![
                blob_oid,
                scanner_version,
                occurrence.ordinal_in_blob,
                occurrence.marker.slug(),
                occurrence.salient_sha256,
                occurrence.salient_text_capped,
                occurrence.line,
                occurrence.column,
            ],
        )?;
    }
    Ok(())
}

/// Apply §29.2's rules 4 and 5 to one read and file both tables for it.
///
/// **The two non-outcomes are recorded, never dropped**: without a row, a blob too large to read
/// and a blob nobody has read yet are the same absence, and the first would be re-read for ever.
///
/// # Errors
/// Fails when SQLite refuses a write.
pub fn record_read(
    tx: &Transaction<'_>,
    read: &BlobRead,
    scanner_version: i64,
    now: i64,
) -> Result<BlobOutcome, IndexError> {
    let outcome = match read.bytes.as_deref() {
        // The body was dropped at the seam because the header said it was over the cap.
        None => BlobOutcome::TooLarge,
        Some(bytes) if bytes.len() as u64 > J7_BLOB_BYTE_CAP => BlobOutcome::TooLarge,
        Some(bytes) => {
            let sniff = bytes.get(..J7_BINARY_SNIFF_BYTES).unwrap_or(bytes);
            if sniff.contains(&0) {
                BlobOutcome::Binary
            } else {
                BlobOutcome::Scanned
            }
        }
    };
    record_blob_scan(
        tx,
        &read.oid,
        scanner_version,
        outcome,
        read.size_bytes,
        now,
    )?;
    if outcome == BlobOutcome::Scanned {
        let occurrences = scan_blob(read.bytes.as_deref().unwrap_or_default());
        record_findings(tx, &read.oid, scanner_version, &occurrences)?;
    }
    Ok(outcome)
}

/// The cached outcome for one blob **at this scanner version**, or `None` for a miss.
///
/// # Errors
/// Fails when SQLite refuses the read.
pub fn cached_scan(
    conn: &Connection,
    blob_oid: &str,
    scanner_version: i64,
) -> Result<Option<CachedScan>, IndexError> {
    let found = conn
        .query_row(
            "SELECT outcome, size_bytes FROM blob_scan
              WHERE blob_oid = ?1 AND scanner_version = ?2",
            rusqlite::params![blob_oid, scanner_version],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(IndexError::from(other)),
        })?;
    Ok(found.and_then(|(slug, size_bytes)| {
        BlobOutcome::from_slug(&slug).map(|outcome| CachedScan {
            outcome,
            size_bytes,
        })
    }))
}

/// The oids of `wanted` that have no row **at the current version**, in the order given.
///
/// **A row at an older version is a miss, not a hit** (§29.3). Older rows are not deleted
/// eagerly, so a rollback to the previous build finds its cache intact.
///
/// # Errors
/// Fails when SQLite refuses the read.
pub fn missing_blobs(
    conn: &Connection,
    wanted: &[String],
    scanner_version: i64,
) -> Result<Vec<String>, IndexError> {
    let mut missing = Vec::new();
    for oid in wanted {
        if cached_scan(conn, oid, scanner_version)?.is_none() {
            missing.push(oid.clone());
        }
    }
    Ok(missing)
}

/// §29.8's revocation: delete everything the grant produced.
///
/// **The row itself survives**, with `head_oid`, the four presence tri-states,
/// `presence_observed_at` and `predicate_version` intact — those were never under this grant, and
/// `head_oid` is `NOT NULL`, so clearing it would mean deleting the row and losing four answers
/// the user never revoked.
///
/// `blob_finding` goes with `blob_scan` through the foreign key, and is deleted explicitly
/// anyway: a row orphaned by a `PRAGMA foreign_keys = OFF` connection would survive a promise.
///
/// # Errors
/// Fails when SQLite refuses a write.
pub fn revoke_content_scan(tx: &Transaction<'_>) -> Result<(), IndexError> {
    tx.execute("DELETE FROM blob_finding", [])?;
    tx.execute("DELETE FROM blob_scan", [])?;
    tx.execute(
        "UPDATE project_content_scan
            SET complete_head_oid = NULL, blobs_total = NULL, blobs_pending = NULL,
                completed_at = NULL",
        [],
    )?;
    Ok(())
}

/// Every occurrence cached for one blob, ascending on `ordinal_in_blob`.
///
/// # Errors
/// Fails when SQLite refuses the read.
pub fn findings_for_blob(
    conn: &Connection,
    blob_oid: &str,
    scanner_version: i64,
) -> Result<Vec<BlobOccurrence>, IndexError> {
    let mut statement = conn.prepare(
        "SELECT ordinal_in_blob, marker, line, \"column\", salient_sha256, salient_text_capped
           FROM blob_finding
          WHERE blob_oid = ?1 AND scanner_version = ?2
          ORDER BY ordinal_in_blob",
    )?;
    let rows = statement.query_map(rusqlite::params![blob_oid, scanner_version], |row| {
        Ok((
            row.get::<_, u32>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, u32>(2)?,
            row.get::<_, u32>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, String>(5)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (ordinal_in_blob, marker, line, column, salient_sha256, salient_text_capped) = row?;
        // A marker slug this build cannot name was written by a newer one; skipping it is the
        // same rule `JobKind::from_slug` applies, and inventing a variant for it would be worse.
        let Some(marker) = Marker::from_slug(&marker) else {
            continue;
        };
        out.push(BlobOccurrence {
            ordinal_in_blob,
            marker,
            line,
            column,
            salient_sha256,
            salient_text_capped,
        });
    }
    Ok(out)
}
