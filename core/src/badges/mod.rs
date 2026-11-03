//! §40's badges: eight fixed predicates, each latched once in `badge_award`, paying nothing and
//! unlocking nothing.
//!
//! The table's readers and writers are closed (§40.7): this module (evaluate, list, `due`, mark
//! landed), the sidecar registry in `index::sidecar` (export and restore, through
//! [`store::export_section`] and [`store::restore_section`]) and the merge's repoint of
//! `project_id`. Nothing else reads the table — that is what *unlocks nothing* means in code.
//! The schema holds the latch itself: an award is never deleted, never re-inserted and never
//! rewritten, and only `project_id` and a single landing ever move.
//!
//! §40.4's constants are declared here and nowhere else. Necromancer's gap is not among them: it
//! is §38's dormancy constant, read from the ledger.

use crate::index::IndexError;
use crate::protocol::BadgeKind;

pub mod evaluate;
pub mod evidence;
pub(crate) mod population;
pub mod store;

/// Comeback: two consecutive commit-days of the whole library with `d2 − d1` at least this.
pub const COMEBACK_GAP_DAYS: i64 = 30;
/// Polyglot: distinct primary languages in one calendar year.
pub const POLYGLOT_LANGUAGES: usize = 5;
/// Marathon: `session.credited_seconds` of one session.
pub const MARATHON_CREDITED_SECONDS: i64 = 14_400;
/// Curator: projects at §31's 100% at once.
pub const CURATOR_PROJECTS: usize = 10;
/// Undertaker: distinct projects carrying a `clean_archive` row.
pub const UNDERTAKER_PROJECTS: usize = 5;
/// Firefighter: distinct `(project, local date)` pairs of a same-day repair.
pub const FIREFIGHTER_PAIRS: usize = 10;
/// Lifeline: consecutive clear local dates.
pub const LIFELINE_DAYS: i64 = 30;
/// The fortnight over which §40.8's retro awards land.
pub const TRICKLE_DAYS: i64 = 14;

/// §40.8: the order awards land in when several are due.
pub const LANDING_ORDER: [BadgeKind; 8] = [
    BadgeKind::Necromancer,
    BadgeKind::Polyglot,
    BadgeKind::Comeback,
    BadgeKind::Curator,
    BadgeKind::Marathon,
    BadgeKind::Undertaker,
    BadgeKind::Firefighter,
    BadgeKind::Lifeline,
];

/// The TEXT `badge_award.kind` stores for `kind`: the generated enum's own serde rename, so the
/// column's vocabulary is the schema's and never a second table.
///
/// # Panics
/// Never for a generated unit enum, which always serialises as its rename.
#[must_use]
pub fn kind_text(kind: BadgeKind) -> String {
    match serde_json::to_value(kind) {
        Ok(serde_json::Value::String(text)) => text,
        other => unreachable!("a BadgeKind did not serialise as a string: {other:?}"),
    }
}

/// Why a badge operation could not complete.
#[derive(Debug, thiserror::Error)]
pub enum BadgeError {
    /// The index refused a read or a write.
    #[error(transparent)]
    Index(#[from] IndexError),
    /// The kind asked about holds no award.
    #[error("no {0:?} award is held")]
    NotAwarded(BadgeKind),
    /// A stored evidence document is not the generated `BadgeEvidence`.
    #[error("badge evidence: {0}")]
    Evidence(String),
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
mod tests {
    use super::*;

    #[test]
    fn the_landing_order_is_every_kind_once() {
        let mut order: Vec<String> = LANDING_ORDER.iter().map(|k| kind_text(*k)).collect();
        let mut all: Vec<String> = BadgeKind::ALL.iter().map(|k| kind_text(*k)).collect();
        order.sort();
        all.sort();
        eprintln!("{} kinds in the landing order", order.len());
        assert_eq!(order.len(), 8);
        assert_eq!(order, all);
    }
}
