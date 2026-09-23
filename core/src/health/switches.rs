//! §30.9 — the per-check switches: `app_meta` keys, default **on**, one entry per variant.
//!
//! **Storage is the existing settings store**, `app_meta`, which is key/value — so this costs no
//! migration. The key is `health_check.<variant>`, the variant spelled from §28's `DebtSource`
//! **character-identical**, and the prefix is fixed so the set can be enumerated with
//! `LIKE 'health_check.%'` against a flat table that already holds `effects_tier`,
//! `roast_enabled` and five others.
//!
//! **The count is derived from the generated enum and never written down.** `DebtSource::ALL` is
//! emitted by `protocol/lib/emit-rust.mjs` from the schema's own variant list, so a tenth source
//! reaches this module by construction rather than by someone remembering.
//!
//! **Off hides; it never closes, and it never pays.** A switch that closed items would make
//! turning a check off an XP source, and closing an item because the user stopped looking is not
//! a state transition of the project: the item is still there when the check comes back on, no XP
//! was paid for the interval, and **the item's identity is unchanged across the toggle** — or
//! `level_floor` monotonicity is a promise the ledger cannot keep.
//!
//! **A check switched back on is `unknown` until it next runs — not `ok`, and not `0`.** The
//! mechanism is here rather than in a timestamp: turning a check off deletes its `debt_sweep`
//! row, because while the check is off the app makes no observation claim for that source. It
//! deletes **no `debt_item` row**. §29.8's grant does exactly this one step up — *"turning it off
//! deletes what it wrote"* — and a second key holding *when the switch last moved* would both
//! break §30.9's `LIKE 'health_check.%'` enumeration and be a time nobody reads for its own sake.

use rusqlite::{Connection, Transaction};

use crate::index::IndexError;
use crate::protocol::{DebtSource, HealthCheckSwitch};

/// §30.9's fixed prefix. `AC-P3-30-12` enumerates the set with it, so it is stated once.
pub const SWITCH_KEY_PREFIX: &str = "health_check.";

/// `health_check.<variant>`, the variant spelled through serde so the only vocabulary in this
/// module is `protocol/schema/protocol.json`'s.
///
/// # Errors
/// Fails when a generated enum does not serialise as a string, which would mean the emitter
/// changed shape under it.
pub fn switch_key(check: DebtSource) -> Result<String, IndexError> {
    match serde_json::to_value(check) {
        Ok(serde_json::Value::String(slug)) => Ok(format!("{SWITCH_KEY_PREFIX}{slug}")),
        _ => Err(IndexError::Sidecar(
            "a generated enum did not serialise as a string".into(),
        )),
    }
}

/// Every switch, **always in full** — one entry per `DebtSource` variant, with an absent key read
/// as **on** on `roast_enabled`'s existing precedent.
///
/// A short list would make *this check has no opinion stored* and *this check does not exist*
/// two spellings of one fact, which is the distinction §28's sweep rows exist to keep.
///
/// # Errors
/// Fails when the index cannot be read.
pub fn read_switches(conn: &Connection) -> Result<Vec<HealthCheckSwitch>, IndexError> {
    let mut out = Vec::with_capacity(DebtSource::ALL.len());
    for check in DebtSource::ALL {
        let key = switch_key(check)?;
        let stored: Option<String> = conn
            .query_row("SELECT v FROM app_meta WHERE k = ?1", [&key], |r| r.get(0))
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(IndexError::from(other)),
            })?;
        out.push(HealthCheckSwitch {
            check,
            // Absent is **on**. The whole default lives in this one expression.
            enabled: stored.map_or(true, |v| v == "1"),
        });
    }
    Ok(out)
}

/// R128/F8: `todo_marker` is the one source whose evidence needs §29.8's content-scan grant.
#[must_use]
pub const fn needs_grant(source: DebtSource) -> bool {
    matches!(source, DebtSource::TodoMarker)
}

/// The sources that are **off** for every project: switched off, or needing a grant that is not
/// given (R128/F8). The one statement of *off* the producers read: an off source is not swept at
/// all — no sweep row, no item opened or closed — so a check switched back on is `unknown` until
/// it next runs, whatever settled while it was off.
#[must_use]
pub fn off_sources(switches: &[HealthCheckSwitch], granted: bool) -> Vec<DebtSource> {
    switches
        .iter()
        .filter(|s| !s.enabled || (needs_grant(s.check) && !granted))
        .map(|s| s.check)
        .collect()
}

/// Apply **only the entries the patch names**, following the seven existing settings' shape.
///
/// Turning a check off takes its `debt_sweep` row with it, and leaves every `debt_item` row
/// exactly where it was — identity, `first_seen_at` and all. That is what makes the check come
/// back as `unknown` rather than as `ok` or as a zero, and what keeps the toggle from ever paying
/// or reversing XP.
///
/// # Errors
/// Fails when the index cannot be written.
pub fn write_switches(
    tx: &Transaction<'_>,
    switches: &[HealthCheckSwitch],
) -> Result<(), IndexError> {
    for switch in switches {
        let key = switch_key(switch.check)?;
        tx.execute(
            "INSERT INTO app_meta (k, v) VALUES (?1, ?2)
             ON CONFLICT(k) DO UPDATE SET v = excluded.v",
            rusqlite::params![key, if switch.enabled { "1" } else { "0" }],
        )?;
        if !switch.enabled {
            let Ok(serde_json::Value::String(slug)) = serde_json::to_value(switch.check) else {
                return Err(IndexError::Sidecar(
                    "a generated enum did not serialise as a string".into(),
                ));
            };
            // The sweep row only, never an item: the app stops claiming an observation for this
            // source and forgets nothing the user earned.
            tx.execute("DELETE FROM debt_sweep WHERE source = ?1", [&slug])?;
        }
    }
    Ok(())
}
