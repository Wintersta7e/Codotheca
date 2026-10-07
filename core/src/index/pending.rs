//! §48.8.4's pending table: what a rebuild restored for a subject no project holds yet, staged
//! until the hand-off that brings the subject back applies it.

use std::collections::BTreeMap;

use rusqlite::{Connection, OptionalExtension as _, Transaction};

use super::sidecar::{
    insert_row, location_key_of_row, rows, unhex, RestoreCtx, RestoreOutcome, Scope, SectionRow,
    Sidecar, SidecarLocationKey, SidecarProject, SidecarRow, SidecarValue, SECTIONS,
};
use super::subject::{subject_for_project, ProjectSubject};
use super::IndexError;
use crate::protocol::{LocationId, ProjectId};

/// One staged record, as its `record` column carries it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PendingRecord {
    /// A project's per-project record.
    Project {
        /// The record as the document carried it.
        record: SidecarProject,
        /// The names of the collections the project was a member of: the matcher has no
        /// document to read them from.
        member_of: Vec<String>,
    },
    /// One row of a `Subject` section.
    Section {
        /// The section's registered name.
        name: String,
        /// The row as the document carried it.
        row: SectionRow,
    },
}

/// Stage every per-subject record a rebuild restores from `doc`, in the caller's transaction.
///
/// The document's own pending rows go in verbatim — a second corruption before every subject
/// returned loses none of the first's — then one [`PendingRecord::Project`] per project record
/// and one [`PendingRecord::Section`] per `Subject` section row, each stamped with
/// `doc.generation` and queued at `now`. Answers how many rows it staged.
///
/// # Errors
/// Fails when the generation does not fit a column, a record does not serialise, a `Subject`
/// section row names no subject, or SQLite refuses an insert.
pub fn stage_pending(tx: &Transaction<'_>, doc: &Sidecar, now: i64) -> Result<u64, IndexError> {
    let generation = i64::try_from(doc.generation)
        .map_err(|e| IndexError::Sidecar(format!("generation {}: {e}", doc.generation)))?;
    let mut insert = tx.prepare(
        "INSERT INTO sidecar_pending (source_generation, subject_key, location_keys, record,
                                      queued_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
    )?;
    let mut staged = 0_u64;
    for row in &doc.payload.pending {
        insert.execute(rusqlite::params![
            row.source_generation,
            row.subject_key,
            row.location_keys,
            row.record,
            row.queued_at,
        ])?;
        staged += 1;
    }
    for project in &doc.payload.projects {
        let member_of = doc
            .payload
            .collections
            .iter()
            .filter(|c| c.members.contains(&project.subject))
            .map(|c| c.name.clone())
            .collect();
        let record = PendingRecord::Project {
            record: project.clone(),
            member_of,
        };
        insert.execute(rusqlite::params![
            generation,
            project.subject,
            to_json(&project.location_keys)?,
            to_json(&record)?,
            now,
        ])?;
        staged += 1;
    }
    for section in SECTIONS.iter().filter(|s| s.scope == Scope::Subject) {
        for row in doc.payload.sections.get(section.name).into_iter().flatten() {
            let subject = row.subject.as_ref().ok_or_else(|| {
                IndexError::Sidecar(format!("a {} row names no subject", section.name))
            })?;
            let record = PendingRecord::Section {
                name: section.name.to_owned(),
                row: row.clone(),
            };
            insert.execute(rusqlite::params![
                generation,
                subject,
                to_json(&row.location_keys)?,
                to_json(&record)?,
                now,
            ])?;
            staged += 1;
        }
    }
    Ok(staged)
}

fn to_json<T: serde::Serialize>(value: &T) -> Result<String, IndexError> {
    serde_json::to_string(value).map_err(|e| IndexError::Sidecar(e.to_string()))
}

/// What one [`match_pending`] call wrote and what it left.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MatchReport {
    /// Rows written, by kind — `projects`, `notes`, `locations`, `sessions`,
    /// `session_segments`, `xp_events`, `launch_targets`, `collection_members`, or a section's
    /// name. An insert skipped because its row already exists counts zero.
    pub applied: BTreeMap<String, u64>,
    /// Records left pending because several live projects hold the subject and the record's
    /// location keys name none of them, or more than one.
    pub left_ambiguous: u64,
}

impl MatchReport {
    fn add(&mut self, kind: &str, n: u64) {
        *self.applied.entry(kind.to_owned()).or_default() += n;
    }
}

/// The one live project holding `subject`, or `None` when none or several do — never a guess
/// between them (§48.8.4).
///
/// Live is `merged_into IS NULL`. A lineage subject is held by a project with that lineage and
/// remote, a path subject by a project with a copy at that path.
///
/// # Errors
/// Fails when SQLite refuses the read.
pub fn resolve_subject_unique(
    conn: &Connection,
    subject: &ProjectSubject,
) -> Result<Option<ProjectId>, IndexError> {
    Ok(match live_holders(conn, subject)?.as_slice() {
        [only] => Some(*only),
        _ => None,
    })
}

/// Every live project holding `subject`, lowest id first.
fn live_holders(conn: &Connection, subject: &ProjectSubject) -> Result<Vec<ProjectId>, IndexError> {
    let ids: Vec<i64> = match subject {
        ProjectSubject::Lineage {
            lineage_key,
            remote_key,
        } => conn
            .prepare(
                "SELECT id FROM project
                 WHERE lineage_key = ?1 AND remote_key IS ?2 AND merged_into IS NULL
                 ORDER BY id",
            )?
            .query_map(rusqlite::params![lineage_key, remote_key], |r| r.get(0))?
            .collect::<Result<_, _>>()?,
        ProjectSubject::Path {
            kind,
            distro,
            path_key,
        } => conn
            .prepare(
                "SELECT DISTINCT p.id FROM project p
                 JOIN location l ON l.project_id = p.id
                 WHERE l.kind = ?1 AND l.distro = ?2 AND l.path_key = ?3
                   AND p.merged_into IS NULL
                 ORDER BY p.id",
            )?
            .query_map(rusqlite::params![kind, distro, path_key], |r| r.get(0))?
            .collect::<Result<_, _>>()?,
    };
    Ok(ids.into_iter().map(ProjectId).collect())
}

/// Apply every record staged on `project`'s subject that is `project`'s, in the caller's
/// transaction, deleting each one it applied (§48.8.4).
///
/// Called wherever a write computes or changes a subject key. Records apply oldest generation
/// first. When one live project holds the subject, every record on it is that project's; when
/// several do, a record is the one project's whose copies its location keys name, and otherwise
/// stays pending — never applied to a guess. A record that does not decode, names a section this
/// build does not register, or whose section restore answers `Pending` stays too. The apply and
/// its `DELETE` share the transaction, so a record applies exactly once. Announces nothing.
///
/// # Errors
/// Fails when SQLite refuses a read or write, or a record's hex or generation is malformed.
pub fn match_pending(
    tx: &Transaction<'_>,
    project: ProjectId,
    now: i64,
) -> Result<MatchReport, IndexError> {
    let mut report = MatchReport::default();
    let Some(subject) = subject_for_project(tx, project)? else {
        return Ok(report);
    };
    let key = subject.to_key();
    let staged: Vec<(i64, i64, String, String)> = tx
        .prepare(
            "SELECT id, source_generation, location_keys, record FROM sidecar_pending
             WHERE subject_key = ?1 ORDER BY source_generation, id",
        )?
        .query_map([&key], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
        .collect::<Result<_, _>>()?;
    if staged.is_empty() {
        return Ok(report);
    }
    let unique = resolve_subject_unique(tx, &subject)?;
    let holders = match unique {
        Some(_) => Vec::new(),
        None => live_holders(tx, &subject)?,
    };
    for (id, generation, keys, record) in staged {
        let owner = match unique {
            Some(only) => Some(only),
            None => named_holder(tx, &holders, &keys)?,
        };
        if owner != Some(project) {
            if owner.is_none() && holders.len() > 1 {
                report.left_ambiguous += 1;
            }
            continue;
        }
        let Ok(record) = serde_json::from_str::<PendingRecord>(&record) else {
            continue;
        };
        let source_generation = u64::try_from(generation)
            .map_err(|e| IndexError::Sidecar(format!("generation {generation}: {e}")))?;
        let applied = match &record {
            PendingRecord::Project { record, member_of } => {
                apply_project(tx, project, &key, record, member_of, &mut report)?;
                true
            }
            PendingRecord::Section { name, row } => {
                apply_section(tx, project, name, row, now, source_generation, &mut report)?
            }
        };
        if applied {
            tx.execute("DELETE FROM sidecar_pending WHERE id = ?1", [id])?;
        }
    }
    Ok(report)
}

/// The one holder among `holders` with a copy at one of `keys`, or `None` when none or several
/// have one. `keys` is the JSON text the pending row's `location_keys` column holds.
fn named_holder(
    tx: &Transaction<'_>,
    holders: &[ProjectId],
    keys: &str,
) -> Result<Option<ProjectId>, IndexError> {
    let Ok(keys) = serde_json::from_str::<Vec<SidecarLocationKey>>(keys) else {
        return Ok(None);
    };
    let mut named: Vec<ProjectId> = Vec::new();
    for key in &keys {
        if let Some((_, holder)) = location_at(tx, key)? {
            if holders.contains(&holder) && !named.contains(&holder) {
                named.push(holder);
            }
        }
    }
    Ok(match named.as_slice() {
        [only] => Some(*only),
        _ => None,
    })
}

/// The location at `key`, with its project, if one exists.
fn location_at(
    tx: &Transaction<'_>,
    key: &SidecarLocationKey,
) -> Result<Option<(LocationId, ProjectId)>, IndexError> {
    Ok(tx
        .query_row(
            "SELECT id, project_id FROM location WHERE kind = ?1 AND distro = ?2 AND path_key = ?3",
            rusqlite::params![key.kind, key.distro, unhex(&key.path_key)?],
            |r| Ok((LocationId(r.get(0)?), ProjectId(r.get(1)?))),
        )
        .optional()?)
}

/// The location at `key` if it is one of `project`'s copies.
fn own_location(
    tx: &Transaction<'_>,
    project: ProjectId,
    key: &SidecarLocationKey,
) -> Result<Option<LocationId>, IndexError> {
    Ok(location_at(tx, key)?
        .filter(|(_, holder)| *holder == project)
        .map(|(id, _)| id))
}

/// A project record, dependents last: removed copies first, so each session names its copy.
fn apply_project(
    tx: &Transaction<'_>,
    project: ProjectId,
    key: &str,
    p: &SidecarProject,
    member_of: &[String],
    report: &mut MatchReport,
) -> Result<(), IndexError> {
    for row in &p.removed_locations {
        report.add("locations", recreate_location(tx, project, row)?);
    }
    apply_project_row(tx, project, p)?;
    report.add("projects", 1);
    report.add("notes", u64::from(p.notes.is_some()));
    for s in &p.sessions {
        let location = match &s.location_key {
            Some(location) => own_location(tx, project, location)?,
            None => None,
        };
        tx.execute(
            "INSERT INTO session
               (project_id, location_id, started_at, ended_at, credited_seconds, close_reason)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                project.0,
                location.map(|l| l.0),
                s.started_at,
                s.ended_at,
                s.credited_seconds,
                s.close_reason
            ],
        )?;
        report.add("sessions", 1);
        let session_id = tx.last_insert_rowid();
        for seg in &s.segments {
            tx.execute(
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
            report.add("session_segments", 1);
        }
    }
    for e in &p.xp_events {
        let written = tx.execute(
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
        )?;
        report.add("xp_events", rows(written));
    }
    for t in &p.launch_targets {
        tx.execute(
            "INSERT INTO launch_target
               (project_id, language, kind, name, exec_bytes, args_json, cwd_mode, env_json,
                sort_index, detected)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 0)",
            rusqlite::params![
                project.0,
                t.language,
                t.kind,
                t.name,
                unhex(&t.exec_hex)?,
                t.args_json,
                t.cwd_mode,
                t.env_json,
                t.sort_index
            ],
        )?;
        report.add("launch_targets", 1);
    }
    for name in member_of {
        let written = tx.execute(
            "INSERT INTO collection_member (collection_id, project_id)
             SELECT id, ?2 FROM collection WHERE name = ?1 AND kind = 'manual'
             ON CONFLICT (collection_id, project_id) DO NOTHING",
            rusqlite::params![name, project.0],
        )?;
        report.add("collection_members", rows(written));
    }
    Ok(())
}

/// The project row's own non-derivable columns: flags, note, `acknowledged_at`,
/// `reroll_offset` and `seed_basename`.
///
/// The one place a restored record writes `is_archived`; §44's Seal replaces that write with its
/// own writer here.
fn apply_project_row(
    tx: &Transaction<'_>,
    project: ProjectId,
    p: &SidecarProject,
) -> Result<(), IndexError> {
    tx.execute(
        "UPDATE project
            SET notes = COALESCE(?2, notes),
                is_pinned = ?3, is_archived = ?4, is_hidden = ?5,
                acknowledged_at = COALESCE(?6, acknowledged_at),
                reroll_offset = ?7,
                seed_basename = ?8
          WHERE id = ?1",
        rusqlite::params![
            project.0,
            p.notes,
            i64::from(p.is_pinned),
            i64::from(p.is_archived),
            i64::from(p.is_hidden),
            p.acknowledged_at,
            p.reroll_offset,
            p.seed_basename,
        ],
    )?;
    Ok(())
}

/// Re-create a copy this app removed, every column as exported and `project` as its owner,
/// unless a location already holds its key. Answers how many rows it wrote.
fn recreate_location(
    tx: &Transaction<'_>,
    project: ProjectId,
    row: &SidecarRow,
) -> Result<u64, IndexError> {
    if location_at(tx, &location_key_of_row(row)?)?.is_some() {
        return Ok(0);
    }
    let mut row = row.clone();
    row.insert("project_id".to_owned(), SidecarValue::Integer(project.0));
    insert_row(tx, "location", &row, &["id"])?;
    Ok(1)
}

/// One `Subject` section row, through its owner's restore. Answers whether it applied.
fn apply_section(
    tx: &Transaction<'_>,
    project: ProjectId,
    name: &str,
    row: &SectionRow,
    now: i64,
    source_generation: u64,
    report: &mut MatchReport,
) -> Result<bool, IndexError> {
    let Some(section) = SECTIONS
        .iter()
        .find(|s| s.name == name && s.scope == Scope::Subject)
    else {
        return Ok(false);
    };
    let mut locations = Vec::new();
    for key in &row.location_keys {
        if let Some(id) = own_location(tx, project, key)? {
            locations.push((key.clone(), id));
        }
    }
    let ctx = RestoreCtx {
        now,
        project: Some(project),
        locations: &locations,
        source_generation,
    };
    Ok(match (section.restore)(tx, row, &ctx)? {
        RestoreOutcome::Applied(n) => {
            report.add(section.name, n);
            true
        }
        RestoreOutcome::Pending => false,
    })
}
