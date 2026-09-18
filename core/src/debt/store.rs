//! §28.3 — the item store: open, refresh, close, reap.

use super::identity::DebtKey;
use crate::protocol::{DebtItemState, DebtScoring, LocationId, ObservationBasis};

/// One `debt_item` row as read back.
///
/// `layer` is deliberately **not** here and is not a column: it is a property of the item's
/// source, read from §28.2's registry, so the wire's `DebtItem.layer` is a join and not a stored
/// value that could drift from the source it describes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredItem {
    pub id: i64,
    pub key: DebtKey,
    pub state: DebtItemState,
    /// Set from the source's registry row and **overridable per item by the producer** — an
    /// advisory is `scored` when a fix is available and `shown_only` when one is not.
    pub scoring: DebtScoring,
    /// The anchor. **READ, not diagnostic**: [`super::sweep::comparable`] compares it against the
    /// sweep's location before any closure (§28.3 rule 1).
    pub last_seen_location_id: Option<LocationId>,
    pub basis: Option<ObservationBasis>,
    pub path_display: Option<String>,
    pub line: Option<u32>,
    pub column: Option<u32>,
    pub salient_text: Option<String>,
    pub first_seen_at: i64,
    pub last_seen_at: i64,
}
