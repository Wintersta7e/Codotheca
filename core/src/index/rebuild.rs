//! §48.7.1 step 4: rebuild a corrupt index beside it and swap it in only after the commit.
//!
//! The order: probe the database without touching it, build a fresh index **beside** it, restore
//! into that in one transaction, then set the corrupt files aside and swap the fresh one in.
//! Every act on the data directory is recorded as it happens, so a rebuild that fails anywhere
//! takes them all back and leaves the directory as it found it — REBUILD may simply be retried.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OpenFlags};

use super::migrate::{self, Migration};
use super::recovery::{self, sibling, QuarantinedFiles, Undo};
use super::sidecar::{self, RestoreCtx, RestoreOutcome, Scope, Sidecar, SidecarState, SECTIONS};
use super::{open_connection, pending, Index, IndexError};
use crate::protocol::ProjectId;

/// The rebuild report's file name inside the data directory, beside the index it describes.
pub const REBUILD_REPORT_FILE: &str = "rebuild-report.json";

/// What [`probe_open`] found, before any ordinary open could change a file.
#[derive(Debug)]
pub enum Probe {
    /// No database file: the ordinary open creates one.
    Missing,
    /// The database opens and reads its schema.
    Opens,
    /// SQLite reads the file as not a database, or as a corrupt one.
    Corrupt {
        /// SQLite's own words.
        detail: String,
    },
    /// Any other failure, reported as itself.
    Other(IndexError),
}

/// What a rebuild did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RebuildOutcome {
    /// The database opened, so nothing was rebuilt and nothing moved: a database that opens is
    /// never quarantined.
    Opened,
    /// The fresh index is in place and the corrupt files are set aside; the report is written.
    Rebuilt(RebuildReportFile),
}

/// Why a rebuild did not happen. Every variant leaves the data directory as it was found, except
/// a `Failed` whose undo could not finish, which names what stayed.
#[derive(Debug, thiserror::Error)]
pub enum RebuildError {
    /// The database failed to open for a reason other than corruption.
    #[error("the index did not open, and not because it is corrupt: {0}")]
    NotCorrupt(IndexError),
    /// A newer build wrote the sidecar, so nothing was restored and nothing moved.
    #[error("the sidecar was written by a newer build: {reason}")]
    SidecarNewer {
        /// The reader's reason.
        reason: String,
    },
    /// An act failed, and every act before it was undone unless `undone` says otherwise.
    #[error("the rebuild failed{}: {reason}", if *.undone { " and changed nothing" } else { "" })]
    Failed {
        /// What failed, and anything that could not be undone.
        reason: String,
        /// Whether every act before the failure was undone, so the data directory is as it was
        /// found. Only then does the line say the rebuild changed nothing.
        undone: bool,
    },
}

/// Each act of a rebuild, in order, as the step hook sees it after the act is done.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RebuildStep {
    /// The fresh index exists beside the corrupt one, migrated and stamped.
    Built,
    /// The restore transaction committed.
    Restored,
    /// The fresh index's journal is folded into its file and the connection closed.
    Checkpointed,
    /// `index.db` is set aside.
    QuarantinedDb,
    /// Its `-wal` and `-shm`, where they existed, are set aside.
    QuarantinedSiblings,
    /// The sidecar is copied into the quarantine set.
    CopiedSidecar,
    /// The fresh index is renamed into place.
    Swapped,
}

/// `rebuild-report.json`: what §48.7.1 step 5's notice states, until the user acknowledges it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RebuildReportFile {
    /// When the quarantine happened, in Unix seconds; every set-aside name carries it.
    pub quarantined_at: i64,
    /// Where each set-aside file is now.
    pub quarantine_files: Vec<String>,
    /// How many of each record came back now, by the sidecar's count keys.
    pub restored: BTreeMap<String, u64>,
    /// How many records wait in the pending table for their subject's hand-off.
    pub pending: u64,
    /// The sidecar's `written_at`: nothing made after it can come back. `None` with no sidecar.
    pub gap_started_at: Option<i64>,
}

/// Classify `db` as the ordinary open would find it, without changing it or any file beside it.
///
/// An ordinary open of a file SQLite finds is not a database unlinks its `-wal` and `-shm`. This
/// opens it read-only and `immutable`, which takes no lock and reads no journal, so nothing is
/// created, replayed or removed; an uncheckpointed commit it skips is recovered by the ordinary
/// open that follows `Opens`. The one answer that skip gets wrong is a page 1 torn mid-checkpoint
/// whose good copy is still in the journal, so a `Corrupt` beside a non-empty `-wal` is read
/// again, through an ordinary open of a copy made in the system temp directory.
#[must_use]
pub fn probe_open(db: &Path) -> Probe {
    probe_open_in(db, &std::env::temp_dir())
}

/// [`probe_open`], with the directory the second read copies into given.
///
/// The copy goes in a fresh directory, readable by this user alone, made inside `scratch` and
/// removed before this returns. `scratch` must lie outside the data directory, so the copy can
/// never be mistaken for the index and nothing in the data directory is created or unlinked.
#[must_use]
pub fn probe_open_in(db: &Path, scratch: &Path) -> Probe {
    match std::fs::metadata(db) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Probe::Missing,
        Err(e) => return Probe::Other(IndexError::Io(e)),
        Ok(_) => {}
    }
    let uri = match immutable_uri(db) {
        Ok(uri) => uri,
        Err(e) => return Probe::Other(IndexError::Io(e)),
    };
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY
        | OpenFlags::SQLITE_OPEN_URI
        | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let read = Connection::open_with_flags(uri, flags).and_then(|conn| read_schema(&conn));
    match read.map_err(recovery::classify) {
        Ok(()) => Probe::Opens,
        Err(IndexError::Corrupt { detail }) if has_journal(db) => {
            read_through_journal(db, scratch, detail)
        }
        Err(IndexError::Corrupt { detail }) => Probe::Corrupt { detail },
        Err(other) => Probe::Other(other),
    }
}

/// The read both probes make: the schema version, then the schema table.
fn read_schema(conn: &Connection) -> rusqlite::Result<()> {
    conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))?;
    conn.query_row("SELECT count(*) FROM sqlite_master", [], |r| {
        r.get::<_, i64>(0)
    })?;
    Ok(())
}

/// Whether the journal the immutable read skipped holds anything. A `-wal` that cannot be
/// examined counts as holding something, so the second read runs and answers for it.
fn has_journal(db: &Path) -> bool {
    match std::fs::metadata(sibling(db, "-wal")) {
        Ok(meta) => meta.len() > 0,
        Err(e) => e.kind() != std::io::ErrorKind::NotFound,
    }
}

/// The second read: copy `db` and whichever of its `-wal` and `-shm` exist into a fresh directory
/// in `scratch`, then open the copy the ordinary way, which applies the journal as the core's own
/// open does. The copy reads → `Opens`, and the ordinary open that follows recovers the original
/// the same way. The copy is corrupt too → the first read's `Corrupt` stands. Anything else,
/// failing to make the copy included, → `Other`: never `Corrupt` on a read that did not finish.
fn read_through_journal(db: &Path, scratch: &Path, detail: String) -> Probe {
    static COPIES: AtomicU64 = AtomicU64::new(0);
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let copy_dir = scratch.join(format!(
        "codotheca-probe-{}-{stamp}-{}",
        std::process::id(),
        COPIES.fetch_add(1, Ordering::Relaxed)
    ));
    if let Err(e) = create_private_dir(&copy_dir) {
        return Probe::Other(IndexError::Io(e));
    }
    let read = copy_into(db, &copy_dir)
        .map_err(IndexError::Io)
        .and_then(|copy| {
            let conn = open_connection(&copy)?;
            read_schema(&conn).map_err(recovery::classify)
        });
    remove_copy(&copy_dir);
    match read {
        Ok(()) => Probe::Opens,
        Err(IndexError::Corrupt { .. }) => Probe::Corrupt { detail },
        Err(other) => Probe::Other(other),
    }
}

/// Copies `db` and each of its `-wal` and `-shm` that exists into `copy_dir`, under the same
/// names, and answers the copy's path.
fn copy_into(db: &Path, copy_dir: &Path) -> std::io::Result<PathBuf> {
    let name = db
        .file_name()
        .ok_or_else(|| std::io::Error::other("the index path names no file"))?;
    let copy = copy_dir.join(name);
    for suffix in ["", "-wal", "-shm"] {
        match std::fs::copy(sibling(db, suffix), sibling(&copy, suffix)) {
            Err(e) if !suffix.is_empty() && e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
            Ok(_) => {}
        }
    }
    Ok(copy)
}

/// Removes the second read's copy: every file in its directory — the ones copied in and any
/// SQLite left beside them — then the directory. A copy that will not go stays, readable by this
/// user alone, in the system temp directory until the system clears it; the answer does not
/// change.
fn remove_copy(copy_dir: &Path) {
    if let Ok(entries) = std::fs::read_dir(copy_dir) {
        for copied in entries.flatten().map(|entry| entry.path()) {
            let _ = std::fs::remove_file(&copied);
        }
    }
    let _ = std::fs::remove_dir(copy_dir);
}

/// Makes `dir` readable by this user alone, failing if anything is already there, so a path
/// another user planted in the shared temp directory is never reused or followed.
#[cfg(unix)]
fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt as _;
    std::fs::DirBuilder::new().mode(0o700).create(dir)
}

/// Makes `dir`, failing if anything is already there. Windows' temp directory is the user's own,
/// so a plain directory in it is already private.
#[cfg(not(unix))]
fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir(dir)
}

/// `db` as an immutable, read-only `file:` URI, every byte outside the path alphabet escaped.
fn immutable_uri(db: &Path) -> std::io::Result<String> {
    use std::fmt::Write as _;
    let path = uri_path(&std::path::absolute(db)?);
    let mut uri = String::from("file://");
    if !path.starts_with(b"/") {
        uri.push('/');
    }
    for &byte in &path {
        if byte.is_ascii_alphanumeric() || b"/-._~:".contains(&byte) {
            uri.push(char::from(byte));
        } else {
            let _ = write!(uri, "%{byte:02X}");
        }
    }
    uri.push_str("?mode=ro&immutable=1");
    Ok(uri)
}

/// The path's exact bytes.
#[cfg(unix)]
fn uri_path(path: &Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt as _;
    path.as_os_str().as_bytes().to_vec()
}

/// The path with `/` separators and no verbatim prefix: SQLite reads a drive letter only in
/// `file:///C:/…`.
#[cfg(windows)]
fn uri_path(path: &Path) -> Vec<u8> {
    let text = path.to_string_lossy();
    text.strip_prefix(r"\\?\")
        .unwrap_or(&text)
        .replace('\\', "/")
        .into_bytes()
}

/// The rebuild with the shipped migrations and no hook: the entry the startup mode calls.
///
/// # Errors
/// As [`rebuild_in_place_with`].
pub fn rebuild_in_place(data_dir: &Path, now: i64) -> Result<RebuildOutcome, RebuildError> {
    rebuild_in_place_with(data_dir, now, migrate::MIGRATIONS, &|_| Ok(()))
}

/// The rebuild, with the migration set injectable and `step` run after each act, so a test can
/// fail between any two. The production entry passes a hook that never fails.
///
/// # Errors
/// [`RebuildError::NotCorrupt`] when the probe fails for any reason but corruption;
/// [`RebuildError::SidecarNewer`] when a newer build wrote the sidecar, before anything moves;
/// [`RebuildError::Failed`] when any later act or `step` fails, after undoing every act it can.
pub fn rebuild_in_place_with(
    data_dir: &Path,
    now: i64,
    migrations: &[Migration],
    step: &dyn Fn(RebuildStep) -> Result<(), IndexError>,
) -> Result<RebuildOutcome, RebuildError> {
    match probe_open(&Index::db_path(data_dir)) {
        Probe::Missing | Probe::Opens => return Ok(RebuildOutcome::Opened),
        Probe::Other(e) => return Err(RebuildError::NotCorrupt(e)),
        Probe::Corrupt { .. } => {}
    }
    let supported = migrations.last().map_or(0, |m| m.version);
    let doc = match sidecar::inspect(&Index::sidecar_path(data_dir), supported) {
        SidecarState::Newer { reason } => return Err(RebuildError::SidecarNewer { reason }),
        SidecarState::Present(doc) => Some(*doc),
        SidecarState::Absent | SidecarState::Unreadable { .. } => None,
    };
    let mut undo = Undo::default();
    rebuild_beside(data_dir, doc.as_ref(), now, migrations, step, &mut undo)
        .map(RebuildOutcome::Rebuilt)
        .map_err(|e| {
            let (reason, undone) = undo.after(&e);
            RebuildError::Failed { reason, undone }
        })
}

/// Steps 3 to 7, every act recorded in `undo`.
fn rebuild_beside(
    data_dir: &Path,
    doc: Option<&Sidecar>,
    now: i64,
    migrations: &[Migration],
    step: &dyn Fn(RebuildStep) -> Result<(), IndexError>,
    undo: &mut Undo,
) -> Result<RebuildReportFile, IndexError> {
    let db = Index::db_path(data_dir);
    let side = sibling(&db, &format!(".rebuild-{now}"));
    let side_files = [side.clone(), sibling(&side, "-wal"), sibling(&side, "-shm")];
    for file in &side_files {
        recovery::refuse_taken(file)?;
    }
    for file in side_files {
        undo.created(file);
    }

    let mut conn = open_connection(&side)?;
    migrate::apply_all(&mut conn, migrations)?;
    super::stamp_schema_mirror(&conn)?;
    step(RebuildStep::Built)?;

    let (restored, pending) = match doc {
        Some(doc) => restore(&mut conn, doc, now)?,
        None => (BTreeMap::new(), 0),
    };
    step(RebuildStep::Restored)?;

    checkpoint_and_close(conn, &side)?;
    step(RebuildStep::Checkpointed)?;

    let quarantined =
        recovery::quarantine_set(&db, &Index::sidecar_path(data_dir), now, step, undo)?;
    undo.rename(&side, &db)?;
    step(RebuildStep::Swapped)?;

    let report = RebuildReportFile {
        quarantined_at: now,
        quarantine_files: listed(&quarantined),
        restored,
        pending,
        gap_started_at: doc.map(|d| d.written_at),
    };
    write_report(data_dir, &report, undo)?;
    Ok(report)
}

/// The one restore transaction: the global half, every `Global` section, every `NoScan`
/// section's projects with their own records applied, the per-subject records staged for the
/// hand-off, and the generation the document continues from (§48.8.1).
///
/// An id a section preserves needs no sequence fix-up here: SQLite raises an `AUTOINCREMENT`
/// table's sequence to every id inserted explicitly, so the next new row is past the highest one
/// restored.
fn restore(
    conn: &mut Connection,
    doc: &Sidecar,
    now: i64,
) -> Result<(BTreeMap<String, u64>, u64), IndexError> {
    let tx_guard = crate::proto::txguard::TxGuard::enter();
    let tx = conn.transaction()?;
    let globals = sidecar::restore_global(&tx, doc)?;
    let mut restored: BTreeMap<String, u64> = [
        ("roots", globals.roots),
        ("identities", globals.identities),
        ("collections", globals.collections),
        ("settings", globals.settings),
        ("view_state", globals.view_state),
    ]
    .into_iter()
    .map(|(key, n)| (key.to_owned(), n))
    .collect();

    let ctx = RestoreCtx {
        now,
        project: None,
        locations: &[],
        source_generation: doc.generation,
    };
    let sections = [Scope::Global, Scope::NoScan]
        .into_iter()
        .flat_map(|scope| SECTIONS.iter().filter(move |s| s.scope == scope));
    for section in sections {
        let mut applied = 0;
        for row in doc.payload.sections.get(section.name).into_iter().flatten() {
            match (section.restore)(&tx, row, &ctx)? {
                RestoreOutcome::Applied(n) => applied += n,
                // No later scan matches a global or no-scan row, so dropping it would lose it
                // silently.
                RestoreOutcome::Pending => {
                    return Err(IndexError::Sidecar(format!(
                        "section {} left a row pending that nothing will match",
                        section.name
                    )))
                }
            }
        }
        restored.insert(section.name.to_owned(), applied);
    }

    pending::stage_pending(&tx, doc, now)?;
    // Every project in the fresh index is one a `NoScan` section just re-created. No scan will
    // hand it off, so its records apply here (§48.8.3).
    let recreated: Vec<i64> = tx
        .prepare("SELECT id FROM project ORDER BY id")?
        .query_map([], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    for project in recreated {
        let matched = pending::match_pending(&tx, ProjectId(project), now)?;
        for (kind, n) in matched.applied {
            *restored.entry(kind).or_default() += n;
        }
    }
    let waiting: i64 = tx.query_row("SELECT count(*) FROM sidecar_pending", [], |r| r.get(0))?;
    let pending = u64::try_from(waiting)
        .map_err(|e| IndexError::Sidecar(format!("pending count {waiting}: {e}")))?;
    tx.execute(
        "INSERT INTO app_meta (k, v) VALUES ('sidecar_generation', ?1)
         ON CONFLICT(k) DO UPDATE SET v = excluded.v",
        [doc.generation.to_string()],
    )?;
    tx.commit()?;
    drop(tx_guard);
    Ok((restored, pending))
}

/// Fold the side index's journal into its file and close it, so the swap moves one complete
/// file. A journal left non-empty fails the rebuild.
fn checkpoint_and_close(conn: Connection, side: &Path) -> Result<(), IndexError> {
    let busy: i64 = conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| r.get(0))?;
    conn.close().map_err(|(_, e)| e)?;
    let journal = std::fs::metadata(sibling(side, "-wal")).map_or(0, |m| m.len());
    if busy != 0 || journal != 0 {
        return Err(IndexError::Io(std::io::Error::other(format!(
            "the rebuilt index kept {journal} journal bytes after its checkpoint"
        ))));
    }
    Ok(())
}

fn listed(set: &QuarantinedFiles) -> Vec<String> {
    std::iter::once(&set.db)
        .chain(&set.wal)
        .chain(&set.shm)
        .chain(&set.sidecar_copy)
        .map(|p| p.display().to_string())
        .collect()
}

/// Temp and rename, so the shell never reads a half-written report.
fn write_report(
    data_dir: &Path,
    report: &RebuildReportFile,
    undo: &mut Undo,
) -> Result<(), IndexError> {
    let path = data_dir.join(REBUILD_REPORT_FILE);
    let tmp: PathBuf = sibling(&path, ".tmp");
    // No `refuse_taken`: a fixed name in the core's own data directory that only the core
    // writes, and refusing a crashed rebuild's leftover would fail every later rebuild until the
    // user deleted it by hand.
    undo.created(tmp.clone());
    let bytes = serde_json::to_vec_pretty(report).map_err(std::io::Error::other)?;
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    /// On Unix the probe's copy of the index goes in the temp directory every local user shares,
    /// so the directory holding it must shut every other user out, and a path already there —
    /// a directory or a link another user planted — is never reused or followed.
    #[cfg(unix)]
    #[test]
    fn the_copy_directory_is_this_users_alone() {
        use std::os::unix::fs::PermissionsExt as _;

        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("copy");
        super::create_private_dir(&dir).unwrap();
        let mode = std::fs::metadata(&dir).unwrap().permissions().mode();
        assert_eq!(mode & 0o077, 0, "the copy directory's mode is {mode:o}");

        let planted = root.path().join("planted");
        std::os::unix::fs::symlink(root.path(), &planted).unwrap();
        for existing in [&dir, &planted] {
            let refused = super::create_private_dir(existing).unwrap_err();
            assert_eq!(refused.kind(), std::io::ErrorKind::AlreadyExists);
        }
    }
}
