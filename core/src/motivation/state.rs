//! §43.3's one payload: what the status band draws, the answer to `motivation.get` and the body
//! of `motivation/changed`. **The core owns every decision; the renderer derives nothing**
//! (§43.5).

use rusqlite::Transaction;

use super::keys::motivation_shown;
use super::line::pending_line;
use crate::badges::evaluate::EvalCtx;
use crate::clock::LocalZone;
use crate::index::IndexError;
use crate::ledger::level::read_level;
use crate::ledger::xp::xp_by_term;
use crate::protocol::{LevelReading, MotivationState, XpTermTotal};
use crate::rings::read_rings;

/// The band's payload at `now` in `zone`.
///
/// Under quiet mode it is `shown: false` and nothing else, stripped here at the source. Shown,
/// it carries the level reading, the rings, the bar, the XP by term — absent while the level
/// carries no figures, never an empty list read as zero — and the pending line. It reads no
/// clock and computes no local date of its own: the zone arrives as a parameter.
///
/// # Errors
/// Fails when the index cannot be read, or a stored value is not one this build reads.
pub fn motivation_state(
    tx: &Transaction<'_>,
    now: i64,
    zone: &dyn LocalZone,
) -> Result<MotivationState, IndexError> {
    if !motivation_shown(tx)? {
        return Ok(MotivationState {
            shown: false,
            level: None,
            rings: None,
            xp_bar: None,
            pending_line: None,
            terms: None,
        });
    }
    let level = read_level(tx, now)?;
    let rings = read_rings(tx, now, zone)?;
    let terms = match level.figures {
        Some(_) => Some(
            xp_by_term(tx)?
                .terms
                .iter()
                .map(|&(term, xp)| XpTermTotal { term, xp })
                .collect(),
        ),
        None => None,
    };
    let pending_line = pending_line(tx, &level, &EvalCtx { now, zone })?;
    Ok(MotivationState {
        shown: true,
        xp_bar: xp_bar(&level),
        level: Some(level),
        rings: Some(rings),
        pending_line,
        terms,
    })
}

/// How far the XP is through the displayed level.
///
/// `(xp − T(level)) / (T(level + 1) − T(level))`; `None` when the reading carries no figures or
/// the level is held up by the floor, whose XP sits below the level's start.
#[must_use]
pub fn xp_bar(reading: &LevelReading) -> Option<f64> {
    let figures = reading.figures.as_ref().filter(|f| !f.floored)?;
    let span = figures.next_level_at.saturating_sub(figures.level_at);
    if span <= 0 {
        return None;
    }
    let into = figures.xp.saturating_sub(figures.level_at);
    // The bar is a fraction by definition, and the one figure the core derives in floating point.
    #[allow(clippy::float_arithmetic)]
    let bar = xp_as_f64(into) / xp_as_f64(span);
    Some(bar)
}

/// An XP count as a float, exact below 2^53, which no ledger reaches.
// Integer to float has no conversion function; this is the one place the module spells it.
#[allow(clippy::as_conversions, clippy::cast_precision_loss)]
const fn xp_as_f64(xp: i64) -> f64 {
    xp as f64
}
