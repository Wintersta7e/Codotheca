//! §38.10: the ledger's rows no rebuild can re-derive, as three of §48.8.3's sections — the floor,
//! the library's own awards, and the git rows of a project no scan can find again.
//!
//! A retained row with a project is not here: it rides the per-project `xp_events` collection and
//! comes back when its subject does. Every other git row is J4's to write again.

use rusqlite::{Connection, Transaction};
use serde::de::DeserializeOwned;

use super::floor::{ratchet_level_floor, read_floor};
use crate::index::noscan::NO_SCAN_PREDICATE_SQL;
use crate::index::sidecar::{
    dump_row, insert_row, RestoreCtx, RestoreOutcome, SectionRow, SidecarRow, SidecarValue,
};
use crate::index::IndexError;

/// The floor, as its section carries it.
#[derive(serde::Serialize, serde::Deserialize)]
struct FloorRecord {
    level: u32,
}

fn row_error(section: &str, error: impl std::fmt::Display) -> IndexError {
    IndexError::Sidecar(format!("a {section} row: {error}"))
}

fn decode<T: DeserializeOwned>(section: &str, row: &SectionRow) -> Result<T, IndexError> {
    serde_json::from_value(row.data.clone()).map_err(|e| row_error(section, e))
}

/// `ledger_floor`'s export: the floor's level, as one row of no project.
pub(crate) fn export_floor(conn: &Connection) -> Result<Vec<SectionRow>, IndexError> {
    let record = FloorRecord {
        level: read_floor(conn)?,
    };
    Ok(vec![SectionRow {
        subject: None,
        location_keys: Vec::new(),
        data: serde_json::to_value(record).map_err(|e| row_error("ledger_floor", e))?,
    }])
}

/// `ledger_floor`'s restore: a raise through the floor's one writer, so a restored floor lower
/// than the present one changes nothing and never aborts the rebuild. Answers one when it raised.
pub(crate) fn restore_floor(
    tx: &Transaction<'_>,
    row: &SectionRow,
    ctx: &RestoreCtx<'_>,
) -> Result<RestoreOutcome, IndexError> {
    let record: FloorRecord = decode("ledger_floor", row)?;
    let raised = ratchet_level_floor(tx, record.level, ctx.now)?;
    Ok(RestoreOutcome::Applied(u64::from(raised)))
}

/// The `xp_events` rows `select` names by `(id, subject_key)`, each carried whole but for its
/// `id`: the restoring index allocates its own, and an event's id names nothing outside it.
fn whole_rows(
    conn: &Connection,
    section: &str,
    select: &str,
) -> Result<Vec<SectionRow>, IndexError> {
    let ids: Vec<(i64, Option<String>)> = conn
        .prepare(select)?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<Result<_, _>>()?;
    ids.into_iter()
        .map(|(id, subject)| {
            let mut row = dump_row(conn, "xp_events", id)?;
            row.remove("id");
            Ok(SectionRow {
                subject,
                location_keys: Vec::new(),
                data: serde_json::to_value(row).map_err(|e| row_error(section, e))?,
            })
        })
        .collect()
}

/// `ledger_library_awards`' export: every row of no project — the pre-pass award, which no
/// subject's return could bring back.
pub(crate) fn export_library_awards(conn: &Connection) -> Result<Vec<SectionRow>, IndexError> {
    whole_rows(
        conn,
        "ledger_library_awards",
        "SELECT id, NULL FROM xp_events WHERE project_id IS NULL ORDER BY id",
    )
}

/// `ledger_library_awards`' restore: the row as exported, only if no row holds its `dedupe_key`.
pub(crate) fn restore_library_award(
    tx: &Transaction<'_>,
    row: &SectionRow,
    _ctx: &RestoreCtx<'_>,
) -> Result<RestoreOutcome, IndexError> {
    let record: SidecarRow = decode("ledger_library_awards", row)?;
    let Some(SidecarValue::Text(key)) = record.get("dedupe_key") else {
        return Err(row_error(
            "ledger_library_awards",
            "it carries no dedupe_key",
        ));
    };
    let present: bool = tx.query_row(
        "SELECT EXISTS (SELECT 1 FROM xp_events WHERE dedupe_key = ?1)",
        [key],
        |r| r.get(0),
    )?;
    if present {
        return Ok(RestoreOutcome::Applied(0));
    }
    insert_row(tx, "xp_events", &record, &["id"])?;
    Ok(RestoreOutcome::Applied(1))
}

/// `ledger_frozen_git`'s export: the git rows of every project with no copy whose `removed_at` is
/// NULL (§38.7.1). The no-scan predicate is spliced in, so each such project is one the
/// `no_scan_projects` section re-creates, its id kept, before these rows restore onto it.
pub(crate) fn export_frozen_git(conn: &Connection) -> Result<Vec<SectionRow>, IndexError> {
    whole_rows(
        conn,
        "ledger_frozen_git",
        &format!(
            "SELECT x.id, x.subject_key FROM xp_events x JOIN project p ON p.id = x.project_id
              WHERE x.track = 'git' AND {NO_SCAN_PREDICATE_SQL}
                AND NOT EXISTS (SELECT 1 FROM location live
                                 WHERE live.project_id = p.id AND live.removed_at IS NULL)
              ORDER BY x.id"
        ),
    )
}

/// `ledger_frozen_git`'s restore: the row verbatim, on the project it was exported from.
pub(crate) fn restore_frozen_git(
    tx: &Transaction<'_>,
    row: &SectionRow,
    _ctx: &RestoreCtx<'_>,
) -> Result<RestoreOutcome, IndexError> {
    let record: SidecarRow = decode("ledger_frozen_git", row)?;
    insert_row(tx, "xp_events", &record, &["id"])?;
    Ok(RestoreOutcome::Applied(1))
}
