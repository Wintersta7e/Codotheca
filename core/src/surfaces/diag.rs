//! §11.3a group 8's `EXPORT EVERYTHING`, and §11.4's anonymise-by-default rule.
//! One exporting path, not two: §2.5 already tags exported bytes `{"b64": …}`.
//!
//! **The rolling log is not embedded.** It is a free-text stream this module cannot anonymise
//! field by field, and a half-scrubbed log is worse than an absent one. The bundle names the
//! file so the user attaches it deliberately — the same consent shape as `SHOW REAL PATHS`.

use crate::index::path::DisplayPathTable;
use crate::index::{Index, IndexError};
use crate::proto::dispatch::{parse_args, CommandFailure}; // R15: one helper, plan 03's
use crate::protocol::{Bytes, DiagBundle, DiagBundleArgs};
use crate::surfaces::anonymise::{anonymise_path, VolumeShapes};
use crate::surfaces::{display_map, settings, SurfaceCtx};

pub const BUNDLE_FILE_PREFIX: &str = "codotheca-diagnostics-";

/// The name of the rolling log, which the shell owns and this bundle only points at.
const LOG_FILE_NAME: &str = "codotheca.log";

/// Rewrites every path the bundle carries, or lets them through when the user has asked to
/// see them. One instance per bundle, so a volume's pseudonym is stable across sections.
struct Paths {
    include_real: bool,
    shapes: VolumeShapes,
}

impl Paths {
    fn show(&mut self, display: &str, volume: Option<&str>) -> String {
        if self.include_real {
            display.to_owned()
        } else {
            anonymise_path(display, &self.shapes.shape(volume.unwrap_or_default()))
        }
    }
}

/// # Errors
/// Fails when the arguments do not parse, or when the bundle cannot be built or written.
pub fn handle(ctx: &SurfaceCtx<'_>, args: serde_json::Value) -> Result<DiagBundle, CommandFailure> {
    let args: DiagBundleArgs = parse_args(args)?;
    write_bundle(ctx.index, args.include_real_paths, ctx.now)
        .map_err(|e| CommandFailure::internal(e.to_string()))
}

/// Builds the document and leaves it beside the index, temp-and-rename so a torn write is
/// never observable.
///
/// # Errors
/// Fails when the index cannot be read or the file cannot be written.
pub fn write_bundle(
    index: &Index,
    include_real_paths: bool,
    now: i64,
) -> Result<DiagBundle, IndexError> {
    let doc = build(index.conn(), include_real_paths, now)?;
    let text = serde_json::to_vec_pretty(&doc).map_err(|e| IndexError::Sidecar(e.to_string()))?;
    let path = index
        .data_dir()
        .join(format!("{BUNDLE_FILE_PREFIX}{now}.json"));
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, &text)?;
    std::fs::rename(&tmp, &path)?;
    Ok(DiagBundle {
        path_bytes: Bytes(crate::paths::path_bytes(&path)),
        path_display: crate::paths::path_display(&path),
        size_bytes: i64::try_from(text.len()).unwrap_or(i64::MAX),
        anonymised: !include_real_paths,
    })
}

/// # Errors
/// Fails when the index cannot be read.
pub fn build(
    conn: &rusqlite::Connection,
    include_real_paths: bool,
    now: i64,
) -> Result<serde_json::Value, IndexError> {
    let mut paths = Paths {
        include_real: include_real_paths,
        shapes: VolumeShapes::new(),
    };

    Ok(serde_json::json!({
        "generatedAt": now,
        "anonymised": !include_real_paths,
        "schemaVersion": crate::index::migrate::schema_version(conn)?,
        "protocolVersion": crate::protocol::PROTOCOL_VERSION,
        "coreVersion": env!("CARGO_PKG_VERSION"),
        "gitVersion": meta(conn, "git_version")?,
        "settings": settings::read(conn)?,
        "roots": roots(conn, &mut paths)?,
        "projects": projects(conn)?,
        "locations": locations(conn, &mut paths)?,
        "sessions": sessions(conn)?,
        "notes": notes(conn)?,
        "scanProblems": scan_problems(conn, &mut paths)?,
        "jobStates": job_states(conn)?,
        // §11.4: a free-text stream cannot be anonymised field by field, so the
        // bundle names it and the user attaches it deliberately.
        "log": { "path": LOG_FILE_NAME, "text": serde_json::Value::Null },
    }))
}

fn locations(
    conn: &rusqlite::Connection,
    paths: &mut Paths,
) -> Result<Vec<serde_json::Value>, IndexError> {
    // §1.10: `path_display` never joins another query, so the row is read first and the
    // display strings are resolved for its ids afterwards.
    let mut plain = rows(
        conn,
        "SELECT id, project_id, kind, presence, volume_key,
                branch, is_dirty, untracked_count, ahead, behind,
                worktree_observed_at, refstate_observed_at, trusted_at, last_seen_at
         FROM location ORDER BY id",
        |r| {
            Ok(serde_json::json!({
                "id": r.get::<_, i64>(0)?,
                "projectId": r.get::<_, i64>(1)?,
                "kind": r.get::<_, String>(2)?,
                "presence": r.get::<_, String>(3)?,
                "volumeKey": r.get::<_, Option<String>>(4)?,
                "branch": r.get::<_, Option<String>>(5)?,
                "isDirty": r.get::<_, Option<bool>>(6)?,
                "untrackedCount": r.get::<_, Option<i64>>(7)?,
                "ahead": r.get::<_, Option<i64>>(8)?,
                "behind": r.get::<_, Option<i64>>(9)?,
                "worktreeObservedAt": r.get::<_, Option<i64>>(10)?,
                "refstateObservedAt": r.get::<_, Option<i64>>(11)?,
                "trustedAt": r.get::<_, Option<i64>>(12)?,
                "lastSeenAt": r.get::<_, Option<i64>>(13)?,
            }))
        },
    )?;

    let ids: Vec<i64> = plain.iter().filter_map(|v| v["id"].as_i64()).collect();
    let displays = display_map(conn, DisplayPathTable::Location, &ids)?;
    for row in &mut plain {
        let id = row["id"].as_i64().unwrap_or_default();
        let display = displays.get(&id).cloned().unwrap_or_default();
        // The real volume key is what gives one mount a stable pseudonym. It is read here and
        // dropped: the bundle carries the shape, never the identifier it was derived from.
        let volume = row["volumeKey"].as_str().map(str::to_owned);
        row["path"] = serde_json::Value::String(paths.show(&display, volume.as_deref()));
        if let Some(object) = row.as_object_mut() {
            object.remove("volumeKey");
        }
    }
    Ok(plain)
}

fn roots(
    conn: &rusqlite::Connection,
    paths: &mut Paths,
) -> Result<Vec<serde_json::Value>, IndexError> {
    let mut plain = rows(
        conn,
        "SELECT id, kind, enabled, added_by, descend_into_repos FROM scan_root ORDER BY id",
        |r| {
            Ok(serde_json::json!({
                "id": r.get::<_, i64>(0)?,
                "kind": r.get::<_, String>(1)?,
                "enabled": r.get::<_, bool>(2)?,
                "addedBy": r.get::<_, String>(3)?,
                "descendIntoRepos": r.get::<_, bool>(4)?,
            }))
        },
    )?;

    let ids: Vec<i64> = plain.iter().filter_map(|v| v["id"].as_i64()).collect();
    let displays = display_map(conn, DisplayPathTable::ScanRoot, &ids)?;
    for row in &mut plain {
        let id = row["id"].as_i64().unwrap_or_default();
        let display = displays.get(&id).cloned().unwrap_or_default();
        // A root is not on a location's volume, so it gets its own shape namespace.
        row["path"] = serde_json::Value::String(paths.show(&display, Some("root")));
    }
    Ok(plain)
}

fn projects(conn: &rusqlite::Connection) -> Result<Vec<serde_json::Value>, IndexError> {
    rows(
        conn,
        "SELECT id, name, primary_language, archetype, condition_signal,
                is_pinned, is_archived, is_hidden, is_reference, is_fork,
                is_shallow, ambiguous_lineage, error_kind, error_at,
                tracked_files, size_tracked_bytes, art_state
         FROM project ORDER BY id",
        |r| {
            Ok(serde_json::json!({
                "id": r.get::<_, i64>(0)?,
                "name": r.get::<_, String>(1)?,
                "primaryLanguage": r.get::<_, Option<String>>(2)?,
                "archetype": r.get::<_, Option<String>>(3)?,
                "conditionSignal": r.get::<_, Option<String>>(4)?,
                "isPinned": r.get::<_, bool>(5)?,
                "isArchived": r.get::<_, bool>(6)?,
                "isHidden": r.get::<_, bool>(7)?,
                "isReference": r.get::<_, bool>(8)?,
                "isFork": r.get::<_, bool>(9)?,
                "isShallow": r.get::<_, bool>(10)?,
                "ambiguousLineage": r.get::<_, bool>(11)?,
                "errorKind": r.get::<_, Option<String>>(12)?,
                "errorAt": r.get::<_, Option<i64>>(13)?,
                "trackedFiles": r.get::<_, Option<i64>>(14)?,
                "sizeTrackedBytes": r.get::<_, Option<i64>>(15)?,
                "artState": r.get::<_, String>(16)?,
            }))
        },
    )
}

fn sessions(conn: &rusqlite::Connection) -> Result<Vec<serde_json::Value>, IndexError> {
    rows(
        conn,
        "SELECT id, project_id, started_at, ended_at, credited_seconds, close_reason
         FROM session ORDER BY id",
        |r| {
            Ok(serde_json::json!({
                "id": r.get::<_, i64>(0)?,
                "projectId": r.get::<_, i64>(1)?,
                "startedAt": r.get::<_, i64>(2)?,
                "endedAt": r.get::<_, Option<i64>>(3)?,
                "creditedSeconds": r.get::<_, i64>(4)?,
                "closeReason": r.get::<_, Option<String>>(5)?,
            }))
        },
    )
}

fn notes(conn: &rusqlite::Connection) -> Result<Vec<serde_json::Value>, IndexError> {
    rows(
        conn,
        "SELECT id, notes FROM project WHERE notes IS NOT NULL ORDER BY id",
        |r| {
            Ok(serde_json::json!({
                "projectId": r.get::<_, i64>(0)?,
                "note": r.get::<_, String>(1)?,
            }))
        },
    )
}

fn scan_problems(
    conn: &rusqlite::Connection,
    paths: &mut Paths,
) -> Result<Vec<serde_json::Value>, IndexError> {
    let mut plain = rows(
        conn,
        "SELECT id, scan_run_id, kind, detail, count FROM scan_problem ORDER BY id",
        |r| {
            Ok(serde_json::json!({
                "id": r.get::<_, i64>(0)?,
                "scanRunId": r.get::<_, i64>(1)?,
                "kind": r.get::<_, String>(2)?,
                "detail": r.get::<_, Option<String>>(3)?,
                "count": r.get::<_, i64>(4)?,
            }))
        },
    )?;

    let ids: Vec<i64> = plain.iter().filter_map(|v| v["id"].as_i64()).collect();
    let displays = display_map(conn, DisplayPathTable::ScanProblem, &ids)?;
    for row in &mut plain {
        let id = row["id"].as_i64().unwrap_or_default();
        let display = displays.get(&id).cloned().unwrap_or_default();
        row["path"] = serde_json::Value::String(paths.show(&display, Some("problem")));
    }
    Ok(plain)
}

fn job_states(conn: &rusqlite::Connection) -> Result<Vec<serde_json::Value>, IndexError> {
    rows(
        conn,
        "SELECT project_id, job, state, fail_count, reason, at
         FROM project_job_state ORDER BY project_id, job",
        |r| {
            Ok(serde_json::json!({
                "projectId": r.get::<_, i64>(0)?,
                "job": r.get::<_, String>(1)?,
                "state": r.get::<_, String>(2)?,
                "failCount": r.get::<_, i64>(3)?,
                "reason": r.get::<_, Option<String>>(4)?,
                "at": r.get::<_, i64>(5)?,
            }))
        },
    )
}

fn meta(conn: &rusqlite::Connection, key: &str) -> Result<Option<String>, IndexError> {
    let mut stmt = conn.prepare("SELECT v FROM app_meta WHERE k = ?1")?;
    let mut q = stmt.query([key])?;
    Ok(match q.next()? {
        Some(row) => Some(row.get(0)?),
        None => None,
    })
}

fn rows<F>(
    conn: &rusqlite::Connection,
    sql: &str,
    mut f: F,
) -> Result<Vec<serde_json::Value>, IndexError>
where
    F: FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<serde_json::Value>,
{
    let mut stmt = conn.prepare(sql)?;
    let mut q = stmt.query([])?;
    let mut out = Vec::new();
    while let Some(row) = q.next()? {
        out.push(f(row)?);
    }
    Ok(out)
}
