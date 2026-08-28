//! The logical subject a sidecar record is keyed on (§1.12), and the same string
//! `xp_events.subject_key` holds (§1.7).
//!
//! A rebuild reassigns every project id, so an export keyed on `id` restores a user's notes
//! onto the wrong projects. Lineage plus canonical remote survives a rebuild because both are
//! recomputed from the repository; where there is no lineage at all the location's path is what
//! is left to recognise the project by (§1.1).

use rusqlite::OptionalExtension as _;

use super::IndexError;
use crate::protocol::ProjectId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectSubject {
    Lineage {
        lineage_key: String,
        remote_key: Option<String>,
    },
    Path {
        kind: String,
        distro: String,
        path_key: Vec<u8>,
    },
}

impl ProjectSubject {
    #[must_use]
    pub fn to_key(&self) -> String {
        match self {
            Self::Lineage {
                lineage_key,
                remote_key,
            } => format!(
                "lineage:{lineage_key}|remote:{}",
                remote_key.as_deref().unwrap_or("")
            ),
            Self::Path {
                kind,
                distro,
                path_key,
            } => format!("path:{kind}:{distro}:{}", to_hex(path_key)),
        }
    }

    #[must_use]
    pub fn parse(key: &str) -> Option<Self> {
        if let Some(rest) = key.strip_prefix("lineage:") {
            let (lineage_key, remote) = rest.split_once("|remote:")?;
            return Some(Self::Lineage {
                lineage_key: lineage_key.to_owned(),
                remote_key: if remote.is_empty() {
                    None
                } else {
                    Some(remote.to_owned())
                },
            });
        }
        let rest = key.strip_prefix("path:")?;
        let (kind, rest) = rest.split_once(':')?;
        let (distro, hex) = rest.split_once(':')?;
        Some(Self::Path {
            kind: kind.to_owned(),
            distro: distro.to_owned(),
            path_key: from_hex(hex)?,
        })
    }
}

/// Hex, because `path_key` is a BLOB of arbitrary bytes and the key is a JSON object key.
fn to_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

fn from_hex(hex: &str) -> Option<Vec<u8>> {
    if hex.len() % 2 != 0 {
        return None;
    }
    let raw = hex.as_bytes();
    let mut out = Vec::with_capacity(hex.len() / 2);
    for pair in raw.chunks_exact(2) {
        let s = std::str::from_utf8(pair).ok()?;
        out.push(u8::from_str_radix(s, 16).ok()?);
    }
    Some(out)
}

/// The subject a project would be exported under, or `None` if the project does not exist and
/// has no location either.
pub fn subject_for_project(
    conn: &rusqlite::Connection,
    project: ProjectId,
) -> Result<Option<ProjectSubject>, IndexError> {
    let row: Option<(Option<String>, Option<String>)> = conn
        .query_row(
            "SELECT lineage_key, remote_key FROM project WHERE id = ?1",
            [project.0],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some((lineage, remote)) = row else {
        return Ok(None);
    };
    if let Some(lineage_key) = lineage {
        return Ok(Some(ProjectSubject::Lineage {
            lineage_key,
            remote_key: remote,
        }));
    }
    // §1.1: no commits at all — identity is the location alone. Lowest id, so the choice is
    // deterministic and independent of walk order.
    let loc: Option<(String, String, Vec<u8>)> = conn
        .query_row(
            "SELECT kind, distro, path_key FROM location
             WHERE project_id = ?1 ORDER BY id LIMIT 1",
            [project.0],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    Ok(loc.map(|(kind, distro, path_key)| ProjectSubject::Path {
        kind,
        distro,
        path_key,
    }))
}

/// The project a subject names now, or `None` if nothing matches. Never a nearest match.
pub fn resolve_subject(
    conn: &rusqlite::Connection,
    subject: &ProjectSubject,
) -> Result<Option<ProjectId>, IndexError> {
    let id: Option<i64> = match subject {
        ProjectSubject::Lineage {
            lineage_key,
            remote_key,
        } => conn
            .query_row(
                "SELECT id FROM project
                 WHERE lineage_key = ?1 AND remote_key IS ?2 AND merged_into IS NULL
                 ORDER BY id LIMIT 1",
                rusqlite::params![lineage_key, remote_key],
                |r| r.get(0),
            )
            .optional()?,
        ProjectSubject::Path {
            kind,
            distro,
            path_key,
        } => conn
            .query_row(
                "SELECT p.id FROM project p
                 JOIN location l ON l.project_id = p.id
                 WHERE l.kind = ?1 AND l.distro = ?2 AND l.path_key = ?3
                   AND p.merged_into IS NULL
                 ORDER BY p.id LIMIT 1",
                rusqlite::params![kind, distro, path_key],
                |r| r.get(0),
            )
            .optional()?,
    };
    Ok(id.map(ProjectId))
}
