//! §43.5's silent class: a rise the user did not just cause moves the number and makes no line.
//!
//! Each writer in the class takes one [`SilentAbsorb`] in its own transaction — `begin` before its
//! first write, `finish` after its last — and the guard decides from what it read whether a line
//! was already pending. With none pending, the level shown follows the displayed level, down as
//! well as up (§43.5's last paragraph); with one pending, the rise folds into it.
//!
//! It reads no quiet-mode key: a silent write is silent in every mode.

use rusqlite::Transaction;

use super::keys::{advance_level_line_since, level_line_since, level_shown, write_level_shown};
use super::reason::COUNTED_ROW_SQL;
use crate::index::IndexError;
use crate::ledger::level::read_level;

/// Why a write is silent — §43.5's four classes. Every other level-moving write is forward.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SilentClass {
    /// A J4 settle whose history is being read rather than extended: the project's first read,
    /// or the re-read an upgrade or the user asked for.
    FirstRead,
    /// A recompute the user confirmed: an identity answer applied, or a merge.
    Confirmed,
    /// A change of the price table or the curve, detected at start-up. The old level cannot be
    /// read any more, so whether a line was pending is decided from the ledger's rows.
    Rebalance,
    /// A sidecar restore. Restored rows carry no silent mark, so the reason's mark always moves
    /// past them, pending line or not.
    Restore,
}

/// One silent write's guard over its own transaction.
///
/// Inert while no level has been presented: it then reads no level at all, so the first scan's
/// many settles pay nothing for it.
#[derive(Debug)]
#[must_use = "a silent write that never finishes its absorb leaves the rise as a line"]
pub struct SilentAbsorb {
    armed: Option<Armed>,
}

/// What `begin` read, for `finish`.
#[derive(Debug)]
struct Armed {
    class: SilentClass,
    /// A line was pending before the write: the rise folds into it.
    pending_before: bool,
    /// The writer's instant, which the level reading's floor ratchet is dated by.
    now: i64,
}

impl SilentAbsorb {
    /// Open the guard, before the writer's first write in `tx`.
    ///
    /// # Errors
    /// Fails when the index cannot be read or a stored key does not parse.
    pub fn begin(tx: &Transaction<'_>, class: SilentClass, now: i64) -> Result<Self, IndexError> {
        let Some(shown) = level_shown(tx)? else {
            return Ok(Self { armed: None });
        };
        let pending_before = if class == SilentClass::Rebalance {
            counted_rows_since_mark(tx)?
        } else {
            displayed(tx, now)?.is_some_and(|level| level > shown)
        };
        Ok(Self {
            armed: Some(Armed {
                class,
                pending_before,
                now,
            }),
        })
    }

    /// Close the guard, after the writer's last write in `tx` and before its commit.
    ///
    /// # Errors
    /// Fails when the index cannot be read or a key cannot be written.
    pub fn finish(self, tx: &Transaction<'_>) -> Result<(), IndexError> {
        let Some(armed) = self.armed else {
            return Ok(());
        };
        if !armed.pending_before {
            if let Some(level) = displayed(tx, armed.now)? {
                return write_level_shown(tx, level);
            }
        }
        if armed.class == SilentClass::Restore {
            advance_level_line_since(tx)?;
        }
        Ok(())
    }
}

/// The displayed level, `None` while the reading carries no figures.
fn displayed(tx: &Transaction<'_>, now: i64) -> Result<Option<u32>, IndexError> {
    Ok(read_level(tx, now)?.figures.map(|figures| figures.level))
}

/// Whether a row the reason would count sits past its mark: a rebalance's reading of *a line was
/// pending*, since the level before it cannot be computed. It undercounts a line — a rise carried
/// only by accrual onto an existing row is absorbed — and never fabricates one. No mark is no
/// counted row, as the reason reads it.
fn counted_rows_since_mark(tx: &Transaction<'_>) -> Result<bool, IndexError> {
    let Some(since) = level_line_since(tx)? else {
        return Ok(false);
    };
    let any: bool = tx.query_row(
        &format!("SELECT EXISTS (SELECT 1 FROM xp_events x WHERE x.id > ?1 AND {COUNTED_ROW_SQL})"),
        [since],
        |r| r.get(0),
    )?;
    Ok(any)
}
