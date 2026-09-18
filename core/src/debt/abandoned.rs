//! `abandoned_with_debt` — **the conjunct that must not satisfy itself.**
//!
//! A2 lights `cobwebs` when the project is in the `abandoned` band **and** it has at least one
//! open debt item. **The item this source produces is itself a debt item**, so a predicate that
//! counted it would be self-satisfying: the item could never close, and **no abandoned project
//! could ever reach Done** — which is exactly the proof A2 was made to preserve, re-broken one
//! level down.
//!
//! ```text
//! lit  ⟺  condition_signal = 'abandoned'
//!          ∧ ∃ item : state = 'open' ∧ scoring = 'scored' ∧ source ≠ 'abandoned_with_debt'
//! ```
//!
//! **That form and no other.** §28.2's unamended sentence counts open items *at any `scoring`*
//! and **R122 narrows it**: an unfixable advisory is not outstanding work, and saying so is a
//! false accusation of the kind A7 forbids. It is also what keeps A2's proof intact once Done
//! stays on `scoredOpen`.
//!
//! **§33 carries a pointer to this conjunct and restates neither half** (R130/F4); if this form
//! and §33's ever differ, this one is normative.
//!
//! **Closability is carried by the other conjunct.** The item closes when `condition_signal`
//! leaves `abandoned` (§5.4a owns the band) or when the last other open **scored** item closes,
//! and both are acts. **That is what makes this source pass §28.3's no-elapsed-time test** —
//! because of its second conjunct, not in spite of it.

use rusqlite::Transaction;

use super::singletons::{ArmReading, SingletonArm};
use super::store::ObservedItem;
use super::{registry_for, DebtError};
use crate::protocol::{DebtSource, ProjectId};

/// The excluded-source, scored-only predicate — **the single owner of this form.**
///
/// # Errors
/// Fails when SQLite refuses a read.
pub fn abandoned_conjunct(tx: &Transaction<'_>, project: ProjectId) -> Result<bool, DebtError> {
    let abandoned: bool = tx.query_row(
        "SELECT condition_signal IS 'abandoned' FROM project WHERE id = ?1",
        [project.0],
        |r| r.get(0),
    )?;
    if !abandoned {
        return Ok(false);
    }
    let others: i64 = tx.query_row(
        "SELECT count(*) FROM debt_item
          WHERE project_id = ?1
            AND state = 'open'
            AND scoring = 'scored'
            AND source <> 'abandoned_with_debt'",
        [project.0],
        |r| r.get(0),
    )?;
    Ok(others > 0)
}

/// The seventh [`SingletonArm`].
///
/// Its `basis` is **NULL** and its `location_id` is **NULL** — it is derived from other stored
/// observations and observes nothing itself, so it has no basis to carry and inventing one would
/// be a lie. Its scoring is `shown_only` by registry default: the item closes when the project
/// stops being abandoned, and paying XP for that would be paying for **activity**. The layer
/// still lights, because lighting is what `shown_only` keeps.
#[derive(Debug)]
pub struct AbandonedArm;

impl SingletonArm for AbandonedArm {
    fn source(&self) -> DebtSource {
        DebtSource::AbandonedWithDebt
    }

    fn observe(&self, tx: &Transaction<'_>, project: ProjectId) -> Result<ArmReading, DebtError> {
        let row = *registry_for(DebtSource::AbandonedWithDebt);
        if abandoned_conjunct(tx, project)? {
            Ok(ArmReading::PredicateFalse(ObservedItem {
                key: super::identity::DebtKey::singleton("", DebtSource::AbandonedWithDebt),
                scoring: row.default_scoring,
                location: None,
                basis: None,
                path_bytes: None,
                path_display: None,
                line: None,
                column: None,
                salient_text: None,
            }))
        } else {
            // **Never `Unobservable`.** Both conjuncts are stored facts this transaction can
            // read, so *the predicate holds* is always sayable — and an `unobservable` here would
            // leave the item open for ever on a project that stopped being abandoned.
            Ok(ArmReading::PredicateTrue)
        }
    }
}
