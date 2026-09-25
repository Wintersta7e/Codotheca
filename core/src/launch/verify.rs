//! §4bis.5's `targets.verify`, per row and language-blind.
//!
//! Verification **walks every stored row — override, language and global alike — and each
//! carries its own `verified_at` / `verify_state`; a language row is not covered by having
//! verified the global row it was copied from.** §11.5 adds when it runs: at startup, and
//! before a spawn.

use std::path::Path;

use crate::launch::resolve::exec_display;
use crate::launch::LaunchError;
use crate::proto::txguard::TxGuard;

/// R31: declared in `protocol/schema/protocol.json`, generated into `crate::protocol`.
/// Re-exported so `launch::verify::VerifyState` still names it; `as_str` and `parse` stay as an
/// inherent impl on the generated type because the generated enum has no methods and the
/// `verify_state` column is text.
pub use crate::protocol::VerifyState;

impl VerifyState {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unverified => "unverified",
            Self::Ok => "ok",
            Self::Missing => "missing",
            Self::NotExecutable => "not_executable",
        }
    }
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "unverified" => Some(Self::Unverified),
            "ok" => Some(Self::Ok),
            "missing" => Some(Self::Missing),
            "not_executable" => Some(Self::NotExecutable),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verification {
    pub target_id: i64,
    pub verify_state: VerifyState,
    pub verified_at: i64,
    pub exec_display: String,
}

/// Present-but-unusable is its own state: a directory, or a file with no execute bit, is
/// `not_executable`, never `missing`. §11.5's window says different things about the two.
#[must_use]
pub fn verify_path(exec: &Path) -> VerifyState {
    let Ok(meta) = std::fs::metadata(exec) else {
        return VerifyState::Missing;
    };
    if !meta.is_file() {
        return VerifyState::NotExecutable;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        if meta.permissions().mode() & 0o111 == 0 {
            return VerifyState::NotExecutable;
        }
    }
    VerifyState::Ok
}

fn record(
    tx: &rusqlite::Transaction<'_>,
    target: &crate::launch::resolve::StoredTarget,
    now: i64,
) -> Result<Verification, LaunchError> {
    let path = crate::paths::path_from_bytes(&target.exec_bytes);
    let state = verify_path(&path);
    tx.execute(
        "UPDATE launch_target SET verify_state = ?2, verified_at = ?3 WHERE id = ?1",
        rusqlite::params![target.id, state.as_str(), now],
    )?;
    Ok(Verification {
        target_id: target.id,
        verify_state: state,
        verified_at: now,
        exec_display: exec_display(target),
    })
}

/// Every stored row, override, language and global alike, disabled ones included.
/// Language-blind by construction: the query has no `language` predicate at all.
pub fn verify_all(
    conn: &mut rusqlite::Connection,
    now: i64,
) -> Result<Vec<Verification>, LaunchError> {
    let ids = crate::launch::resolve::all_target_ids(conn)?;
    let mut all = Vec::with_capacity(ids.len());
    for id in ids {
        all.push(crate::launch::resolve::load_any_target(conn, id)?);
    }
    let _guard = TxGuard::enter();
    let tx = conn.transaction()?;
    let mut out = Vec::with_capacity(all.len());
    for target in &all {
        out.push(record(&tx, target, now)?);
    }
    tx.commit()?;
    Ok(out)
}

pub fn verify_one(
    conn: &mut rusqlite::Connection,
    target_id: i64,
    now: i64,
) -> Result<Verification, LaunchError> {
    let target = crate::launch::resolve::load_target(conn, target_id)?;
    let _guard = TxGuard::enter();
    let tx = conn.transaction()?;
    let out = record(&tx, &target, now)?;
    tx.commit()?;
    Ok(out)
}
