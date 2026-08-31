//! Persisting what the probe found, and §4bis.2a's re-detection rewrite rule.
//!
//! Two rules are the whole point of this module:
//!
//! 1. **Re-detection rewrites `exec_bytes` for every row matching the re-detected
//!    `(kind, name)`, language and per-project rows included.** Without it the next versioned
//!    in-place update repairs the global default and strands the override rows pointing into a
//!    directory that no longer exists.
//! 2. **Detection writes the global scope only** — all three scope columns NULL. It never
//!    writes a language row: phase 1 ships no built-in language-to-editor table, so on a fresh
//!    install every drawer row reads the one evidence-ranked default.

use std::collections::BTreeSet;
use std::path::Path;

use crate::launch::catalogue::{CatalogueEntry, TargetKind};
use crate::launch::probe::{catalogued, TargetProbe};
use crate::launch::rank::{rank_editors, EditorEvidence, RankInputs, RankStep};
use crate::launch::recents::recent_paths;
use crate::launch::LaunchError;
use crate::proto::txguard::TxGuard;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DetectReport {
    pub inserted: u64,
    pub rewritten: u64,
    pub default_target_id: Option<i64>,
    pub ranked_by: Option<RankStep>,
}

pub fn known_location_keys(conn: &rusqlite::Connection) -> Result<BTreeSet<Vec<u8>>, LaunchError> {
    let mut stmt = conn.prepare("SELECT path_key FROM location")?;
    let mapped = stmt.query_map([], |row| row.get::<_, Vec<u8>>(0))?;
    let mut out = BTreeSet::new();
    for key in mapped {
        out.insert(key?);
    }
    Ok(out)
}

/// §4bis.2a: re-detection rewrites `exec_bytes` for **every** row matching `(kind, name)`.
///
/// The row goes back to `unverified` because the executable it names has changed, and §11.5's
/// window must not show a stale `ok` for a path nothing has looked at.
pub fn rewrite_exec_for(
    tx: &rusqlite::Transaction<'_>,
    kind: TargetKind,
    name: &str,
    exec: &[u8],
    now: i64,
) -> Result<usize, LaunchError> {
    let changed = tx.execute(
        "UPDATE launch_target
            SET exec_bytes = ?3, verify_state = 'unverified', verified_at = ?4
          WHERE kind = ?1 AND name = ?2 AND exec_bytes <> ?3",
        rusqlite::params![kind.as_str(), name, exec, now],
    )?;
    Ok(changed)
}

pub fn detect_and_store(
    conn: &mut rusqlite::Connection,
    probe: &dyn TargetProbe,
    config_root: &Path,
    user_home: &str,
    now: i64,
) -> Result<DetectReport, LaunchError> {
    let facts = probe.probe();
    let known = known_location_keys(conn)?;
    let pairs = catalogued(&facts.apps);

    let editors: Vec<EditorEvidence> = pairs
        .iter()
        .enumerate()
        .filter(|(_, (_, entry))| entry.kind == TargetKind::Editor)
        .map(|(index, (app, entry))| EditorEvidence {
            index,
            stem: app.stem.clone(),
            recent_paths: recent_paths(entry.recents, config_root, user_home),
            state_mtime: state_mtime(entry, config_root),
            installed_at: app.installed_at,
        })
        .collect();
    let ranked = rank_editors(&RankInputs {
        editors: &editors,
        known_locations: &known,
        editor_env_stem: facts.editor_env.as_deref(),
        folder_handler_stem: facts.folder_handler_stem.as_deref(),
    });

    let _guard = TxGuard::enter();
    let tx = conn.transaction()?;
    let mut inserted = 0_u64;
    let mut rewritten = 0_u64;
    let mut default_target_id = None;

    for (position, (app, entry)) in pairs.iter().enumerate() {
        let exec = crate::paths::path_bytes(&app.exec);
        rewritten += u64::try_from(rewrite_exec_for(&tx, entry.kind, entry.name, &exec, now)?)
            .unwrap_or_default();
        // The evidence winner heads its kind; everything else keeps probe order behind it.
        let sort_index = if ranked.is_some_and(|r| r.index == position) {
            0
        } else {
            i64::try_from(position)
                .unwrap_or(i64::MAX)
                .saturating_add(1)
        };
        let args = serde_json::to_string(entry.args).unwrap_or_else(|_| "[]".to_owned());
        let changed = tx.execute(
            "INSERT INTO launch_target
                 (project_id, location_id, language, kind, name, exec_bytes, args_json,
                  cwd_mode, env_json, sort_index, detected, verify_state)
             SELECT NULL, NULL, NULL, ?1, ?2, ?3, ?4, ?5, '{}', ?6, 1, 'unverified'
             WHERE NOT EXISTS (
                 SELECT 1 FROM launch_target
                  WHERE project_id IS NULL AND location_id IS NULL AND language IS NULL
                    AND kind = ?1 AND name = ?2)",
            rusqlite::params![
                entry.kind.as_str(),
                entry.name,
                exec,
                args,
                entry.cwd.as_str(),
                sort_index
            ],
        )?;
        inserted += u64::try_from(changed).unwrap_or_default();
        if ranked.is_some_and(|r| r.index == position) {
            default_target_id = tx
                .query_row(
                    "SELECT id FROM launch_target
                      WHERE project_id IS NULL AND location_id IS NULL AND language IS NULL
                        AND kind = ?1 AND name = ?2",
                    rusqlite::params![entry.kind.as_str(), entry.name],
                    |row| row.get::<_, i64>(0),
                )
                .ok();
        }
    }
    tx.commit()?;
    Ok(DetectReport {
        inserted,
        rewritten,
        default_target_id,
        ranked_by: ranked.map(|r| r.step),
    })
}

fn state_mtime(entry: &CatalogueEntry, config_root: &Path) -> Option<i64> {
    entry
        .config_dirs
        .iter()
        .filter_map(|rel| std::fs::metadata(config_root.join(rel)).ok())
        .filter_map(|m| m.modified().ok())
        .filter_map(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .filter_map(|d| i64::try_from(d.as_secs()).ok())
        .max()
}
