//! §43.4's queue: the one line the band holds until it is clicked. The level comes first, then
//! §40's due award, then a landed award whose line was never clicked; first match wins.
//!
//! The line is derived from stored state on every read, never held in the renderer, so it
//! survives navigation, a reload and a core restart.

use rusqlite::Transaction;

use super::keys::{
    level_line_return, level_line_since, level_shown, motivation_shown, KEY_BADGE_LINE_ACKED,
};
use super::reason::reason_since;
use crate::badges::evaluate::EvalCtx;
use crate::badges::landing::{due, last_landed, list_awards};
use crate::badges::{kind_text, BadgeError};
use crate::index::IndexError;
use crate::protocol::{BadgeKind, LevelReading, LevelState, MotivationLine, MotivationLineForm};
use crate::surfaces::settings::get;

/// The line pending over `reading`, or `None`.
///
/// Under quiet mode there is none. A level never presented and computable arrives; a displayed
/// level above the one shown rises — or returns, when quiet mode ended with it pending — and
/// several rises before a click are one line at the current level. With no level line, the
/// award §40 lands today is a line not yet landed, and the most recently landed award is a line
/// until its click is acknowledged.
///
/// It reads no clock: `ctx` carries the instant and the zone `due` dates by.
///
/// # Errors
/// Fails when the index cannot be read, or a stored key, kind or evidence document is not one
/// this build reads.
pub fn pending_line(
    tx: &Transaction<'_>,
    reading: &LevelReading,
    ctx: &EvalCtx<'_>,
) -> Result<Option<MotivationLine>, IndexError> {
    if !motivation_shown(tx)? {
        return Ok(None);
    }
    if let Some(line) = level_line(tx, reading)? {
        return Ok(Some(line));
    }
    let due = due(tx, ctx).map_err(index_error)?;
    let unacknowledged = || -> Result<Option<BadgeKind>, IndexError> {
        let Some(newest) = last_landed(tx)? else {
            return Ok(None);
        };
        let acked = get(tx, KEY_BADGE_LINE_ACKED)?;
        Ok((acked.as_deref() != Some(kind_text(newest).as_str())).then_some(newest))
    };
    let Some(kind) = due.map_or_else(unacknowledged, |kind| Ok(Some(kind)))? else {
        return Ok(None);
    };
    let award = list_awards(tx)
        .map_err(index_error)?
        .into_iter()
        .find(|award| award.kind == kind)
        .ok_or_else(|| IndexError::Corrupt {
            detail: format!("the {} award is due and not listed", kind_text(kind)),
        })?;
    Ok(Some(MotivationLine {
        form: MotivationLineForm::Badge,
        level: None,
        reason: None,
        award: Some(award),
    }))
}

/// The level's line: an arrival, a rise or a return, carrying the displayed level.
fn level_line(
    tx: &Transaction<'_>,
    reading: &LevelReading,
) -> Result<Option<MotivationLine>, IndexError> {
    let Some(figures) = &reading.figures else {
        return Ok(None);
    };
    let level = |form, reason| MotivationLine {
        form,
        level: Some(figures.level),
        reason,
        award: None,
    };
    let Some(shown) = level_shown(tx)? else {
        let computable = reading.state != LevelState::NotComputed;
        return Ok(computable.then(|| level(MotivationLineForm::Arrival, None)));
    };
    if figures.level <= shown {
        return Ok(None);
    }
    if level_line_return(tx)?.is_some() {
        return Ok(Some(level(MotivationLineForm::Return, None)));
    }
    let reason = reason_since(tx, level_line_since(tx)?)?;
    Ok(Some(level(MotivationLineForm::Rise, reason)))
}

/// A badge read's failure as the index's: the line is part of an index read, and no badge
/// failure here is the caller's to answer differently.
fn index_error(error: BadgeError) -> IndexError {
    match error {
        BadgeError::Index(inner) => inner,
        other => IndexError::Corrupt {
            detail: other.to_string(),
        },
    }
}
