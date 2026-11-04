//! §43's `app_meta` keys. `app_meta` is key/value, so none of them costs a migration.
//!
//! Each key has one writer: day one the acknowledgement, the level shown and its mark
//! [`write_level_shown`] — the mark alone, a restore's, through the helper it calls — the return
//! quiet mode's end, quiet itself `settings.set`, and the acknowledged badge line the
//! acknowledgement.

use rusqlite::Transaction;

use crate::index::IndexError;
use crate::surfaces::settings::{get, put};

/// §43.11's quiet-mode switch: `"1"` shows the motivation layer, `"0"` hides it. Absent is
/// shown — the layer is on until the user turns it off, and nothing ever asks.
pub const KEY_MOTIVATION_SHOWN: &str = "motivation_shown";

/// §43.9's day one: the local date `YYYY-MM-DD` the layer was first on screen. Written once;
/// the rings, the rhythm and the badge landing all read it from here.
pub const KEY_MOTIVATION_DAY_ONE: &str = "motivation_day_one";

/// §43.5's displayed level the user was last shown, an integer; absent = never presented.
pub const KEY_LEVEL_SHOWN: &str = "level_shown";

/// The ledger's highest `xp_events.id` when [`KEY_LEVEL_SHOWN`] was last written: the line's
/// reason reads only the rows after it.
pub const KEY_LEVEL_LINE_SINCE: &str = "level_line_since";

/// The displayed level at the moment quiet mode ended with a rise pending: that rise returns
/// as a level alone, with no reason.
pub const KEY_LEVEL_LINE_RETURN: &str = "level_line_return";

/// The `BadgeKind` slug of the last badge line the user clicked: a landed award stays a line
/// until its click is acknowledged here.
pub const KEY_BADGE_LINE_ACKED: &str = "badge_line_acked";

/// Whether the motivation layer is shown, the value `Settings.motivationShown` carries. An
/// absent key reads `true`, `roast_enabled`'s precedent: off only when the user said so.
///
/// # Errors
/// Fails when the index cannot be read.
pub fn motivation_shown(conn: &rusqlite::Connection) -> Result<bool, IndexError> {
    Ok(get(conn, KEY_MOTIVATION_SHOWN)?.is_none_or(|v| v == "1"))
}

/// Day one as stored, a local date `YYYY-MM-DD`; `None` until the layer is first shown. The one
/// reader of the key — every other reader parses this.
///
/// # Errors
/// Fails when the index cannot be read.
pub fn day_one(conn: &rusqlite::Connection) -> Result<Option<String>, IndexError> {
    get(conn, KEY_MOTIVATION_DAY_ONE)
}

/// Whether `today`, a local date, is day one. **False while day one is unwritten**: what it
/// suppresses — the invitation, the recap — is suppressed only on a day the layer was shown.
///
/// # Errors
/// Fails when the index cannot be read.
pub fn is_day_one(conn: &rusqlite::Connection, today: &str) -> Result<bool, IndexError> {
    Ok(day_one(conn)?.is_some_and(|day| day == today))
}

/// The displayed level the user was last shown; `None` when no level has been presented.
///
/// # Errors
/// Fails when the index cannot be read, or the stored value is not a level.
pub fn level_shown(conn: &rusqlite::Connection) -> Result<Option<u32>, IndexError> {
    parsed(conn, KEY_LEVEL_SHOWN)
}

/// The ledger mark the line's reason reads after; `None` when no level has been presented.
///
/// # Errors
/// Fails when the index cannot be read, or the stored value is not a row id.
pub fn level_line_since(conn: &rusqlite::Connection) -> Result<Option<i64>, IndexError> {
    parsed(conn, KEY_LEVEL_LINE_SINCE)
}

/// The level a rise pending at quiet mode's end was recorded at; `None` when no rise waits to
/// return.
///
/// # Errors
/// Fails when the index cannot be read, or the stored value is not a level.
pub fn level_line_return(conn: &rusqlite::Connection) -> Result<Option<u32>, IndexError> {
    parsed(conn, KEY_LEVEL_LINE_RETURN)
}

/// Record `level` as the level the user was shown.
///
/// The one writer of all three level keys: the level, the ledger's highest row id as the
/// reason's mark (0 on an empty ledger), and the return cleared — so the three can never be
/// written apart.
///
/// # Errors
/// Fails when SQLite refuses the read or a write.
pub fn write_level_shown(tx: &Transaction<'_>, level: u32) -> Result<(), IndexError> {
    put(tx, KEY_LEVEL_SHOWN, &level.to_string())?;
    advance_level_line_since(tx)?;
    tx.execute("DELETE FROM app_meta WHERE k = ?1", [KEY_LEVEL_LINE_RETURN])?;
    Ok(())
}

/// Move the reason's mark to the ledger's highest row id (0 on an empty ledger), the level shown
/// untouched: a restore's rows carry no silent mark, so only the mark keeps them out of a reason.
///
/// # Errors
/// Fails when SQLite refuses the read or the write.
pub(crate) fn advance_level_line_since(tx: &Transaction<'_>) -> Result<(), IndexError> {
    let mark: i64 = tx.query_row("SELECT COALESCE(MAX(id), 0) FROM xp_events", [], |r| {
        r.get(0)
    })?;
    put(tx, KEY_LEVEL_LINE_SINCE, &mark.to_string())
}

/// A numeric key, `None` when absent; a stored value that does not parse is a corrupt index,
/// never a silently missing one.
fn parsed<T: std::str::FromStr>(
    conn: &rusqlite::Connection,
    key: &str,
) -> Result<Option<T>, IndexError> {
    get(conn, key)?
        .map(|text| {
            text.parse().map_err(|_| IndexError::Corrupt {
                detail: format!("{key} is not a number: {text}"),
            })
        })
        .transpose()
}
