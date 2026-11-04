//! §11.3's settings, stored in `app_meta` and migrated with the schema.
//! §1.9 lists `effects_tier`; the other six keys are added here — `app_meta`
//! is a key/value table, so no migration is involved.

use crate::index::IndexError;
use crate::proto::dispatch::{parse_args, CommandFailure}; // R15: one helper, plan 03's
use crate::proto::txguard::TxGuard;
use crate::protocol::{EffectsTier, LogLevel, RootId, Settings, SettingsPatch, SettingsSetArgs};
use crate::rings::{targets, FOCUS_TARGET_DEFAULT_MINUTES};
use crate::surfaces::SurfaceCtx;

/// `app_meta` key for the effects tier, stored as its protocol string.
pub const KEY_EFFECTS_TIER: &str = "effects_tier";
/// `app_meta` key for the reduced-motion override, `"1"` for on.
pub const KEY_REDUCED_MOTION_OVERRIDE: &str = "reduced_motion_override";
/// `app_meta` key for launching at login, `"1"` for on.
pub const KEY_AUTOSTART: &str = "autostart";
/// `app_meta` key for the resident window's global shortcut; empty or absent is no shortcut.
pub const KEY_RESIDENT_SHORTCUT: &str = "resident_shortcut";
/// `app_meta` key for roasting inside an opened project card, `"1"` for on; absent is on.
pub const KEY_ROAST_ENABLED: &str = "roast_enabled";
/// `app_meta` key for the rolling log's level, stored as its protocol string.
pub const KEY_LOG_LEVEL: &str = "log_level";

/// §29.8's grant: whole-tree source-file reading, and **J7's alone** (A11.3).
///
/// Lockfiles and the four presence predicates ride J6's existing named-file grant — *"we read
/// your source code"* and *"we read your `package-lock.json`"* are different sentences to a user
/// and the second is already true. Off until the user turns it on.
pub const KEY_CONTENT_SCAN_ENABLED: &str = "content_scan_enabled";

/// Whether §29's blob read has been granted. **Absent is off**, which is the default the whole
/// gate depends on.
///
/// # Errors
/// Fails when the index cannot be read.
pub fn content_scan_enabled(conn: &rusqlite::Connection) -> Result<bool, IndexError> {
    Ok(get(conn, KEY_CONTENT_SCAN_ENABLED)?.as_deref() == Some("1"))
}

/// §24.3a's install root, chosen once from the roots that already exist.
///
/// `app_meta` is key/value, so this costs **no migration**. It stores a `RootId` rather than a
/// path because the renderer may never originate one: it picks from `roots.list`, and a folder
/// that is not yet a root reaches the product only through the shell-owned `IPC_PICK_ROOT` dialog.
pub const KEY_INSTALL_ROOT_ID: &str = "install_root_id";

/// §11.3 and §8.6 fix every one of these.
///
/// `install_root_id` defaults to `None` — **no root chosen**, which `install.preview` reports as
/// `InstallRefusal::NoInstallRootChosen`: a refusal the user can act on, and a different fact from
/// a root that *was* chosen and has since gone away (`RootUnavailable`).
pub const DEFAULTS: Settings = Settings {
    effects_tier: EffectsTier::Auto,
    reduced_motion_override: false,
    autostart: false,
    resident_shortcut: None,
    roast_enabled: true,
    log_level: LogLevel::Info,
    install_root_id: None,
    // §29.8: **off until the user turns it on.** The whole gate depends on this default, and it
    // is the only setting whose default decides whether a file is opened at all.
    content_scan_enabled: false,
    // §30.9's per-check switches. Empty here and **not** a default value: the list is one entry
    // per `DebtSource` variant, always in full, and it is built by reading the enum rather than
    // by writing a table down. `read` fills it; this const carries the scalar defaults only.
    health_checks: Vec::new(),
    // §39.12: absent reads 45, and no rhythm target exists before the first settlement or a
    // choice — `None`, never a default the week would be judged against.
    focus_target_minutes: FOCUS_TARGET_DEFAULT_MINUTES,
    rhythm_target: None,
    // §43.11: the motivation layer is shown until the user turns it off; nothing asks.
    motivation_shown: true,
};

/// # Errors
/// Fails when the index cannot be read.
pub fn handle_get(ctx: &SurfaceCtx<'_>) -> Result<Settings, CommandFailure> {
    read(ctx.index.conn()).map_err(|e| CommandFailure::internal(e.to_string()))
}

/// # Errors
/// Fails when the arguments do not parse, when the proposed chord binds nothing, when a target
/// lies outside its range, or when the index cannot be written.
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
    // §39.13: an out-of-range target is the same refusal as an invalid chord, checked before
    // anything is written; `write` re-checks inside its transaction.
    targets::check_patch(&args.patch).map_err(|e| CommandFailure::protocol(e.to_string()))?;
    write(ctx.index.conn(), &args.patch).map_err(|e| CommandFailure::internal(e.to_string()))
}

/// One `app_meta` value, `None` when the key is absent.
pub(crate) fn get(conn: &rusqlite::Connection, key: &str) -> Result<Option<String>, IndexError> {
    let mut stmt = conn.prepare("SELECT v FROM app_meta WHERE k = ?1")?;
    let mut rows = stmt.query([key])?;
    Ok(match rows.next()? {
        Some(row) => Some(row.get(0)?),
        None => None,
    })
}

/// Inserts or replaces one `app_meta` value.
pub(crate) fn put(conn: &rusqlite::Connection, key: &str, value: &str) -> Result<(), IndexError> {
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
        // An unparseable stored id is **no root chosen**, not a failed read — the same rule
        // `parse_enum` applies to a value written by a newer build. §11.3's drawer must open.
        install_root_id: get(conn, KEY_INSTALL_ROOT_ID)?
            .and_then(|v| v.parse::<i64>().ok())
            .map(RootId),
        content_scan_enabled: content_scan_enabled(conn)?,
        // §30.9: one entry per `DebtSource` variant, always in full, with an absent key read
        // as **on** — `roast_enabled`'s precedent, applied per check.
        health_checks: crate::health::switches::read_switches(conn)?,
        focus_target_minutes: targets::focus_target_minutes(conn)?,
        rhythm_target: targets::rhythm_target(conn)?.map(|(target, _basis)| target),
        motivation_shown: crate::motivation::keys::motivation_shown(conn)?,
    })
}

/// Applies the fields the patch names and leaves the rest alone.
///
/// [p4] §43.10: an `autostart` patch is a setting and nothing more — it does not end first run.
/// The stamp's writers are the turn's answer, the end of a scan run and the migration.
///
/// # Errors
/// Fails when the index cannot be written, or when a target lies outside its range — then the
/// whole patch is rolled back.
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
        if let Some(v) = patch.install_root_id {
            put(&tx, KEY_INSTALL_ROOT_ID, &v.0.to_string())?;
        }
        if let Some(v) = patch.health_checks.as_deref() {
            // §30.9. Applies only the entries it names, and an entry turned off takes the
            // source's sweep row with it — never a `debt_item` row.
            crate::health::switches::write_switches(&tx, v)?;
        }
        if let Some(v) = patch.focus_target_minutes {
            targets::set_focus_target(&tx, v)?;
        }
        if let Some(v) = patch.rhythm_target {
            // §39.8: basis `chosen`, so the first settlement keeps it rather than deriving one.
            targets::choose_rhythm_target(&tx, v)?;
        }
        if let Some(v) = patch.motivation_shown {
            put(&tx, crate::motivation::keys::KEY_MOTIVATION_SHOWN, bit(v))?;
        }
        if let Some(v) = patch.content_scan_enabled {
            put(&tx, KEY_CONTENT_SCAN_ENABLED, bit(v))?;
            // §29.8: turning it off **deletes what it wrote**, in this same transaction. A
            // promise that leaves the data behind is not the promise that was made.
            if !v {
                crate::jobs::content_scan::revoke_content_scan(&tx)?;
            }
        }
        tx.commit()?;
    }
    read(conn)
}

const fn bit(on: bool) -> &'static str {
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
