//! §43's `app_meta` keys. `app_meta` is key/value, so none of them costs a migration.

use crate::index::IndexError;
use crate::surfaces::settings::get;

/// §43.11's quiet-mode switch: `"1"` shows the motivation layer, `"0"` hides it. Absent is
/// shown — the layer is on until the user turns it off, and nothing ever asks.
pub const KEY_MOTIVATION_SHOWN: &str = "motivation_shown";

/// Whether the motivation layer is shown, the value `Settings.motivationShown` carries. An
/// absent key reads `true`, `roast_enabled`'s precedent: off only when the user said so.
///
/// # Errors
/// Fails when the index cannot be read.
pub fn motivation_shown(conn: &rusqlite::Connection) -> Result<bool, IndexError> {
    Ok(get(conn, KEY_MOTIVATION_SHOWN)?.is_none_or(|v| v == "1"))
}
