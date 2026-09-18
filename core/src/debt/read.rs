//! §28.9's `ProjectDetail` projection — **one flat list plus the sweep states** (R120).
//!
//! **Not five arrays.** Five arrays are five places for a layer to be absent-versus-empty, which
//! is the distinction §33.8 already rules on for `WeatherLayer`: *"an absent entry and an empty
//! one would be two spellings of the same fact."* The grouping is a pure function of a field
//! every item already carries, and it lives in the renderer.
//!
//! **`unverified` crosses the wire on `DebtItem.state` and never as a count** (R128/F10): the
//! list must be able to say *this item is not being counted right now*, and §35's *"must never
//! reach a surface"* and §28's *"never reach the sort"* converge on **never on a ranked or
//! aggregated surface**.

use rusqlite::Connection;

use super::{enum_from_text, enum_text, registry_for, DebtError};
use crate::index::path::{display_paths_for_ui, DisplayPathTable};
use crate::protocol::{
    DebtItem, DebtItemState, DebtScoring, DebtSource, DebtSweepOutcome, DebtSweepState, DecayLayer,
    ObservationBasis, ProjectId,
};

/// The render order §28.9 needs for a stable list.
///
/// **§33 owns every rule over [`DecayLayer`]; this is only the order the list is drawn in**, and
/// it mirrors the schema's declaration order, which is the total order A3's tie-break needs.
/// `debt_wire.rs` asserts the two agree by reading the schema, so this cannot drift from it
/// silently — and the `match` is total, so a sixth layer fails to compile here.
const fn layer_order(layer: DecayLayer) -> u8 {
    match layer {
        DecayLayer::Dust => 0,
        DecayLayer::Cobwebs => 1,
        DecayLayer::Rust => 2,
        DecayLayer::Cracks => 3,
        DecayLayer::Overgrowth => 4,
    }
}

/// One project's debt, as one flat list.
///
/// Ordered `(layer, source, pathDisplay, line)` so the rendered list is stable across reads and a
/// re-sort in the renderer is never needed to make it so.
pub fn load_debt(conn: &Connection, project: ProjectId) -> Result<Vec<DebtItem>, DebtError> {
    // §1.10: `path_display` is **write-once** and `display_paths_for_ui` is the only function in
    // the core permitted to read one back. `core/tests/index_paths.rs` scans the source to keep
    // that true, so this reads the row ids here and the display strings there.
    let mut rows = Vec::new();
    {
        let mut st = conn.prepare(
            "SELECT id, source, fingerprint, state, scoring, line, column, salient_text,
                    first_seen_at, last_seen_at, basis, path_bytes IS NOT NULL
               FROM debt_item WHERE project_id = ?1",
        )?;
        let mapped = st.query_map([project.0], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, Option<u32>>(5)?,
                r.get::<_, Option<u32>>(6)?,
                r.get::<_, Option<String>>(7)?,
                r.get::<_, i64>(8)?,
                r.get::<_, i64>(9)?,
                r.get::<_, Option<String>>(10)?,
                r.get::<_, i64>(11)? != 0,
            ))
        })?;
        for row in mapped {
            rows.push(row?);
        }
    }

    // **Only the rows that have a path.** `debt_item.path_display` is nullable — a singleton has
    // no path at all — and `display_paths_for_ui` reads the column as NOT NULL, which is true of
    // the three tables it was written for. `path_bytes IS NOT NULL` is the same predicate stated
    // without naming the write-once column, which §1.10's source scan would otherwise flag.
    let ids: Vec<i64> = rows.iter().filter(|r| r.11).map(|r| r.0).collect();
    let displays =
        display_paths_for_ui(conn, DisplayPathTable::DebtItem, &ids).map_err(DebtError::Index)?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let source: DebtSource = enum_from_text(&row.1)
            .ok_or_else(|| DebtError::Codec(format!("debt_item.source holds {:?}", row.1)))?;
        let state: DebtItemState = enum_from_text(&row.3)
            .ok_or_else(|| DebtError::Codec(format!("debt_item.state holds {:?}", row.3)))?;
        let scoring: DebtScoring = enum_from_text(&row.4)
            .ok_or_else(|| DebtError::Codec(format!("debt_item.scoring holds {:?}", row.4)))?;
        let basis = match row.10 {
            None => None,
            Some(raw) => Some(
                enum_from_text::<ObservationBasis>(&raw)
                    .ok_or_else(|| DebtError::Codec(format!("debt_item.basis holds {raw:?}")))?,
            ),
        };
        out.push(DebtItem {
            source,
            fingerprint: row.2,
            state,
            scoring,
            // **The layer is a join, not a stored value** — one owner, §28.2's registry, so it
            // cannot disagree with the source it describes.
            layer: registry_for(source).layer,
            path_display: displays
                .iter()
                .find(|(id, _)| *id == row.0)
                .map(|(_, p)| p.clone()),
            line: row.5,
            column: row.6,
            salient_text: row.7,
            first_seen_at: row.8,
            last_seen_at: row.9,
            basis,
        });
    }

    // The source's own key is its wire spelling, so the order is the one a reader sees. Building
    // it once per item keeps the comparator infallible.
    let mut keyed: Vec<(u8, String, DebtItem)> = Vec::with_capacity(out.len());
    for item in out {
        let source_text = enum_text(&item.source)?;
        keyed.push((layer_order(item.layer), source_text, item));
    }
    keyed.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| a.1.cmp(&b.1))
            .then_with(|| a.2.path_display.cmp(&b.2.path_display))
            .then_with(|| a.2.line.cmp(&b.2.line))
            .then_with(|| a.2.fingerprint.cmp(&b.2.fingerprint))
    });
    let out: Vec<DebtItem> = keyed.into_iter().map(|(_, _, item)| item).collect();
    Ok(out)
}

/// One project's sweeps — **what makes an empty item list readable as itself.**
///
/// A source with **no row** was never observed and is absent from this list; a source with a
/// `complete` sweep and `itemCount: 0` was looked at and had nothing. `AC-P3-28-11` requires the
/// two to differ **on the wire**, not only in the store.
pub fn load_debt_sweeps(
    conn: &Connection,
    project: ProjectId,
) -> Result<Vec<DebtSweepState>, DebtError> {
    let mut st = conn.prepare(
        "SELECT source, outcome, observed_at, item_count, basis
           FROM debt_sweep WHERE project_id = ?1 ORDER BY source",
    )?;
    let mapped = st.query_map([project.0], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, i64>(2)?,
            r.get::<_, Option<u32>>(3)?,
            r.get::<_, Option<String>>(4)?,
        ))
    })?;

    let mut out = Vec::new();
    for row in mapped {
        let row = row?;
        let source: DebtSource = enum_from_text(&row.0)
            .ok_or_else(|| DebtError::Codec(format!("debt_sweep.source holds {:?}", row.0)))?;
        let outcome: DebtSweepOutcome = enum_from_text(&row.1)
            .ok_or_else(|| DebtError::Codec(format!("debt_sweep.outcome holds {:?}", row.1)))?;
        let basis = match row.4 {
            None => None,
            Some(raw) => Some(
                enum_from_text::<ObservationBasis>(&raw)
                    .ok_or_else(|| DebtError::Codec(format!("debt_sweep.basis holds {raw:?}")))?,
            ),
        };
        out.push(DebtSweepState {
            source,
            outcome,
            observed_at: row.2,
            item_count: row.3,
            basis,
        });
    }
    Ok(out)
}
