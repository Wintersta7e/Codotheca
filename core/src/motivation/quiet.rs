//! §43.11's end of quiet mode: a rise that waited out quiet returns as a level alone.

use rusqlite::Transaction;

use super::keys::{level_shown, KEY_LEVEL_LINE_RETURN};
use crate::index::IndexError;
use crate::ledger::level::read_level;
use crate::surfaces::settings::put;

/// The switch moved from `was` to `now_shown`, in the transaction that wrote it.
///
/// Turning the layer back on with a level above the one last shown records that level as the
/// return: the line then takes the `return` form, the level and no reason, since what earned it
/// was never on screen. A level never shown arrives through the queue instead, and no rise makes
/// no line. Any other move writes nothing. `now` dates the level reading.
///
/// # Errors
/// Fails when the index cannot be read, a stored level does not parse, or the write is refused.
pub fn on_shown_changed(
    tx: &Transaction<'_>,
    was: bool,
    now_shown: bool,
    now: i64,
) -> Result<(), IndexError> {
    if was || !now_shown {
        return Ok(());
    }
    let Some(shown) = level_shown(tx)? else {
        return Ok(());
    };
    if let Some(figures) = read_level(tx, now)?.figures {
        if figures.level > shown {
            put(tx, KEY_LEVEL_LINE_RETURN, &figures.level.to_string())?;
        }
    }
    Ok(())
}
