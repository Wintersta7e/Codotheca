//! §11.3's settings, stored in `app_meta` and migrated with the schema.
//! §1.9 lists `effects_tier`; the other five keys are added here — `app_meta`
//! is a key/value table, so no migration is involved.

use crate::index::IndexError;
use crate::proto::dispatch::{parse_args, CommandFailure}; // R15: one helper, plan 03's
use crate::proto::txguard::TxGuard;
use crate::protocol::{EffectsTier, LogLevel, Settings, SettingsPatch, SettingsSetArgs};
use crate::surfaces::SurfaceCtx;

pub const KEY_EFFECTS_TIER: &str = "effects_tier";
pub const KEY_REDUCED_MOTION_OVERRIDE: &str = "reduced_motion_override";
pub const KEY_AUTOSTART: &str = "autostart";
pub const KEY_RESIDENT_SHORTCUT: &str = "resident_shortcut";
pub const KEY_ROAST_ENABLED: &str = "roast_enabled";
pub const KEY_LOG_LEVEL: &str = "log_level";

/// §11.3 and §8.6 fix every one of these.
pub const DEFAULTS: Settings = Settings {
    effects_tier: EffectsTier::Auto,
    reduced_motion_override: false,
    autostart: false,
    resident_shortcut: None,
    roast_enabled: true,
    log_level: LogLevel::Info,
};

/// # Errors
/// Fails when the index cannot be read.
pub fn handle_get(ctx: &SurfaceCtx<'_>) -> Result<Settings, CommandFailure> {
    read(ctx.index.conn()).map_err(|e| CommandFailure::internal(e.to_string()))
}

/// # Errors
/// Fails when the arguments do not parse, when the proposed chord binds nothing, or when the
/// index cannot be written.
pub fn handle_set(
    ctx: &SurfaceCtx<'_>,
    args: serde_json::Value,
) -> Result<Settings, CommandFailure> {
    let args: SettingsSetArgs = parse_args(args)?;
    if let Some(chord) = args.patch.resident_shortcut.as_deref() {
        // The empty string is the clear, and clearing needs no modifier.
        if !chord.is_empty() && !validate_shortcut(chord) {
            return Err(CommandFailure::protocol(
                "a shortcut needs a modifier and a key",
            ));
        }
    }
    write(ctx.index.conn(), &args.patch).map_err(|e| CommandFailure::internal(e.to_string()))
}

fn get(conn: &rusqlite::Connection, key: &str) -> Result<Option<String>, IndexError> {
    let mut stmt = conn.prepare("SELECT v FROM app_meta WHERE k = ?1")?;
    let mut rows = stmt.query([key])?;
    Ok(match rows.next()? {
        Some(row) => Some(row.get(0)?),
        None => None,
    })
}

fn put(conn: &rusqlite::Connection, key: &str, value: &str) -> Result<(), IndexError> {
    conn.execute(
        "INSERT INTO app_meta (k, v) VALUES (?1, ?2)
         ON CONFLICT(k) DO UPDATE SET v = excluded.v",
        rusqlite::params![key, value],
    )?;
    Ok(())
}

/// A stored value this build cannot read is the default, not a failed read: §11.3's drawer
/// must open on a database written by a newer build.
fn parse_enum<T: serde::de::DeserializeOwned>(raw: Option<String>, fallback: T) -> T {
    raw.and_then(|v| serde_json::from_value(serde_json::Value::String(v)).ok())
        .unwrap_or(fallback)
}

/// # Errors
/// Fails when the index cannot be read.
pub fn read(conn: &rusqlite::Connection) -> Result<Settings, IndexError> {
    let shortcut = get(conn, KEY_RESIDENT_SHORTCUT)?.filter(|s| !s.is_empty());
    Ok(Settings {
        effects_tier: parse_enum(get(conn, KEY_EFFECTS_TIER)?, DEFAULTS.effects_tier),
        reduced_motion_override: get(conn, KEY_REDUCED_MOTION_OVERRIDE)?.as_deref() == Some("1"),
        autostart: get(conn, KEY_AUTOSTART)?.as_deref() == Some("1"),
        resident_shortcut: shortcut,
        roast_enabled: get(conn, KEY_ROAST_ENABLED)?.map_or(DEFAULTS.roast_enabled, |v| v == "1"),
        log_level: parse_enum(get(conn, KEY_LOG_LEVEL)?, DEFAULTS.log_level),
    })
}

/// Applies the fields the patch names and leaves the rest alone.
///
/// # Errors
/// Fails when the index cannot be written.
pub fn write(conn: &rusqlite::Connection, patch: &SettingsPatch) -> Result<Settings, IndexError> {
    {
        let _guard = TxGuard::enter();
        let tx = conn.unchecked_transaction()?;
        if let Some(v) = patch.effects_tier {
            put(&tx, KEY_EFFECTS_TIER, enum_str(&v)?.as_str())?;
        }
        if let Some(v) = patch.reduced_motion_override {
            put(&tx, KEY_REDUCED_MOTION_OVERRIDE, bit(v))?;
        }
        if let Some(v) = patch.autostart {
            put(&tx, KEY_AUTOSTART, bit(v))?;
        }
        if let Some(v) = patch.resident_shortcut.as_deref() {
            put(&tx, KEY_RESIDENT_SHORTCUT, v)?;
        }
        if let Some(v) = patch.roast_enabled {
            put(&tx, KEY_ROAST_ENABLED, bit(v))?;
        }
        if let Some(v) = patch.log_level {
            put(&tx, KEY_LOG_LEVEL, enum_str(&v)?.as_str())?;
        }
        tx.commit()?;
    }
    read(conn)
}

fn bit(on: bool) -> &'static str {
    if on {
        "1"
    } else {
        "0"
    }
}

fn enum_str<T: serde::Serialize>(value: &T) -> Result<String, IndexError> {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(s)) => Ok(s),
        _ => Err(IndexError::Sidecar(
            "enum did not serialise as a string".into(),
        )),
    }
}

/// An accelerator needs at least one modifier and exactly one final key, or it
/// takes a bare keystroke away from every application on the machine.
#[must_use]
pub fn validate_shortcut(chord: &str) -> bool {
    const MODIFIERS: [&str; 5] = ["Control", "Alt", "Shift", "Super", "AltGr"];
    let parts: Vec<&str> = chord.split('+').collect();
    if parts.len() < 2 {
        return false;
    }
    let Some((key, mods)) = parts.split_last() else {
        return false;
    };
    !key.is_empty()
        && !MODIFIERS.contains(key)
        && !mods.is_empty()
        && mods.iter().all(|m| MODIFIERS.contains(m))
}
