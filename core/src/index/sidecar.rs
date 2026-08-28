//! §1.12's JSON sidecar: the five-plus categories a rebuild cannot re-derive.
//!
//! Every record is keyed on a `ProjectSubject`, never an id, because a rebuild reassigns ids
//! and an id-keyed restore puts one project's notes on another. Written atomically, hourly and
//! on clean shutdown, with a generation number and a checksum.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use rusqlite::Connection;

use super::subject::subject_for_project;
use super::IndexError;
use crate::protocol::ProjectId;

pub const SIDECAR_FORMAT: u32 = 1;
pub const SIDECAR_INTERVAL_SECS: i64 = 3600;

/// `app_meta` keys the new database owns rather than restores.
const REBUILD_OWNED_SETTINGS: [&str; 3] = ["schema_version", "sidecar_generation", "git_version"];

macro_rules! record {
    ($name:ident { $($field:ident : $ty:ty),* $(,)? }) => {
        #[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
        pub struct $name { $(pub $field: $ty),* }
    };
}

record!(SidecarSegment {
    started_at: i64,
    ended_at: Option<i64>,
    credited_seconds: i64,
    closed_by: Option<String>,
});
record!(SidecarSession {
    started_at: i64,
    ended_at: Option<i64>,
    credited_seconds: i64,
    close_reason: Option<String>,
    segments: Vec<SidecarSegment>,
});
record!(SidecarXpEvent {
    ts: i64,
    tz_offset_min: Option<i64>,
    kind: String,
    dedupe_key: String,
    meta: Option<String>,
});
record!(SidecarLaunchTarget {
    kind: String,
    name: String,
    exec_hex: String,
    args_json: String,
    cwd_mode: String,
    env_json: String,
    sort_index: i64,
    language: Option<String>,
});
record!(SidecarProject {
    subject: String,
    notes: Option<String>,
    is_pinned: bool,
    is_archived: bool,
    is_hidden: bool,
    acknowledged_at: Option<i64>,
    seed_basename: String,
    reroll_offset: i64,
    sessions: Vec<SidecarSession>,
    xp_events: Vec<SidecarXpEvent>,
    launch_targets: Vec<SidecarLaunchTarget>,
});
record!(SidecarCollection {
    name: String,
    kind: String,
    query_text: Option<String>,
    query_grammar_version: Option<i64>,
    sort_index: i64,
    members: Vec<String>,
});
record!(SidecarRoot {
    kind: String,
    distro: String,
    path_hex: String,
    enabled: bool,
    added_by: String,
    descend_into_repos: bool,
    added_at: i64,
});
record!(SidecarAlias {
    email: String,
    reason: String,
});
record!(SidecarIdentity {
    email: String,
    name: Option<String>,
    is_user: bool,
    source: String,
    confirmed_at: Option<i64>,
    aliases: Vec<SidecarAlias>,
});
record!(SidecarMerge {
    survivor: String,
    absorbed: String,
    merged_at: i64,
    association_kind: String,
    evidence_json: String,
    absorbed_json: String,
});

#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SidecarPayload {
    pub projects: Vec<SidecarProject>,
    pub collections: Vec<SidecarCollection>,
    pub roots: Vec<SidecarRoot>,
    pub identities: Vec<SidecarIdentity>,
    pub merges: Vec<SidecarMerge>,
    pub settings: BTreeMap<String, String>,
    pub view_state: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Sidecar {
    pub format: u32,
    pub generation: u64,
    pub written_at: i64,
    pub checksum: String,
    pub payload: SidecarPayload,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub struct SidecarCounts {
    pub projects: u64,
    pub notes: u64,
    pub sessions: u64,
    pub session_segments: u64,
    pub xp_events: u64,
    pub launch_targets: u64,
    pub collections: u64,
    pub collection_members: u64,
    pub roots: u64,
    pub identities: u64,
    pub merges: u64,
    pub settings: u64,
    pub view_state: u64,
}

#[derive(Debug, Clone)]
pub struct SidecarWriteReport {
    pub path: PathBuf,
    pub generation: u64,
    pub written_at: i64,
    pub counts: SidecarCounts,
}

#[must_use]
pub fn is_due(last_written_at: Option<i64>, now: i64) -> bool {
    match last_written_at {
        None => true,
        Some(last) => now.saturating_sub(last) >= SIDECAR_INTERVAL_SECS,
    }
}

fn len_u64<T>(v: &[T]) -> u64 {
    u64::try_from(v.len()).unwrap_or(u64::MAX)
}

#[must_use]
pub fn counts(s: &Sidecar) -> SidecarCounts {
    let p = &s.payload;
    let mut c = SidecarCounts {
        projects: len_u64(&p.projects),
        collections: len_u64(&p.collections),
        roots: len_u64(&p.roots),
        identities: len_u64(&p.identities),
        merges: len_u64(&p.merges),
        settings: u64::try_from(p.settings.len()).unwrap_or(u64::MAX),
        view_state: u64::try_from(p.view_state.len()).unwrap_or(u64::MAX),
        ..SidecarCounts::default()
    };
    for project in &p.projects {
        if project.notes.is_some() {
            c.notes += 1;
        }
        c.sessions += len_u64(&project.sessions);
        for session in &project.sessions {
            c.session_segments += len_u64(&session.segments);
        }
        c.xp_events += len_u64(&project.xp_events);
        c.launch_targets += len_u64(&project.launch_targets);
    }
    for collection in &p.collections {
        c.collection_members += len_u64(&collection.members);
    }
    c
}

pub fn export(conn: &Connection, generation: u64, now: i64) -> Result<Sidecar, IndexError> {
    let payload = SidecarPayload {
        projects: export_projects(conn)?,
        collections: export_collections(conn)?,
        roots: export_roots(conn)?,
        identities: export_identities(conn)?,
        merges: export_merges(conn)?,
        settings: export_kv(conn, "app_meta", &REBUILD_OWNED_SETTINGS)?,
        view_state: export_kv(conn, "view_state", &[])?,
    };
    let checksum = checksum_of(&payload)?;
    Ok(Sidecar {
        format: SIDECAR_FORMAT,
        generation,
        written_at: now,
        checksum,
        payload,
    })
}

/// FNV-1a/64 over the serialised payload. Deterministic because the payload is structs and
/// `BTreeMap`s, never `HashMap`s.
fn checksum_of(payload: &SidecarPayload) -> Result<String, IndexError> {
    let bytes = serde_json::to_vec(payload).map_err(|e| IndexError::Sidecar(e.to_string()))?;
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in &bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    Ok(format!("fnv1a64:{h:016x}"))
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

fn export_kv(
    conn: &Connection,
    table: &str,
    skip: &[&str],
) -> Result<BTreeMap<String, String>, IndexError> {
    let mut out = BTreeMap::new();
    let mut stmt = conn.prepare(&format!("SELECT k, v FROM {table} ORDER BY k"))?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let k: String = row.get(0)?;
        if skip.contains(&k.as_str()) {
            continue;
        }
        out.insert(k, row.get(1)?);
    }
    Ok(out)
}

fn export_projects(conn: &Connection) -> Result<Vec<SidecarProject>, IndexError> {
    let mut out = Vec::new();
    let mut stmt = conn.prepare(
        "SELECT id, notes, is_pinned, is_archived, is_hidden, acknowledged_at,
                seed_basename, reroll_offset
         FROM project WHERE merged_into IS NULL ORDER BY id",
    )?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let id = ProjectId(row.get(0)?);
        let Some(subject) = subject_for_project(conn, id)? else {
            // No lineage and no location: nothing outlives the rebuild to key on.
            continue;
        };
        out.push(SidecarProject {
            subject: subject.to_key(),
            notes: row.get(1)?,
            is_pinned: row.get::<_, i64>(2)? != 0,
            is_archived: row.get::<_, i64>(3)? != 0,
            is_hidden: row.get::<_, i64>(4)? != 0,
            acknowledged_at: row.get(5)?,
            seed_basename: row.get(6)?,
            reroll_offset: row.get(7)?,
            sessions: export_sessions(conn, id)?,
            xp_events: export_xp_events(conn, id)?,
            launch_targets: export_targets(conn, id)?,
        });
    }
    Ok(out)
}

fn export_sessions(conn: &Connection, id: ProjectId) -> Result<Vec<SidecarSession>, IndexError> {
    let mut sessions = Vec::new();
    let mut stmt = conn.prepare(
        "SELECT id, started_at, ended_at, credited_seconds, close_reason
         FROM session WHERE project_id = ?1 ORDER BY started_at, id",
    )?;
    let mut rows = stmt.query([id.0])?;
    while let Some(row) = rows.next()? {
        let session_id: i64 = row.get(0)?;
        let mut seg_stmt = conn.prepare(
            "SELECT started_at, ended_at, credited_seconds, closed_by
             FROM session_segment WHERE session_id = ?1 ORDER BY started_at, id",
        )?;
        let mut seg_rows = seg_stmt.query([session_id])?;
        let mut segments = Vec::new();
        while let Some(s) = seg_rows.next()? {
            segments.push(SidecarSegment {
                started_at: s.get(0)?,
                ended_at: s.get(1)?,
                credited_seconds: s.get(2)?,
                closed_by: s.get(3)?,
            });
        }
        sessions.push(SidecarSession {
            started_at: row.get(1)?,
            ended_at: row.get(2)?,
            credited_seconds: row.get(3)?,
            close_reason: row.get(4)?,
            segments,
        });
    }
    Ok(sessions)
}

fn export_xp_events(conn: &Connection, id: ProjectId) -> Result<Vec<SidecarXpEvent>, IndexError> {
    // The session track only: §1.7 makes the git track a pure function of history, which a
    // rebuild re-derives.
    let mut stmt = conn.prepare(
        "SELECT ts, tz_offset_min, kind, dedupe_key, meta
         FROM xp_events WHERE project_id = ?1 AND track = 'session' ORDER BY ts, id",
    )?;
    let mut rows = stmt.query([id.0])?;
    let mut out = Vec::new();
    while let Some(r) = rows.next()? {
        out.push(SidecarXpEvent {
            ts: r.get(0)?,
            tz_offset_min: r.get(1)?,
            kind: r.get(2)?,
            dedupe_key: r.get(3)?,
            meta: r.get(4)?,
        });
    }
    Ok(out)
}

fn export_targets(
    conn: &Connection,
    id: ProjectId,
) -> Result<Vec<SidecarLaunchTarget>, IndexError> {
    // Custom rows only: a detected row is re-detected, and §4bis rewrites its exec_bytes anyway.
    let mut stmt = conn.prepare(
        "SELECT kind, name, exec_bytes, args_json, cwd_mode, env_json, sort_index, language
         FROM launch_target WHERE project_id = ?1 AND detected = 0 ORDER BY sort_index, id",
    )?;
    let mut rows = stmt.query([id.0])?;
    let mut out = Vec::new();
    while let Some(r) = rows.next()? {
        let exec: Vec<u8> = r.get(2)?;
        out.push(SidecarLaunchTarget {
            kind: r.get(0)?,
            name: r.get(1)?,
            exec_hex: hex(&exec),
            args_json: r.get(3)?,
            cwd_mode: r.get(4)?,
            env_json: r.get(5)?,
            sort_index: r.get(6)?,
            language: r.get(7)?,
        });
    }
    Ok(out)
}

fn export_collections(conn: &Connection) -> Result<Vec<SidecarCollection>, IndexError> {
    let mut stmt = conn.prepare(
        "SELECT id, name, kind, query_text, query_grammar_version, sort_index
         FROM collection ORDER BY sort_index, id",
    )?;
    let mut rows = stmt.query([])?;
    let mut out = Vec::new();
    while let Some(r) = rows.next()? {
        let cid: i64 = r.get(0)?;
        let mut m_stmt = conn.prepare(
            "SELECT project_id FROM collection_member WHERE collection_id = ?1 ORDER BY project_id",
        )?;
        let mut m_rows = m_stmt.query([cid])?;
        let mut members = Vec::new();
        while let Some(m) = m_rows.next()? {
            if let Some(s) = subject_for_project(conn, ProjectId(m.get(0)?))? {
                members.push(s.to_key());
            }
        }
        out.push(SidecarCollection {
            name: r.get(1)?,
            kind: r.get(2)?,
            query_text: r.get(3)?,
            query_grammar_version: r.get(4)?,
            sort_index: r.get(5)?,
            members,
        });
    }
    Ok(out)
}

fn export_roots(conn: &Connection) -> Result<Vec<SidecarRoot>, IndexError> {
    let mut stmt = conn.prepare(
        "SELECT kind, distro, path_bytes, enabled, added_by, descend_into_repos, added_at
         FROM scan_root ORDER BY id",
    )?;
    let mut rows = stmt.query([])?;
    let mut out = Vec::new();
    while let Some(r) = rows.next()? {
        let bytes: Vec<u8> = r.get(2)?;
        out.push(SidecarRoot {
            kind: r.get(0)?,
            distro: r.get(1)?,
            path_hex: hex(&bytes),
            enabled: r.get::<_, i64>(3)? != 0,
            added_by: r.get(4)?,
            descend_into_repos: r.get::<_, i64>(5)? != 0,
            added_at: r.get(6)?,
        });
    }
    Ok(out)
}

fn export_identities(conn: &Connection) -> Result<Vec<SidecarIdentity>, IndexError> {
    let mut stmt = conn.prepare(
        "SELECT id, email, name, is_user, source, confirmed_at FROM identity ORDER BY id",
    )?;
    let mut rows = stmt.query([])?;
    let mut out = Vec::new();
    while let Some(r) = rows.next()? {
        let iid: i64 = r.get(0)?;
        let mut a_stmt = conn.prepare(
            "SELECT email, reason FROM identity_alias WHERE identity_id = ?1 ORDER BY id",
        )?;
        let mut a_rows = a_stmt.query([iid])?;
        let mut aliases = Vec::new();
        while let Some(a) = a_rows.next()? {
            aliases.push(SidecarAlias {
                email: a.get(0)?,
                reason: a.get(1)?,
            });
        }
        out.push(SidecarIdentity {
            email: r.get(1)?,
            name: r.get(2)?,
            is_user: r.get::<_, i64>(3)? != 0,
            source: r.get(4)?,
            confirmed_at: r.get(5)?,
            aliases,
        });
    }
    Ok(out)
}

fn export_merges(conn: &Connection) -> Result<Vec<SidecarMerge>, IndexError> {
    let mut stmt = conn.prepare(
        "SELECT survivor_project_id, absorbed_project_id, merged_at, association_kind,
                evidence_json, absorbed_json
         FROM merge_record ORDER BY id",
    )?;
    let mut rows = stmt.query([])?;
    let mut out = Vec::new();
    while let Some(r) = rows.next()? {
        let survivor = subject_for_project(conn, ProjectId(r.get(0)?))?;
        let absorbed = subject_for_project(conn, ProjectId(r.get(1)?))?;
        let (Some(survivor), Some(absorbed)) = (survivor, absorbed) else {
            continue;
        };
        out.push(SidecarMerge {
            survivor: survivor.to_key(),
            absorbed: absorbed.to_key(),
            merged_at: r.get(2)?,
            association_kind: r.get(3)?,
            evidence_json: r.get(4)?,
            absorbed_json: r.get(5)?,
        });
    }
    Ok(out)
}

/// Temp file, `sync_all`, then rename — atomic on both targets. `sync_all` runs before the
/// rename so a power cut cannot leave a renamed-but-empty file.
pub fn write_atomically(sidecar: &Sidecar, path: &Path) -> Result<(), IndexError> {
    use std::io::Write as _;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut tmp = path.as_os_str().to_os_string();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);

    let bytes =
        serde_json::to_vec_pretty(sidecar).map_err(|e| IndexError::Sidecar(e.to_string()))?;
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(&bytes)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Read a sidecar, refusing an unknown format or a checksum that does not match the payload.
pub fn read(path: &Path) -> Result<Sidecar, IndexError> {
    let bytes = std::fs::read(path)?;
    let doc: Sidecar =
        serde_json::from_slice(&bytes).map_err(|e| IndexError::Sidecar(e.to_string()))?;
    if doc.format != SIDECAR_FORMAT {
        return Err(IndexError::Sidecar(format!(
            "sidecar format {} is not {SIDECAR_FORMAT}",
            doc.format
        )));
    }
    let expected = checksum_of(&doc.payload)?;
    if expected != doc.checksum {
        return Err(IndexError::Sidecar(
            "sidecar checksum does not match its payload".to_owned(),
        ));
    }
    Ok(doc)
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub struct RestoreCounts {
    pub roots: u64,
    pub identities: u64,
    pub aliases: u64,
    pub settings: u64,
    pub view_state: u64,
    pub collections: u64,
    pub collection_members: u64,
    pub projects: u64,
    pub notes: u64,
    pub sessions: u64,
    pub session_segments: u64,
    pub xp_events: u64,
    pub launch_targets: u64,
}

/// Rows affected, as a count. `Connection::execute` returns `usize`.
fn rows(n: usize) -> u64 {
    u64::try_from(n).unwrap_or(0)
}

fn unhex(s: &str) -> Result<Vec<u8>, IndexError> {
    let raw = s.as_bytes();
    if raw.len() % 2 != 0 {
        return Err(IndexError::Sidecar(format!("not hex: {s}")));
    }
    let mut out = Vec::with_capacity(raw.len() / 2);
    for pair in raw.chunks_exact(2) {
        let text = std::str::from_utf8(pair).map_err(|e| IndexError::Sidecar(e.to_string()))?;
        out.push(u8::from_str_radix(text, 16).map_err(|e| IndexError::Sidecar(e.to_string()))?);
    }
    Ok(out)
}

/// Everything that attaches to no project: roots, identities, settings, view state and
/// collection definitions. Safe to run twice — every insert is `ON CONFLICT DO NOTHING`
/// against the natural key the DDL already declares. Deleting first would discard rows the
/// user made after the rebuild.
pub fn restore_global(conn: &Connection, doc: &Sidecar) -> Result<RestoreCounts, IndexError> {
    let mut c = RestoreCounts::default();

    for root in &doc.payload.roots {
        let bytes = unhex(&root.path_hex)?;
        // path_key is recomputed rather than exported: it is a pure function of the bytes.
        let key = crate::index::path::StoredPath::from_bytes(
            bytes,
            if root.kind == "win" {
                crate::index::path::PathPlatform::Windows
            } else {
                crate::index::path::PathPlatform::Unix
            },
        );
        let (path_bytes, path_key, path_display) = key.as_params();
        c.roots += rows(conn.execute(
            "INSERT INTO scan_root
               (kind, distro, path_bytes, path_key, path_display, enabled, added_by,
                descend_into_repos, added_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT (kind, distro, path_key) DO NOTHING",
            rusqlite::params![
                root.kind,
                root.distro,
                path_bytes,
                path_key,
                path_display,
                i64::from(root.enabled),
                root.added_by,
                i64::from(root.descend_into_repos),
                root.added_at,
            ],
        )?);
    }

    for identity in &doc.payload.identities {
        c.identities += rows(conn.execute(
            "INSERT INTO identity (email, name, is_user, source, confirmed_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT (email) DO NOTHING",
            rusqlite::params![
                identity.email,
                identity.name,
                i64::from(identity.is_user),
                identity.source,
                identity.confirmed_at,
            ],
        )?);
        for alias in &identity.aliases {
            c.aliases += rows(conn.execute(
                "INSERT INTO identity_alias (identity_id, email, reason)
                 SELECT id, ?2, ?3 FROM identity WHERE email = ?1
                 ON CONFLICT (email) DO NOTHING",
                rusqlite::params![identity.email, alias.email, alias.reason],
            )?);
        }
    }

    for (k, v) in &doc.payload.settings {
        c.settings += rows(conn.execute(
            "INSERT INTO app_meta (k, v) VALUES (?1, ?2)
             ON CONFLICT(k) DO UPDATE SET v = excluded.v",
            rusqlite::params![k, v],
        )?);
    }
    for (k, v) in &doc.payload.view_state {
        c.view_state += rows(conn.execute(
            "INSERT INTO view_state (k, v) VALUES (?1, ?2)
             ON CONFLICT(k) DO UPDATE SET v = excluded.v",
            rusqlite::params![k, v],
        )?);
    }

    for collection in &doc.payload.collections {
        c.collections += rows(conn.execute(
            "INSERT INTO collection (name, kind, query_text, query_grammar_version, sort_index)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT (name) DO NOTHING",
            rusqlite::params![
                collection.name,
                collection.kind,
                collection.query_text,
                collection.query_grammar_version,
                collection.sort_index,
            ],
        )?);
    }
    Ok(c)
}

/// The project-scoped half, applied when the scan re-discovers a subject.
pub fn restore_for_subject(
    conn: &Connection,
    doc: &Sidecar,
    project: ProjectId,
) -> Result<RestoreCounts, IndexError> {
    let mut c = RestoreCounts::default();
    let Some(subject) = subject_for_project(conn, project)? else {
        return Ok(c);
    };
    let key = subject.to_key();
    let Some(p) = doc.payload.projects.iter().find(|p| p.subject == key) else {
        return Ok(c);
    };
    c.projects = 1;

    restore_project_row(conn, project, p)?;
    if p.notes.is_some() {
        c.notes = 1;
    }
    let (sessions, segments) = restore_sessions(conn, project, p)?;
    c.sessions = sessions;
    c.session_segments = segments;
    c.xp_events = restore_xp_events(conn, project, &key, p)?;
    c.launch_targets = restore_targets(conn, project, p)?;
    c.collection_members = restore_memberships(conn, project, doc, &key)?;
    Ok(c)
}

fn restore_project_row(
    conn: &Connection,
    project: ProjectId,
    p: &SidecarProject,
) -> Result<(), IndexError> {
    conn.execute(
        "UPDATE project
            SET notes = COALESCE(?2, notes),
                is_pinned = ?3, is_archived = ?4, is_hidden = ?5,
                acknowledged_at = COALESCE(?6, acknowledged_at),
                reroll_offset = ?7
          WHERE id = ?1",
        rusqlite::params![
            project.0,
            p.notes,
            i64::from(p.is_pinned),
            i64::from(p.is_archived),
            i64::from(p.is_hidden),
            p.acknowledged_at,
            p.reroll_offset,
        ],
    )?;
    Ok(())
}

/// Returns `(sessions, segments)`.
fn restore_sessions(
    conn: &Connection,
    project: ProjectId,
    p: &SidecarProject,
) -> Result<(u64, u64), IndexError> {
    let (mut sessions, mut segments) = (0, 0);
    for s in &p.sessions {
        conn.execute(
            "INSERT INTO session
               (project_id, started_at, ended_at, credited_seconds, close_reason)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![
                project.0,
                s.started_at,
                s.ended_at,
                s.credited_seconds,
                s.close_reason
            ],
        )?;
        sessions += 1;
        let session_id = conn.last_insert_rowid();
        for seg in &s.segments {
            conn.execute(
                "INSERT INTO session_segment
                   (session_id, started_at, ended_at, credited_seconds, closed_by)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![
                    session_id,
                    seg.started_at,
                    seg.ended_at,
                    seg.credited_seconds,
                    seg.closed_by
                ],
            )?;
            segments += 1;
        }
    }
    Ok((sessions, segments))
}

fn restore_xp_events(
    conn: &Connection,
    project: ProjectId,
    key: &str,
    p: &SidecarProject,
) -> Result<u64, IndexError> {
    let mut n = 0;
    for e in &p.xp_events {
        n += rows(conn.execute(
            "INSERT INTO xp_events
               (ts, tz_offset_min, project_id, subject_key, kind, dedupe_key, track, meta)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'session', ?7)
             ON CONFLICT (dedupe_key) DO NOTHING",
            rusqlite::params![
                e.ts,
                e.tz_offset_min,
                project.0,
                key,
                e.kind,
                e.dedupe_key,
                e.meta
            ],
        )?);
    }
    Ok(n)
}

fn restore_targets(
    conn: &Connection,
    project: ProjectId,
    p: &SidecarProject,
) -> Result<u64, IndexError> {
    let mut n = 0;
    for t in &p.launch_targets {
        let exec = unhex(&t.exec_hex)?;
        conn.execute(
            "INSERT INTO launch_target
               (project_id, language, kind, name, exec_bytes, args_json, cwd_mode, env_json,
                sort_index, detected)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 0)",
            rusqlite::params![
                project.0,
                t.language,
                t.kind,
                t.name,
                exec,
                t.args_json,
                t.cwd_mode,
                t.env_json,
                t.sort_index
            ],
        )?;
        n += 1;
    }
    Ok(n)
}

fn restore_memberships(
    conn: &Connection,
    project: ProjectId,
    doc: &Sidecar,
    key: &str,
) -> Result<u64, IndexError> {
    let mut n = 0;
    for collection in &doc.payload.collections {
        if collection.members.iter().any(|m| m == key) {
            n += rows(conn.execute(
                "INSERT INTO collection_member (collection_id, project_id)
                 SELECT id, ?2 FROM collection WHERE name = ?1 AND kind = 'manual'
                 ON CONFLICT (collection_id, project_id) DO NOTHING",
                rusqlite::params![collection.name, project.0],
            )?);
        }
    }
    Ok(n)
}
