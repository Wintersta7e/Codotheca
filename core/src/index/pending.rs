//! §48.8.4's pending table: what a rebuild restored for a subject no project holds yet, staged
//! until the hand-off that brings the subject back applies it.

use rusqlite::Transaction;

use super::sidecar::{Scope, SectionRow, Sidecar, SidecarProject, SECTIONS};
use super::IndexError;

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
