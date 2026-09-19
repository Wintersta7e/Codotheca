//! §8.2's `projects.list`. Sections and their coverage-carrying aggregates first, rows second.
//!
//! The result type is plan 02's generated `ProjectPage` and is never hand-declared here — R14
//! renamed plan 14's React component to `ProjectPageView` so this name stays the schema's.

use std::collections::{BTreeMap, BTreeSet};

use crate::art::compose::local_year;
use crate::projects::rows::{load_project_rows, scan_generation, LoadedRow};
use crate::projects::{ProjectsCtx, ProjectsError};
use crate::proto::dispatch::{parse_args, CommandFailure};
use crate::protocol::{
    EraAggregate, EraSection, HealthState, ProjectPage, ProjectRow, ProjectsListArgs, SortKey,
    Window,
};
use crate::query::ast::QueryTerm;
use crate::query::execute::{evaluate_query, ExecContext};
use crate::query::{parse_query, QueryAst};

pub const ERA_LIVE_DAYS: i64 = 7;
pub const ERA_MONTH_DAYS: i64 = 30;
pub const ERA_QUARTER_DAYS: i64 = 90;
/// Named year sections run from the cut year down to cut − 10; the tail is cut − 11 and earlier.
pub const ERA_NAMED_YEARS: i32 = 10;

const DAY: i64 = 86_400;

/// §8.1. Ids are stable strings — `view_state` persists collapse across relaunches and ordinal
/// indices would drift — and `era:tail` carries no year, so no stored state is orphaned by a
/// New Year.
#[must_use]
pub fn era_section_id_for(row: &ProjectRow, now: i64, tz_offset_min: i32) -> String {
    // §23.4: **tested first**, before both overrides. Order 98 alone does not achieve this —
    // `is_archived` is a *user* flag (§1.2), a user may archive a not-cloned project, and
    // `era:archived` at order 92 is an **interleaved** section whose header sums tracked bytes.
    // Filing a tile with no bytes and no `Play` among tiles that have both breaks the settled
    // *never interleaved* ruling. §23.1: `primary_location IS NULL` is the whole predicate, and
    // there is no second expression of it.
    if row.primary_location.is_none() {
        return "era:notcloned".to_owned();
    }
    if row.is_archived {
        return "era:archived".to_owned();
    }
    if row.is_submodule {
        return "era:submodules".to_owned();
    }
    let age_days = (now - row.last_touched_at) / DAY;
    if age_days <= ERA_LIVE_DAYS {
        return "era:live".to_owned();
    }
    if age_days <= ERA_MONTH_DAYS {
        return "era:month".to_owned();
    }
    if age_days <= ERA_QUARTER_DAYS {
        return "era:q".to_owned();
    }
    // Calendar years, never a rolling 365-day bucket wearing a calendar label (§8.1).
    let cut = local_year(now, tz_offset_min);
    let touched = local_year(row.last_touched_at, tz_offset_min);
    if touched >= cut {
        return "era:year".to_owned();
    }
    if touched >= cut - ERA_NAMED_YEARS {
        return format!("era:{touched}");
    }
    "era:tail".to_owned()
}

/// §8.1's order table. A year section sits at `10 + (cut − year)`, so the newest named year is
/// 11 and the oldest is 20; everything the table does not name sorts after them.
#[must_use]
pub fn era_section_order(id: &str, cut_against_year: i32) -> u32 {
    match id {
        "era:live" => 0,
        "era:month" => 1,
        "era:q" => 2,
        "era:year" => 3,
        "era:tail" => 90,
        "era:archived" => 92,
        "era:submodules" => 94,
        "era:notcloned" => 98,
        other => other
            .strip_prefix("era:")
            .and_then(|y| y.parse::<i32>().ok())
            .and_then(|year| u32::try_from(10 + (cut_against_year - year)).ok())
            .unwrap_or(99),
    }
}

fn year_of(id: &str) -> Option<u32> {
    id.strip_prefix("era:").and_then(|y| y.parse::<u32>().ok())
}

fn add_u32(total: u32, one: u32) -> u32 {
    total.saturating_add(one)
}

/// §8.2: `indexedCount` and `unchecked` are counts of **coverage** and are emitted always. A
/// fully covered section sends `indexedCount == count` and `unchecked: 0` rather than omitting
/// them, because the shell must not infer coverage from a missing field.
#[must_use]
pub fn aggregate_era(rows: &[&LoadedRow]) -> EraAggregate {
    let mut agg = EraAggregate {
        tracked_bytes: 0,
        indexed_count: 0,
        unpushed: 0,
        uncommitted: 0,
        interrupted: 0,
        unchecked: 0,
    };
    for row in rows {
        let r = &row.row;
        if let Some(bytes) = r.size_tracked_bytes {
            agg.tracked_bytes = agg.tracked_bytes.saturating_add(bytes);
            agg.indexed_count = add_u32(agg.indexed_count, 1);
        }
        if r.ahead.unwrap_or(0) > 0 {
            agg.unpushed = add_u32(agg.unpushed, 1);
        }
        if r.is_dirty == Some(true) {
            agg.uncommitted = add_u32(agg.uncommitted, 1);
        }
        if r.interrupted_op.is_some() {
            agg.interrupted = add_u32(agg.interrupted, 1);
        }
        // No J1 result: an absent flag line may only ever mean *observed, nothing to report*.
        //
        // §23.4: the counter is a coverage warning about **local git state**, so it counts a row
        // only when the project has at least one `location`. A not-cloned project has no local
        // git state to be uncovered about, and reporting it as `unchecked` would promise a check
        // that can never run.
        if r.primary_location.is_some() && r.refstate_observed_at.is_none() {
            agg.unchecked = add_u32(agg.unchecked, 1);
        }
    }
    agg
}

/// [p3] §35.3's membership rule and §35.2's ordering scalar, as **one** total function.
///
/// `Some(n)` is *this row carries a reading, and its count is n*; `None` is *tail*. Written once
/// it cannot drift into two rules, and the case §30.1 forbids the writer to produce — a `live`
/// reading with a NULL `scored_open` — resolves to the tail rather than to a panic or to a zero.
/// **A zero is a ranked value and never a tail value** (§35.3).
///
/// The match is exhaustive with **no wildcard**: a fifth `HealthState` cannot be added without
/// this function being revisited, which is the whole point of reading the state here.
///
/// It re-applies **no** exclusion of its own (§35.4). Reference, archived, Done, `frozen`,
/// `surface_suppressed`, `absent`, `shown_only`, `unverified` and not-cloned are nine cases and
/// one mechanism: §30's pipeline decides what the reading is, and this reads it. `is_reference`,
/// `is_archived`, `lifecycle` and the item list are never consulted.
#[must_use]
pub fn rank_of(row: &ProjectRow) -> Option<u32> {
    match row.health_summary.state {
        HealthState::Live | HealthState::Frozen => row.health_summary.scored_open,
        HealthState::Absent | HealthState::Suppressed => None,
    }
}

/// §8.0a's default order: most recently touched first, ties on the lowest id.
///
/// Extracted rather than restated: `last_touched` and `needs_attention` both end here, and a
/// fourth copy of one value is the R12 shape. `name` and `size` are **not** routed through it —
/// their tiebreaks are `id` alone and `id`-after-bytes, which are different values that happen to
/// share a term.
fn default_order(a: &LoadedRow, b: &LoadedRow) -> std::cmp::Ordering {
    b.row
        .last_touched_at
        .cmp(&a.row.last_touched_at)
        .then(a.row.id.0.cmp(&b.row.id.0))
}

pub fn sort_rows(rows: &mut [&LoadedRow], sort: SortKey) {
    match sort {
        SortKey::Name => rows.sort_by(|a, b| {
            a.row
                .name
                .to_lowercase()
                .cmp(&b.row.name.to_lowercase())
                .then(a.row.id.0.cmp(&b.row.id.0))
        }),
        // A row with no inventory sorts last. It is not a small repository; it is an unmeasured one.
        SortKey::Size => {
            rows.sort_by(
                |a, b| match (a.row.size_tracked_bytes, b.row.size_tracked_bytes) {
                    (None, None) => a.row.id.0.cmp(&b.row.id.0),
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (Some(x), Some(y)) => y.cmp(&x).then(a.row.id.0.cmp(&b.row.id.0)),
                },
            );
        }
        SortKey::LastTouched => rows.sort_by(|a, b| default_order(a, b)),
        // [p3] §35.3, in `Size`'s shape: a row with no reading sorts last. It is not a project
        // with nothing outstanding; it is one nobody has looked at.
        SortKey::NeedsAttention => {
            rows.sort_by(|a, b| match (rank_of(&a.row), rank_of(&b.row)) {
                (None, None) => default_order(a, b),
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (Some(_), None) => std::cmp::Ordering::Less,
                (Some(x), Some(y)) => y.cmp(&x).then_with(|| default_order(a, b)),
            });
        }
    }
}

/// FNV-1a over the ordered ids. A **cursor**, not a checksum: equality is all §8.2 asks of it,
/// and the renderer computes the same digest, so the two can be compared.
#[must_use]
pub fn order_key_of(ids: &[i64]) -> String {
    let mut hash: u32 = 0x811c_9dc5;
    for id in ids {
        // Low four bytes, little-endian. `try_from` rather than `as`, so no cast is silently
        // truncating a value the reader cannot see being truncated.
        let low = u32::try_from(id.unsigned_abs() & 0xffff_ffff).unwrap_or(0);
        for byte in low.to_le_bytes() {
            hash ^= u32::from(byte);
            hash = hash.wrapping_mul(0x0100_0193);
        }
    }
    format!("{hash:08x}")
}

#[must_use]
pub fn build_project_page(
    ctx: &ProjectsCtx<'_>,
    rows: &[LoadedRow],
    ast: &QueryAst,
    sort: SortKey,
    window: Option<Window>,
    generation: i64,
    exec: &ExecContext<'_>,
) -> ProjectPage {
    let mut ordered = evaluate_query(rows, ast, exec).rows;
    sort_rows(&mut ordered, sort);

    let cut_against_year = local_year(ctx.now, ctx.tz_offset_min);
    let mut buckets: BTreeMap<String, Vec<&LoadedRow>> = BTreeMap::new();
    let mut stamped: Vec<ProjectRow> = Vec::with_capacity(ordered.len());
    for row in &ordered {
        let id = era_section_id_for(&row.row, ctx.now, ctx.tz_offset_min);
        let mut wire = row.row.clone();
        wire.era_section_id.clone_from(&id);
        stamped.push(wire);
        buckets.entry(id).or_default().push(row);
    }

    let cut_u32 = u32::try_from(cut_against_year).unwrap_or(0);
    let mut sections: Vec<EraSection> = buckets
        .iter()
        .map(|(id, section_rows)| EraSection {
            id: id.clone(),
            order: era_section_order(id, cut_against_year),
            year: year_of(id),
            cut_against_year: cut_u32,
            count: u32::try_from(section_rows.len()).unwrap_or(u32::MAX),
            agg: aggregate_era(section_rows),
        })
        .collect();
    sections.sort_by_key(|s| s.order);

    let total = u32::try_from(stamped.len()).unwrap_or(u32::MAX);
    // §8.2: windowing is not LIMIT/OFFSET; the default window is the whole rendered set, which
    // is what the `projects` topic's snapshot needs when it calls this with no arguments.
    let asked = window.unwrap_or(Window { from: 0, to: total });
    let from = asked.from.min(total);
    let to = asked.to.clamp(from, total);
    let slice = stamped
        .get(from as usize..to as usize)
        .unwrap_or_default()
        .to_vec();

    ProjectPage {
        sections,
        rows: slice,
        window: Window { from, to },
        order_key: order_key_of(&stamped.iter().map(|r| r.id.0).collect::<Vec<_>>()),
        generation: u32::try_from(generation).unwrap_or(0),
    }
}

fn collection_ids_by_name(
    conn: &rusqlite::Connection,
) -> Result<BTreeMap<String, i64>, ProjectsError> {
    let mut stmt = conn.prepare("SELECT lower(name), id FROM collection")?;
    let mut out = BTreeMap::new();
    let mut rows = stmt.query([])?;
    while let Some(r) = rows.next()? {
        out.insert(r.get(0)?, r.get(1)?);
    }
    Ok(out)
}

/// §8.3's one round-tripping term. Only run when the query has a bare term of ≥ 3 characters.
fn commit_subject_hits(
    conn: &rusqlite::Connection,
    ast: &QueryAst,
) -> Result<Option<BTreeSet<i64>>, ProjectsError> {
    let Some(needle) = ast.terms.iter().find_map(|t| match t {
        QueryTerm::Bare { text, .. } if text.chars().count() >= 3 => Some(text),
        _ => None,
    }) else {
        return Ok(None);
    };
    let mut stmt = conn.prepare("SELECT project_id FROM fts_commits WHERE subjects LIKE ?1")?;
    let mut out = BTreeSet::new();
    let mut rows = stmt.query(rusqlite::params![format!("%{needle}%")])?;
    while let Some(r) = rows.next()? {
        out.insert(r.get(0)?);
    }
    Ok(Some(out))
}

/// `None` until first run has finished, which makes `is:new` unknown rather than false — the
/// row is absent before the pass that writes it, and that absence is not an error to report.
fn first_run_completed_at(conn: &rusqlite::Connection) -> Option<i64> {
    conn.query_row(
        "SELECT v FROM app_meta WHERE k = 'first_run_completed_at'",
        [],
        |r| r.get::<_, String>(0),
    )
    .ok()
    .and_then(|v| v.parse().ok())
}

pub fn handle(
    ctx: &ProjectsCtx<'_>,
    args: serde_json::Value,
) -> Result<serde_json::Value, CommandFailure> {
    let args: ProjectsListArgs = parse_args(args)?;
    let conn = ctx.index.conn();
    let rows = load_project_rows(ctx)?;
    let ast = parse_query(args.query.as_deref().unwrap_or(""));
    let names = collection_ids_by_name(conn)?;
    let hits = commit_subject_hits(conn, &ast)?;
    let exec = ExecContext {
        now: ctx.now,
        tz_offset_min: ctx.tz_offset_min,
        first_run_completed_at: first_run_completed_at(conn),
        collection_ids_by_name: &names,
        paths_are_case_sensitive: cfg!(not(windows)),
        commit_subject_hits: hits.as_ref(),
    };
    let page = build_project_page(
        ctx,
        &rows,
        &ast,
        args.sort.unwrap_or(SortKey::LastTouched),
        args.window,
        scan_generation(conn)?,
        &exec,
    );
    serde_json::to_value(page).map_err(|e| CommandFailure::internal(e.to_string()))
}
