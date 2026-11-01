//! §39.12's two targets: one reader and one writer per `app_meta` key.
//!
//! A stored value this build cannot read is the default, never a failed read — §11.3's drawer
//! must open on a database a newer build wrote. The rhythm pair is always written together, in
//! the caller's one transaction, so `rhythm_target_basis` is present exactly when
//! `rhythm_target` is.

use crate::index::IndexError;
use crate::protocol::{RhythmTargetBasis, SettingsPatch};
use crate::rings::{
    FOCUS_TARGET_DEFAULT_MINUTES, FOCUS_TARGET_MAX_MINUTES, FOCUS_TARGET_MIN_MINUTES,
};
use crate::surfaces::settings::{get, put};

/// `app_meta` key for the Focus target, in whole minutes, 30–1380.
pub const KEY_FOCUS_TARGET_MINUTES: &str = "focus_target_minutes";
/// `app_meta` key for the rhythm target, active days per week, 1–7. Absent before the first
/// settlement and before any choice.
pub const KEY_RHYTHM_TARGET: &str = "rhythm_target";
/// `app_meta` key for how the rhythm target was set, stored as its protocol string.
pub const KEY_RHYTHM_TARGET_BASIS: &str = "rhythm_target_basis";

/// The fewest active days a week can be asked for.
const RHYTHM_TARGET_MIN: u32 = 1;
/// The most: every day of the week.
const RHYTHM_TARGET_MAX: u32 = 7;

/// Why a target was not stored.
#[derive(Debug, thiserror::Error)]
pub enum TargetError {
    /// The value lies outside the field's range; nothing was written.
    #[error("{field} must be {lo}–{hi}, not {value}")]
    OutOfRange {
        /// The `SettingsPatch` field, as the wire spells it.
        field: &'static str,
        /// The refused value.
        value: u32,
        /// The lowest allowed value.
        lo: u32,
        /// The highest allowed value.
        hi: u32,
    },
    /// The index could not be written.
    #[error(transparent)]
    Index(#[from] IndexError),
}

impl From<TargetError> for IndexError {
    /// `settings::write` answers in `IndexError`; an out-of-range value reaching it is a value
    /// that cannot be converted to a stored one, which is what rusqlite's variant names.
    fn from(error: TargetError) -> Self {
        match error {
            TargetError::Index(e) => e,
            refused @ TargetError::OutOfRange { .. } => {
                Self::Sqlite(rusqlite::Error::ToSqlConversionFailure(Box::new(refused)))
            }
        }
    }
}

const fn in_range(field: &'static str, value: u32, lo: u32, hi: u32) -> Result<u32, TargetError> {
    if value >= lo && value <= hi {
        Ok(value)
    } else {
        Err(TargetError::OutOfRange {
            field,
            value,
            lo,
            hi,
        })
    }
}

const fn check_focus(minutes: u32) -> Result<u32, TargetError> {
    in_range(
        "focusTargetMinutes",
        minutes,
        FOCUS_TARGET_MIN_MINUTES,
        FOCUS_TARGET_MAX_MINUTES,
    )
}

const fn check_rhythm(target: u32) -> Result<u32, TargetError> {
    in_range("rhythmTarget", target, RHYTHM_TARGET_MIN, RHYTHM_TARGET_MAX)
}

/// The range check `settings.set` runs before anything is written, over the targets the patch
/// names.
///
/// # Errors
/// [`TargetError::OutOfRange`] for the first named target outside its range.
pub fn check_patch(patch: &SettingsPatch) -> Result<(), TargetError> {
    if let Some(minutes) = patch.focus_target_minutes {
        check_focus(minutes)?;
    }
    if let Some(target) = patch.rhythm_target {
        check_rhythm(target)?;
    }
    Ok(())
}

/// The Focus target in minutes; absent, unreadable or out of range reads 45.
///
/// # Errors
/// Fails when the index cannot be read.
pub fn focus_target_minutes(conn: &rusqlite::Connection) -> Result<u32, IndexError> {
    Ok(get(conn, KEY_FOCUS_TARGET_MINUTES)?
        .and_then(|v| v.parse::<u32>().ok())
        .and_then(|v| check_focus(v).ok())
        .unwrap_or(FOCUS_TARGET_DEFAULT_MINUTES))
}

/// The rhythm target and its basis: present iff both keys parse and the value is 1–7.
///
/// Absent before the first settlement and before any choice — never a default, because a
/// target nobody set is not one the week is judged against.
///
/// # Errors
/// Fails when the index cannot be read.
pub fn rhythm_target(
    conn: &rusqlite::Connection,
) -> Result<Option<(u32, RhythmTargetBasis)>, IndexError> {
    let target = get(conn, KEY_RHYTHM_TARGET)?
        .and_then(|v| v.parse::<u32>().ok())
        .and_then(|v| check_rhythm(v).ok());
    let basis = get(conn, KEY_RHYTHM_TARGET_BASIS)?.and_then(|v| {
        serde_json::from_value::<RhythmTargetBasis>(serde_json::Value::String(v)).ok()
    });
    Ok(target.zip(basis))
}

/// `settings.set` only: stores the Focus target.
///
/// # Errors
/// [`TargetError::OutOfRange`] outside 30–1380, with nothing written; [`TargetError::Index`]
/// when the index cannot be written.
pub fn set_focus_target(tx: &rusqlite::Transaction<'_>, minutes: u32) -> Result<(), TargetError> {
    let minutes = check_focus(minutes)?;
    put(tx, KEY_FOCUS_TARGET_MINUTES, &minutes.to_string())?;
    Ok(())
}

/// `settings.set` only: the user's choice, basis `chosen`. It governs weeks settled after it
/// (§39.8), and the first settlement keeps it rather than deriving one.
///
/// # Errors
/// [`TargetError::OutOfRange`] outside 1–7, with nothing written; [`TargetError::Index`] when
/// the index cannot be written.
pub fn choose_rhythm_target(
    tx: &rusqlite::Transaction<'_>,
    target: u32,
) -> Result<(), TargetError> {
    let target = check_rhythm(target)?;
    put_rhythm_pair(tx, target, RhythmTargetBasis::Chosen)?;
    Ok(())
}

/// The settlement pass only: writes a `derived` or `fallback` target, never `chosen` — the
/// app never stands in for the user's choice.
pub(crate) fn settle_rhythm_target(
    tx: &rusqlite::Transaction<'_>,
    target: u32,
    basis: RhythmTargetBasis,
) -> Result<(), IndexError> {
    debug_assert_ne!(basis, RhythmTargetBasis::Chosen);
    put_rhythm_pair(tx, target, basis)
}

/// Both rhythm keys, always together, inside the caller's transaction.
fn put_rhythm_pair(
    tx: &rusqlite::Transaction<'_>,
    target: u32,
    basis: RhythmTargetBasis,
) -> Result<(), IndexError> {
    let serde_json::Value::String(basis) =
        serde_json::to_value(basis).map_err(|e| IndexError::Sidecar(e.to_string()))?
    else {
        return Err(IndexError::Sidecar(
            "a generated enum did not serialise as a string".into(),
        ));
    };
    put(tx, KEY_RHYTHM_TARGET, &target.to_string())?;
    put(tx, KEY_RHYTHM_TARGET_BASIS, &basis)?;
    Ok(())
}
