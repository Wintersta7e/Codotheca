//! §30.11 — **the shelf projection: one producer, both surfaces, batched.**
//!
//! `HealthSummary` is the shelf's flattening of the reading: `unknownChecks` is `basis.unknown`,
//! `observedAt` is `basis.observedAt`, `scoredOpen` is the reading's, and **`unverified` is the
//! one quantity the reading does not carry** — the page reads it off `ProjectDetail.debt`'s items,
//! where `DebtItem.state` says so, and the shelf has no items, which is why R117 puts the count
//! here and nowhere else.
//!
//! **`ProjectRow.healthSummary` is non-nullable** (R116.2), so `state` is the single discriminator
//! on both surfaces and `suppressed` stops being indistinguishable from `absent` on the shelf.
//! §35.3's two-part test — *"present **and** its state is not `absent`"* — is **deleted rather
//! than codified**; that deletion is p3-35's to apply, and it is recorded here so p3-35's author
//! does not preserve it.
//!
//! **No grid surface renders a health count.** *Signal (light) on the grid; Workshop (material)
//! only on the project page.* The summary rides the shelf projection for §35's sort and §33's
//! layer computation — **consumption, not rendering**.
//!
//! **The projection is batched or it is a per-row query storm.** `load_project_rows` already
//! builds `locations_by_project` and `collections_by_project` as grouped maps before it walks the
//! statement; this joins them as a third, built from **two** grouped reads over `debt_item` and
//! `debt_sweep` plus one each over `project` and `location`.

use std::collections::BTreeMap;

use rusqlite::Connection;

use super::lifecycle::lifecycle_of;
use super::{
    item_counts_for, location_presences, parse_condition, project_facts, sweeps_for, PerProject,
    ProjectFacts, SharedInputs,
};
use crate::projects::ProjectsError;
use crate::protocol::{
    ConditionSignal, DebtSweepOutcome, HealthReading, HealthSummary, Presence, ProjectId,
    ProjectLifecycle,
};

/// One project's sweep rows, keyed by source slug.
type SweepsBySource = BTreeMap<String, (DebtSweepOutcome, i64)>;
/// One project's item counts, keyed by source slug: (scored open, unverified).
type CountsBySource = BTreeMap<String, (u32, u32)>;

/// The reading flattened for the shelf, beside the lifecycle verdict computed from it.
///
/// `unverified` is summed over the project's items rather than taken from the reading, which does
/// not carry it. It is `None` exactly when the rest are: a reading that says nothing was computed
/// says nothing about unverified items either.
fn flatten(reading: &HealthReading, unverified: u32) -> HealthSummary {
    let basis = reading.basis.as_ref();
    HealthSummary {
        state: reading.state,
        scored_open: reading.scored_open,
        unverified: basis.map(|_| unverified),
        unknown_checks: basis.map(|b| b.unknown),
        observed_at: basis.map(|b| b.observed_at),
    }
}

/// One project's summary and lifecycle, through the same producer the page uses.
///
/// # Errors
/// Fails when the index refuses a read or holds a value this build's schema does not declare.
pub fn summary_for(
    conn: &Connection,
    project: ProjectId,
) -> Result<(HealthSummary, ProjectLifecycle), ProjectsError> {
    let shared = SharedInputs::load(conn)?;
    let facts = project_facts(conn, project)?;
    let counts = item_counts_for(conn, project)?;
    let per_project = PerProject {
        facts,
        locations: location_presences(conn, project)?,
        sweeps: sweeps_for(conn, project)?,
        counts: counts.clone(),
        refstate_observed: super::any_refstate_observed(conn, project)?,
        condition_signal: super::condition_signal_of(conn, project)?,
        user_na: super::stored_user_na(conn, project)?,
    };
    project_summary(&shared, &per_project, &counts)
}

/// Every project's summary and lifecycle, in a **bounded** number of statements: four grouped
/// reads for the whole library, then a `BTreeMap` lookup per row.
///
/// # Errors
/// Fails when the index refuses a read or holds a value this build's schema does not declare.
pub fn summaries_for_all(
    conn: &Connection,
) -> Result<BTreeMap<i64, (HealthSummary, ProjectLifecycle)>, ProjectsError> {
    let shared = SharedInputs::load(conn)?;
    let facts = all_project_facts(conn)?;
    let presences = all_location_presences(conn)?;
    let sweeps = all_sweeps(conn)?;
    let counts = all_item_counts(conn)?;
    let observed = all_refstate_observed(conn)?;
    let rulings =
        crate::completion::inputs::all_stored_user_na(conn).map_err(ProjectsError::Index)?;

    let mut out = BTreeMap::new();
    for (id, (row_facts, signal)) in facts {
        let per_counts = counts.get(&id).cloned().unwrap_or_default();
        let per_project = PerProject {
            facts: row_facts,
            locations: presences.get(&id).cloned().unwrap_or_default(),
            sweeps: sweeps.get(&id).cloned().unwrap_or_default(),
            counts: per_counts.clone(),
            refstate_observed: observed.contains(&id),
            condition_signal: signal,
            user_na: rulings.get(&id).copied().unwrap_or([None; 10]),
        };
        out.insert(id, project_summary(&shared, &per_project, &per_counts)?);
    }
    Ok(out)
}

fn project_summary(
    shared: &SharedInputs,
    project: &PerProject,
    counts: &CountsBySource,
) -> Result<(HealthSummary, ProjectLifecycle), ProjectsError> {
    let (reading, hidden) = super::reading_from(shared, project)?;
    // Over the same items the page lists: a set-aside check's items have left the list, so they
    // leave this count with them, as they leave `scoredOpen`.
    let uncounted: Vec<String> = hidden
        .into_iter()
        .map(super::slug_of)
        .collect::<Result<_, _>>()?;
    let unverified = counts
        .iter()
        .filter(|(source, _)| !uncounted.contains(source))
        .map(|(_, (_, unverified))| *unverified)
        .sum();
    let lifecycle = lifecycle_of(
        &reading,
        project.facts.is_archived,
        project.condition_signal,
    );
    Ok((flatten(&reading, unverified), lifecycle))
}

fn all_project_facts(
    conn: &Connection,
) -> Result<BTreeMap<i64, (ProjectFacts, Option<ConditionSignal>)>, ProjectsError> {
    let mut stmt = conn.prepare(
        "SELECT id, is_reference, authored_by_user, error_kind, acknowledged_at, is_archived,
                condition_signal, archetype
           FROM project",
    )?;
    let mut rows = stmt.query([])?;
    let mut out = BTreeMap::new();
    while let Some(r) = rows.next()? {
        let id: i64 = r.get(0)?;
        let facts = ProjectFacts {
            is_reference: Some(r.get::<_, i64>(1)? != 0),
            authored_by_user: r.get::<_, Option<i64>>(2)?.map(|v| v != 0),
            error_kind: r.get(3)?,
            acknowledged_at: r.get(4)?,
            is_archived: r.get::<_, i64>(5)? != 0,
            archetype: r.get(7)?,
        };
        out.insert(id, (facts, parse_condition(r.get(6)?)?));
    }
    Ok(out)
}

fn all_location_presences(
    conn: &Connection,
) -> Result<BTreeMap<i64, Vec<Presence>>, ProjectsError> {
    let mut stmt = conn.prepare("SELECT project_id, presence FROM location")?;
    let mut rows = stmt.query([])?;
    let mut out: BTreeMap<i64, Vec<Presence>> = BTreeMap::new();
    while let Some(r) = rows.next()? {
        let raw: String = r.get(1)?;
        let presence: Presence = serde_json::from_value(serde_json::Value::String(raw.clone()))
            .map_err(|_| ProjectsError::BadColumn {
                column: "location.presence",
                value: raw,
            })?;
        out.entry(r.get(0)?).or_default().push(presence);
    }
    Ok(out)
}

fn all_sweeps(conn: &Connection) -> Result<BTreeMap<i64, SweepsBySource>, ProjectsError> {
    let mut stmt =
        conn.prepare("SELECT project_id, source, outcome, observed_at FROM debt_sweep")?;
    let mut rows = stmt.query([])?;
    let mut out: BTreeMap<i64, SweepsBySource> = BTreeMap::new();
    while let Some(r) = rows.next()? {
        let raw: String = r.get(2)?;
        let outcome: DebtSweepOutcome =
            serde_json::from_value(serde_json::Value::String(raw.clone())).map_err(|_| {
                ProjectsError::BadColumn {
                    column: "debt_sweep.outcome",
                    value: raw,
                }
            })?;
        out.entry(r.get(0)?)
            .or_default()
            .insert(r.get(1)?, (outcome, r.get(3)?));
    }
    Ok(out)
}

fn all_item_counts(conn: &Connection) -> Result<BTreeMap<i64, CountsBySource>, ProjectsError> {
    let mut stmt = conn.prepare(
        "SELECT project_id, source,
                sum(state = 'open' AND scoring = 'scored'),
                sum(state = 'unverified')
           FROM debt_item GROUP BY project_id, source",
    )?;
    let mut rows = stmt.query([])?;
    let mut out: BTreeMap<i64, CountsBySource> = BTreeMap::new();
    while let Some(r) = rows.next()? {
        let open = u32::try_from(r.get::<_, i64>(2)?).unwrap_or(u32::MAX);
        let unverified = u32::try_from(r.get::<_, i64>(3)?).unwrap_or(u32::MAX);
        out.entry(r.get(0)?)
            .or_default()
            .insert(r.get(1)?, (open, unverified));
    }
    Ok(out)
}

fn all_refstate_observed(conn: &Connection) -> Result<Vec<i64>, ProjectsError> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT project_id FROM location
          WHERE refstate_observed_at IS NOT NULL OR worktree_observed_at IS NOT NULL",
    )?;
    let mut rows = stmt.query([])?;
    let mut out = Vec::new();
    while let Some(r) = rows.next()? {
        out.push(r.get(0)?);
    }
    Ok(out)
}
