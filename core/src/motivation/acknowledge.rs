//! §43.15's `motivation.acknowledge`: what a presentation the user saw writes. The one writer of
//! day one and of the acknowledged badge line, and the caller of the level shown's writer.
//!
//! Every effect is write-once, a raise or an idempotent assignment, so a repeated
//! acknowledgement changes nothing. Under quiet mode nothing of §43's is written; only
//! `firstRunEnded` still ends first run, which is not §43's to hide.

use rusqlite::Transaction;

use super::keys::{
    level_shown, motivation_shown, write_level_shown, KEY_BADGE_LINE_ACKED, KEY_MOTIVATION_DAY_ONE,
};
use super::line::pending_line;
use crate::badges::evaluate::{sweep_unlatched, EvalCtx};
use crate::badges::kind_text;
use crate::badges::landing::last_landed;
use crate::clock::LocalZone;
use crate::index::IndexError;
use crate::ledger::level::read_level;
use crate::protocol::{MotivationAck, MotivationLineForm};
use crate::surfaces::settings::put;

/// What an acknowledgement did that its caller acts on.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AckEffect {
    /// This call wrote day one, and §40's arming sweep ran in its transaction.
    pub day_one_stamped: bool,
    /// The acknowledgement is the end of first run: the caller stamps it, in the same
    /// transaction, against the scan run in progress.
    pub first_run_ended: bool,
}

/// Record `event` in `tx`.
///
/// `level` is the level the renderer displayed, for the two events that carry one; a level
/// above the one the core displays is clamped to it, and one not above the level already shown
/// writes nothing.
///
/// Day one is `now`'s local date in `zone`, derived here and never taken from the caller.
///
/// # Errors
/// Fails when the index cannot be read or written, or a stored value is not one this build
/// reads.
pub fn acknowledge(
    tx: &Transaction<'_>,
    event: MotivationAck,
    level: Option<u32>,
    now: i64,
    zone: &dyn LocalZone,
) -> Result<AckEffect, IndexError> {
    let first_run_ended = event == MotivationAck::FirstRunEnded;
    let mut effect = AckEffect {
        day_one_stamped: false,
        first_run_ended,
    };
    if !motivation_shown(tx)? {
        return Ok(effect);
    }
    let ctx = EvalCtx { now, zone };
    match event {
        MotivationAck::BandShown => {
            effect.day_one_stamped = stamp_day_one(tx, &ctx)?;
        }
        MotivationAck::RevealLevelShown => {
            effect.day_one_stamped = stamp_day_one(tx, &ctx)?;
            raise_level_shown(tx, level, now)?;
        }
        MotivationAck::LevelLineClicked => raise_level_shown(tx, level, now)?,
        MotivationAck::BadgeLineClicked => {
            let reading = read_level(tx, now)?;
            let landed = pending_line(tx, &reading, &ctx)?
                .filter(|line| line.form == MotivationLineForm::Badge)
                .and_then(|line| line.award)
                .filter(|award| award.landed);
            if let Some(award) = landed {
                put(tx, KEY_BADGE_LINE_ACKED, &kind_text(award.kind))?;
            }
        }
        // The award the reveal's panel landed was seen there: it never returns as a line.
        MotivationAck::FirstRunEnded => {
            if let Some(kind) = last_landed(tx)? {
                put(tx, KEY_BADGE_LINE_ACKED, &kind_text(kind))?;
            }
        }
    }
    Ok(effect)
}

/// Writes day one as `now`'s local date if it is unwritten, and only then runs §40's arming
/// sweep in the same transaction. `true` when this call wrote it.
fn stamp_day_one(tx: &Transaction<'_>, ctx: &EvalCtx<'_>) -> Result<bool, IndexError> {
    let today = ctx.zone.at(ctx.now).date();
    let written = tx.execute(
        "INSERT INTO app_meta (k, v) VALUES (?1, ?2) ON CONFLICT (k) DO NOTHING",
        [KEY_MOTIVATION_DAY_ONE, today.as_str()],
    )? == 1;
    if written {
        sweep_unlatched(tx, ctx)?;
    }
    Ok(written)
}

/// Raises the level shown to `level`, clamped to the displayed level. Nothing is written when
/// there is no level, the core displays none, or the result is not above the level shown — so
/// a repeat cannot move the reason's mark past rows a pending rise still names.
fn raise_level_shown(tx: &Transaction<'_>, level: Option<u32>, now: i64) -> Result<(), IndexError> {
    let Some(level) = level else {
        return Ok(());
    };
    let Some(displayed) = read_level(tx, now)?.figures.map(|f| f.level) else {
        return Ok(());
    };
    let raised = level.min(displayed);
    if level_shown(tx)?.is_some_and(|shown| raised <= shown) {
        return Ok(());
    }
    write_level_shown(tx, raised)
}
