//! §4bis.2a's five tiers, first hit wins, a tier with no row skipped rather than guessed at.
//!
//! | # | Tier | Predicate |
//! |---|---|---|
//! | 1 | project override | `project_id = <id>` and `location_id IS NULL` |
//! | 2 | location override | `location_id = <id>` |
//! | 3 | language default | scope columns NULL and `language = <primary_language>`, exact |
//! | 4 | global default | `project_id`, `location_id` and `language` all NULL |
//! | 5 | ask | nothing resolved — `Ok(None)` |
//!
//! Three rules the tests hold:
//!
//! - **`primary_language` NULL resolves at tier 4 and never matches a language row.** NULL is
//!   *not computed*, not *any*. Coalescing it into the `language IS NULL` predicate, taking the
//!   first language row, or taking the most recently set one all open one language's editor for
//!   a repository nothing has looked at yet.
//! - **A project outranks its own copies.** Tier 1 is set by hand under the words `SET FOR THIS
//!   PROJECT ONLY`; if a copy's row overrode it that sentence would be false.
//! - **Resolution never consults `verify_state`.** A failing row still resolves and fails loudly
//!   at spawn; falling through would silently open an application the user did not choose. Rows
//!   with `disabled = 1` *are* skipped — a disabled row is not a row the user has.

use crate::launch::catalogue::{CwdMode, TargetKind};
use crate::launch::LaunchError;

/// R31: declared in `protocol/schema/protocol.json`, generated into `crate::protocol`.
/// Re-exported so `launch::resolve::TargetTier` still names it; `as_str` stays as an inherent
/// impl on the generated type, since the generated enum has no methods and this string is what
/// `resolve` reports.
pub use crate::protocol::TargetTier;

impl TargetTier {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Project => "project",
            Self::Location => "location",
            Self::Language => "language",
            Self::Global => "global",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredTarget {
    pub id: i64,
    pub kind: TargetKind,
    pub name: String,
    pub project_id: Option<i64>,
    pub location_id: Option<i64>,
    pub language: Option<String>,
    pub sort_index: i64,
    pub detected: bool,
    pub verify_state: String,
    pub verified_at: Option<i64>,
    pub exec_bytes: Vec<u8>,
    pub args: Vec<String>,
    pub cwd_mode: CwdMode,
    pub env: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolution {
    pub target: StoredTarget,
    pub tier: TargetTier,
}

const COLUMNS: &str = "id, kind, name, project_id, location_id, language, sort_index, detected,
                       verify_state, verified_at, exec_bytes, args_json, cwd_mode, env_json";

fn row_to_target(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredTarget> {
    let kind_text: String = row.get(1)?;
    let cwd_text: String = row.get(12)?;
    let args_json: String = row.get(11)?;
    let env_json: String = row.get(13)?;
    Ok(StoredTarget {
        id: row.get(0)?,
        kind: TargetKind::parse(&kind_text).unwrap_or(TargetKind::Editor),
        name: row.get(2)?,
        project_id: row.get(3)?,
        location_id: row.get(4)?,
        language: row.get(5)?,
        sort_index: row.get(6)?,
        detected: row.get::<_, i64>(7)? != 0,
        verify_state: row.get(8)?,
        verified_at: row.get(9)?,
        exec_bytes: row.get(10)?,
        args: serde_json::from_str(&args_json).unwrap_or_default(),
        cwd_mode: CwdMode::parse(&cwd_text).unwrap_or(CwdMode::Location),
        env: serde_json::from_str::<std::collections::BTreeMap<String, String>>(&env_json)
            .unwrap_or_default()
            .into_iter()
            .collect(),
    })
}

pub fn load_target(
    conn: &rusqlite::Connection,
    target_id: i64,
) -> Result<StoredTarget, LaunchError> {
    conn.query_row(
        &format!("SELECT {COLUMNS} FROM launch_target WHERE id = ?1 AND disabled = 0"),
        [target_id],
        row_to_target,
    )
    .map_err(|err| match err {
        rusqlite::Error::QueryReturnedNoRows => LaunchError::NoSuchTarget(target_id),
        other => LaunchError::Sqlite(other),
    })
}

/// Every row id, disabled ones included: §4bis.2a's verification is per row and unfiltered,
/// where resolution is filtered. A disabled row still has an executable that can go missing.
pub fn all_target_ids(conn: &rusqlite::Connection) -> Result<Vec<i64>, LaunchError> {
    let mut stmt = conn.prepare("SELECT id FROM launch_target ORDER BY id")?;
    let mapped = stmt.query_map([], |row| row.get::<_, i64>(0))?;
    let mut out = Vec::new();
    for id in mapped {
        out.push(id?);
    }
    Ok(out)
}

/// [`load_target`] without its `disabled = 0` clause.
pub fn load_any_target(
    conn: &rusqlite::Connection,
    target_id: i64,
) -> Result<StoredTarget, LaunchError> {
    conn.query_row(
        &format!("SELECT {COLUMNS} FROM launch_target WHERE id = ?1"),
        [target_id],
        row_to_target,
    )
    .map_err(|err| match err {
        rusqlite::Error::QueryReturnedNoRows => LaunchError::NoSuchTarget(target_id),
        other => LaunchError::Sqlite(other),
    })
}

fn head_of(
    conn: &rusqlite::Connection,
    predicate: &str,
    params: &[&dyn rusqlite::ToSql],
    kind: TargetKind,
) -> Result<Option<StoredTarget>, LaunchError> {
    let sql = format!(
        "SELECT {COLUMNS} FROM launch_target
          WHERE disabled = 0 AND kind = ?1 AND {predicate}
          ORDER BY sort_index, id LIMIT 1"
    );
    let kind_text = kind.as_str();
    let mut bound: Vec<&dyn rusqlite::ToSql> = vec![&kind_text];
    bound.extend_from_slice(params);
    let mut stmt = conn.prepare(&sql)?;
    let mut rows = stmt.query(rusqlite::params_from_iter(bound.iter().copied()))?;
    match rows.next()? {
        Some(row) => Ok(Some(row_to_target(row)?)),
        None => Ok(None),
    }
}

/// §4bis.2a, first hit wins. A tier with no row is skipped; nothing anywhere is `Ok(None)`,
/// which is tier 5 — ask.
pub fn resolve(
    conn: &rusqlite::Connection,
    project_id: i64,
    location_id: Option<i64>,
    primary_language: Option<&str>,
    kind: TargetKind,
) -> Result<Option<Resolution>, LaunchError> {
    if let Some(t) = head_of(
        conn,
        "project_id = ?2 AND location_id IS NULL",
        &[&project_id],
        kind,
    )? {
        return Ok(Some(Resolution {
            target: t,
            tier: TargetTier::Project,
        }));
    }
    if let Some(id) = location_id {
        if let Some(t) = head_of(conn, "location_id = ?2", &[&id], kind)? {
            return Ok(Some(Resolution {
                target: t,
                tier: TargetTier::Location,
            }));
        }
    }
    // Tier 3 runs only for a non-NULL language, and matches it exactly. NULL is
    // *not computed*, never *any*, and never falls into the `language IS NULL` predicate.
    if let Some(language) = primary_language {
        if let Some(t) = head_of(
            conn,
            "project_id IS NULL AND location_id IS NULL AND language = ?2",
            &[&language],
            kind,
        )? {
            return Ok(Some(Resolution {
                target: t,
                tier: TargetTier::Language,
            }));
        }
    }
    if let Some(t) = head_of(
        conn,
        "project_id IS NULL AND location_id IS NULL AND language IS NULL",
        &[],
        kind,
    )? {
        return Ok(Some(Resolution {
            target: t,
            tier: TargetTier::Global,
        }));
    }
    Ok(None)
}

/// Every row the `OPENS IN` menu may draw for this project: its own overrides, its copy's,
/// and the global rows. Ordered by kind then `sort_index`, disabled rows excluded.
pub fn menu_rows(
    conn: &rusqlite::Connection,
    project_id: Option<i64>,
    location_id: Option<i64>,
) -> Result<Vec<StoredTarget>, LaunchError> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM launch_target
          WHERE disabled = 0
            AND (project_id IS NULL OR project_id = ?1)
            AND (location_id IS NULL OR location_id = ?2)
          ORDER BY kind, sort_index, id"
    ))?;
    let mapped = stmt.query_map(rusqlite::params![project_id, location_id], row_to_target)?;
    let mut out = Vec::new();
    for target in mapped {
        out.push(target?);
    }
    Ok(out)
}

#[must_use]
pub fn exec_display(target: &StoredTarget) -> String {
    crate::paths::path_display(&crate::paths::path_from_bytes(&target.exec_bytes))
}
