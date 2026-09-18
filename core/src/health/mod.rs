//! §30 — a project's health as a **reading**.
//!
//! **Nothing about the reading is stored.** §30.11 rules *migration: none* and this module adds
//! none: the reading is derived at read time on `project_presence`'s precedent
//! (`crate::scan::presence::project_presence` — *"`project` has no presence column: this is
//! derived at read time, which is why the function is pure and takes a slice"*), the per-check
//! switches ride `app_meta`, which is key/value, and `acknowledged_at` has been a declared column
//! since `0001_meta_and_projects.sql:102`.
//!
//! The only writes this module adds anywhere are the `acknowledged_at` stamp and the switch keys.
//!
//! **The freeze is applied once, here, and no consumer re-applies it.** §30 freezes what it
//! produces — the reading, the per-check outcomes, and **the open-item set §33 and §35 consume**
//! — so a `frozen` project is handed its last computed item set rather than the current
//! unobservable one. `HealthState` on the wire says which reading a surface is looking at and how
//! old it is; **it is not a flag a consumer acts on**, and neither §33 nor §35 branches on it or
//! holds a second freeze gate.
//!
//! **The reading has no history and none is fabricated.** When an enrolled project first produces
//! a reading, the reading simply begins: no backfill, no retroactive `health_delta`, no *"you had
//! 47 items for two years."* §34's `health_delta` is the only history.

pub mod acknowledge;
pub mod enrolment;
pub mod lifecycle;
pub mod outcome;
pub mod reason;
pub mod state;
pub mod switches;

use std::collections::BTreeMap;

use rusqlite::Connection;

use crate::projects::ProjectsError;
use crate::protocol::{
    DebtItem, DebtSource, DebtSweepOutcome, HealthReading, HealthState, Presence, ProjectId,
};

use outcome::{basis_over, CheckObservation, SweepFacts, SwitchState};
use reason::{check_for, GrantState};
use state::{health_state, StateInputs};

/// What one project's row and copies say, before any check is looked at.
#[derive(Debug, Clone)]
struct ProjectFacts {
    is_reference: Option<bool>,
    authored_by_user: Option<bool>,
    error_kind: Option<String>,
    acknowledged_at: Option<i64>,
    is_archived: bool,
}

/// One project's reading, **and the item set the frozen case hands down**, from one call — so no
/// consumer assembles a second.
///
/// `scoredOpen` and `basis` take the `completion_lit` enforcement shape (§1.10): NULL-able with
/// **no default**, and the writer is forbidden to write `0` for unknown. Nothing below reaches for
/// `unwrap_or(0)` or `COALESCE`; `crate::derive::persist` is the shape this copies — *"an `Option`
/// that is `None` becomes SQL NULL, and nothing below coalesces."*
///
/// **`checks` is empty for `absent` and `suppressed`**, and an empty array is the state saying
/// nothing was computed — never a count of zero checks.
///
/// # Errors
/// Fails when the index refuses a read or holds a value this build's schema does not declare.
pub fn read_for_project(
    conn: &Connection,
    project: ProjectId,
    _now: i64,
) -> Result<(HealthReading, Vec<DebtItem>), ProjectsError> {
    let facts = project_facts(conn, project)?;
    let locations = location_presences(conn, project)?;
    let sweeps = sweeps_for(conn, project)?;
    let counts = item_counts_for(conn, project)?;

    // A reading existed iff something was ever swept: §30 stores none, so the sweep record is the
    // only honest witness that one was computed. It is also gate 3's *ever observed*, widened by
    // the ref-state clock, because a project can have been read without any source being swept.
    let ever_swept = !sweeps.is_empty();
    let ever_observed = ever_swept || any_refstate_observed(conn, project)?;

    let inputs = StateInputs {
        is_reference: facts.is_reference,
        authored_by_user: facts.authored_by_user,
        error_kind: facts.error_kind.clone(),
        ever_observed,
        locations: locations.clone(),
        enrolled: enrolment::is_enrolled(facts.acknowledged_at),
        is_archived: facts.is_archived,
        prior_reading: ever_swept,
    };
    let state = health_state(&inputs);

    // §30.1: the states that say *no reading* carry no checks, no count and no basis — and that
    // is the whole of what they carry. A zero here is the invariant's own counterexample.
    if state == HealthState::Absent || state == HealthState::Suppressed {
        return Ok((
            HealthReading {
                state,
                scored_open: None,
                basis: None,
                checks: Vec::new(),
            },
            Vec::new(),
        ));
    }

    let switches = switches::read_switches(conn).map_err(ProjectsError::Index)?;
    let granted =
        crate::surfaces::settings::content_scan_enabled(conn).map_err(ProjectsError::Index)?;
    let has_account = account_count(conn)? > 0;
    let anchor = crate::scan::presence::project_presence(&locations);

    let mut entries = Vec::with_capacity(switches.len());
    let mut scored_open = 0u32;
    for switch in &switches {
        let (sweep_outcome, observed_at) = sweeps
            .get(&slug_of(switch.check)?)
            .copied()
            .map_or((None, None), |(o, at)| (Some(o), Some(at)));
        let (open, unverified) = counts
            .get(&slug_of(switch.check)?)
            .copied()
            .unwrap_or((0, 0));
        let facts = SweepFacts {
            outcome: sweep_outcome,
            scored_open: open,
            unverified,
            observed_at,
        };
        let switch_state = SwitchState {
            enabled: switch.enabled,
            // R128/F8: `todo_marker` is the one source whose evidence needs §29's grant, and an
            // ungranted scan makes it `off` rather than a check waiting for a sweep that never
            // comes.
            grant_missing: switch.check == DebtSource::TodoMarker && !granted,
            // §31's archetype-proposed mechanism lands with p3-31 (wave 4) and owns this input.
            // `false` here is *nothing has proposed it*, which is true of every project today.
            not_applicable: false,
        };
        let grant = GrantState {
            // §20's forge account. `ci_red`'s evidence is a forge CI status and there is no way
            // to read one without an account.
            account_missing: switch.check == DebtSource::CiRed && !has_account,
            // §32 owns when an advisory value is awaiting readback and lands in wave 4 beside
            // this one; until its sync surface exists nothing here can honestly claim a value is
            // in flight, and claiming it would be the invented reason §30.3 forbids.
            awaiting_sync: false,
        };
        let check = check_for(switch.check, &facts, &switch_state, anchor, &grant);
        scored_open += open;
        entries.push(CheckObservation { check, observed_at });
    }

    let basis = basis_over(&entries);
    Ok((
        HealthReading {
            state,
            // **`scoredOpen` is `Some` only when `basis` is.** Without `unknown` beside it a `0`
            // cannot be read as *nothing open* (R117's gate) and is the bare zero §30.1 exists to
            // prevent, so the pair is null together or present together.
            scored_open: basis.as_ref().map(|_| scored_open),
            basis,
            checks: entries.into_iter().map(|e| e.check).collect(),
        },
        // The item set, from the same call. A `frozen` project is handed the last computed set —
        // which is the stored one, because nothing recomputes a set it cannot observe.
        crate::debt::read::load_debt(conn, project).map_err(debt_error)?,
    ))
}

fn debt_error(error: crate::debt::DebtError) -> ProjectsError {
    match error {
        crate::debt::DebtError::Index(inner) => ProjectsError::Index(inner),
        crate::debt::DebtError::Codec(detail) => ProjectsError::BadColumn {
            column: "debt_item",
            value: detail,
        },
    }
}

fn slug_of(source: DebtSource) -> Result<String, ProjectsError> {
    match serde_json::to_value(source) {
        Ok(serde_json::Value::String(slug)) => Ok(slug),
        _ => Err(ProjectsError::BadColumn {
            column: "debt_source",
            value: format!("{source:?}"),
        }),
    }
}

fn project_facts(conn: &Connection, project: ProjectId) -> Result<ProjectFacts, ProjectsError> {
    conn.query_row(
        "SELECT is_reference, authored_by_user, error_kind, acknowledged_at, is_archived
           FROM project WHERE id = ?1",
        [project.0],
        |r| {
            Ok(ProjectFacts {
                is_reference: Some(r.get::<_, i64>(0)? != 0),
                // **`None` is *authorship has not run*, and gate 2 is what it exists for.**
                authored_by_user: r.get::<_, Option<i64>>(1)?.map(|v| v != 0),
                error_kind: r.get(2)?,
                acknowledged_at: r.get(3)?,
                is_archived: r.get::<_, i64>(4)? != 0,
            })
        },
    )
    .map_err(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => ProjectsError::UnknownProject(project.0),
        other => ProjectsError::Sqlite(other),
    })
}

/// One entry per copy, which is what `project_presence` is total over. Health does not roll it up
/// itself — that would be the second offline predicate.
fn location_presences(
    conn: &Connection,
    project: ProjectId,
) -> Result<Vec<Presence>, ProjectsError> {
    let mut stmt = conn.prepare("SELECT presence FROM location WHERE project_id = ?1")?;
    let mut rows = stmt.query([project.0])?;
    let mut out = Vec::new();
    while let Some(r) = rows.next()? {
        let raw: String = r.get(0)?;
        let presence: Presence = serde_json::from_value(serde_json::Value::String(raw.clone()))
            .map_err(|_| ProjectsError::BadColumn {
                column: "location.presence",
                value: raw,
            })?;
        out.push(presence);
    }
    Ok(out)
}

fn any_refstate_observed(conn: &Connection, project: ProjectId) -> Result<bool, ProjectsError> {
    let n: i64 = conn.query_row(
        "SELECT count(*) FROM location
          WHERE project_id = ?1
            AND (refstate_observed_at IS NOT NULL OR worktree_observed_at IS NOT NULL)",
        [project.0],
        |r| r.get(0),
    )?;
    Ok(n > 0)
}

fn account_count(conn: &Connection) -> Result<i64, ProjectsError> {
    Ok(conn.query_row("SELECT count(*) FROM account", [], |r| r.get(0))?)
}

/// One grouped read over `debt_sweep`: source → (outcome, `observed_at`).
fn sweeps_for(
    conn: &Connection,
    project: ProjectId,
) -> Result<BTreeMap<String, (DebtSweepOutcome, i64)>, ProjectsError> {
    let mut stmt =
        conn.prepare("SELECT source, outcome, observed_at FROM debt_sweep WHERE project_id = ?1")?;
    let mut rows = stmt.query([project.0])?;
    let mut out = BTreeMap::new();
    while let Some(r) = rows.next()? {
        let source: String = r.get(0)?;
        let raw: String = r.get(1)?;
        let outcome: DebtSweepOutcome =
            serde_json::from_value(serde_json::Value::String(raw.clone())).map_err(|_| {
                ProjectsError::BadColumn {
                    column: "debt_sweep.outcome",
                    value: raw,
                }
            })?;
        out.insert(source, (outcome, r.get(2)?));
    }
    Ok(out)
}

/// The second grouped read: source → (scored open items, unverified items).
///
/// **`scored_open` counts items that are both `open` and `scored`** (A7). A `shown_only` item
/// renders, lights its layer, ranks nowhere, and is excluded from this count and from any XP
/// payout — so it is filtered here rather than subtracted later.
fn item_counts_for(
    conn: &Connection,
    project: ProjectId,
) -> Result<BTreeMap<String, (u32, u32)>, ProjectsError> {
    let mut stmt = conn.prepare(
        "SELECT source,
                sum(state = 'open' AND scoring = 'scored'),
                sum(state = 'unverified')
           FROM debt_item WHERE project_id = ?1 GROUP BY source",
    )?;
    let mut rows = stmt.query([project.0])?;
    let mut out = BTreeMap::new();
    while let Some(r) = rows.next()? {
        let source: String = r.get(0)?;
        let open = u32::try_from(r.get::<_, i64>(1)?).unwrap_or(u32::MAX);
        let unverified = u32::try_from(r.get::<_, i64>(2)?).unwrap_or(u32::MAX);
        out.insert(source, (open, unverified));
    }
    Ok(out)
}
