//! The **only** file in this module that reads a table or names `crate::remote`.
//!
//! After R124 it gathers **answers, not evidence**: one reading per Group-A source from §28's
//! `debt_sweep` and `debt_item`, §29's one remaining tri-state, §32's verdict, and the four
//! already-shaped values the forge check needs. It reads no refstate fact the arms now own, and
//! `completion_evaluator.rs`'s source walk fails the build if one reappears.
//!
//! **`core/tests/remote_no_completion_writer.rs` stays green, untouched, and satisfying it shapes
//! this code** (§31.10). This file names a remote marker, so it may name no aggregate and no
//! condition: the values arrive already shaped from `crate::remote::facts`, and nothing here
//! judges one. A plan author is tempted to weaken that gate on day one; the gate is right and the
//! seam is right.

use rusqlite::{Connection, OptionalExtension as _};

use crate::advisories::verdict::verdict_for;
use crate::index::IndexError;
use crate::jobs::j7_markers::presence_for_project;
use crate::protocol::{CompletionCheck, DebtSource, DebtSweepOutcome, ProjectId};

use super::evaluate::{source_not_applicable, CompletionInputs, DepsReading, SingletonReading};

/// The gates `gather` answers `None` for, kept as a named value so the reason a project was not
/// scored is sayable rather than inferred from a bare `None` at the call site.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotScorable {
    /// §31.8: excluded for ever. NULL, `--tier-ref`, no checklist.
    Reference,
    /// **Completion runs after J1.5.** `is_reference = 1` is written only from a computed
    /// `authored_by_user` (§5.5), so scoring earlier lights ticks on a repository that is about
    /// to be excluded entirely.
    AuthorshipNotComputed,
    /// §23: not cloned. Every content check is structurally unevaluable, and scoring a repository
    /// nobody has cloned over the four forge-only checks would rank it above one that was.
    NotCloned,
    /// §31.8: **frozen, not recomputed.** Recomputing would read every content check as
    /// `unknown`, collapse `evaluable`, **and take a project's gold because a drive was
    /// unplugged**.
    Frozen,
    /// A reading that was **never** computed may not be frozen. Same answer, different fact, and
    /// the two are kept apart because one is a stored value standing and the other is nothing.
    NotComputed,
}

/// One project's shaped inputs, or why it is not scorable.
///
/// # Errors
/// Fails when SQLite cannot be read.
pub fn gather(
    conn: &Connection,
    project: ProjectId,
) -> Result<Result<CompletionInputs, NotScorable>, IndexError> {
    let Some(gates) = project_row(conn, project)? else {
        // A row that is not there, or one that was absorbed by a merge, is nothing to score.
        return Ok(Err(NotScorable::NotComputed));
    };
    if gates.is_reference {
        return Ok(Err(NotScorable::Reference));
    }
    if gates.authored_by_user.is_none() {
        return Ok(Err(NotScorable::AuthorshipNotComputed));
    }

    let Some(primary) = primary_copy(conn, project)? else {
        return Ok(Err(NotScorable::NotCloned));
    };
    if matches!(primary.presence.as_str(), "offline" | "missing") {
        return Ok(Err(if stored_row_count(conn, project)? > 0 {
            NotScorable::Frozen
        } else {
            NotScorable::NotComputed
        }));
    }

    // §29.5's four answers; only `ci` is §31's own, the other three reach §31 through §28's arms.
    let presence = presence_for_project(conn, project)?;

    let remote = crate::remote::facts::remote_completion_input(conn, project)?;
    let (connected, facts_state, description, topics) = match remote {
        Some(input) => (
            input.connected,
            input.facts_state,
            input.forge_description,
            input.topic_count,
        ),
        // No remote at all. The state is never read in this case — `description` is `na` — but a
        // value must be chosen, and *no account* is the one that cannot over-claim.
        None => (
            any_account(conn)?,
            crate::protocol::RemoteFactsState::NoAccount,
            None,
            0,
        ),
    };

    Ok(Ok(CompletionInputs {
        readme: singleton(conn, project, DebtSource::MissingReadme)?,
        license: singleton(conn, project, DebtSource::MissingLicense)?,
        tests: singleton(conn, project, DebtSource::MissingTests)?,
        ci_red: singleton(conn, project, DebtSource::CiRed)?,
        pushed: singleton(conn, project, DebtSource::UnpushedCommits)?,
        release: singleton(conn, project, DebtSource::NoRelease)?,
        has_ci: presence.map(|answers| answers.ci),
        // §31.1a's `remote` predicate, at `refs` basis: a copy whose refstate was never persisted
        // has not been looked at, and `NULL` there is *never observed* rather than *no remote*.
        remote_configured: primary.refstate_observed_at.map(|_| gates.has_remote),
        account_connected: connected,
        facts_state,
        has_remote: gates.has_remote,
        forge_description: description,
        topic_count: topics,
        deps: deps(conn, project, primary.now_basis)?,
        archetype: gates.archetype,
        user_na: stored_user_na(conn, project)?,
    }))
}

struct PrimaryCopy {
    presence: String,
    refstate_observed_at: Option<i64>,
    /// The clock §32's verdict is expiry-checked against. It is the project's own newest
    /// observation rather than the wall clock, so `gather` reads no clock of its own.
    now_basis: i64,
}

/// §5.1's primary copy — *most recently touched, present, native side preferred* — read through
/// the one implementation of that rule rather than a second ordering written here.
fn primary_copy(conn: &Connection, project: ProjectId) -> Result<Option<PrimaryCopy>, IndexError> {
    let locations =
        crate::projects::rows::locations_of(conn, project).map_err(|e| IndexError::Corrupt {
            detail: format!("locations_of: {e}"),
        })?;
    let Some(primary) = crate::projects::rows::pick_primary(&locations) else {
        return Ok(None);
    };
    let (presence, observed): (String, Option<i64>) = conn.query_row(
        "SELECT presence, refstate_observed_at FROM location WHERE id = ?1",
        [primary.id.0],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let newest: Option<i64> = conn
        .query_row(
            "SELECT max(updated_at) FROM project WHERE id = ?1",
            [project.0],
            |r| r.get(0),
        )
        .optional()?
        .flatten();
    Ok(Some(PrimaryCopy {
        presence,
        refstate_observed_at: observed,
        now_basis: newest.unwrap_or(0),
    }))
}

/// The four `project` columns `gather` reads, named rather than positional so a column that
/// moves names itself instead of shifting a tuple silently.
struct ProjectGates {
    is_reference: bool,
    authored_by_user: Option<i64>,
    archetype: Option<String>,
    has_remote: bool,
}

fn project_row(conn: &Connection, project: ProjectId) -> Result<Option<ProjectGates>, IndexError> {
    conn.query_row(
        "SELECT is_reference, authored_by_user, archetype, remote_key IS NOT NULL
           FROM project WHERE id = ?1 AND merged_into IS NULL",
        [project.0],
        |r| {
            Ok(ProjectGates {
                is_reference: r.get::<_, i64>(0)? != 0,
                authored_by_user: r.get(1)?,
                archetype: r.get(2)?,
                has_remote: r.get::<_, i64>(3)? != 0,
            })
        },
    )
    .optional()
    .map_err(IndexError::from)
}

fn any_account(conn: &Connection) -> Result<bool, IndexError> {
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM account", [], |r| r.get(0))?;
    Ok(count > 0)
}

fn stored_row_count(conn: &Connection, project: ProjectId) -> Result<i64, IndexError> {
    conn.query_row(
        "SELECT count(*) FROM project_check WHERE project_id = ?1",
        [project.0],
        |r| r.get(0),
    )
    .map_err(IndexError::from)
}

/// §28's stored answer for one source.
///
/// **A missing `debt_sweep` row is `None`, never a zero.** It is what makes a source that was
/// never observed render as *not run yet* rather than as *nothing wrong*.
fn singleton(
    conn: &Connection,
    project: ProjectId,
    source: DebtSource,
) -> Result<SingletonReading, IndexError> {
    let slug = enum_slug(&source);
    let outcome: Option<String> = conn
        .query_row(
            "SELECT outcome FROM debt_sweep WHERE project_id = ?1 AND source = ?2",
            rusqlite::params![project.0, slug],
            |r| r.get(0),
        )
        .optional()?;
    let open: i64 = conn.query_row(
        "SELECT count(*) FROM debt_item
          WHERE project_id = ?1 AND source = ?2 AND state = 'open' AND scoring = 'scored'",
        rusqlite::params![project.0, slug],
        |r| r.get(0),
    )?;
    Ok(SingletonReading {
        source,
        // A word this build cannot name was written by a newer one. `None` is the honest reading
        // of it — never observed — and never a guess at which outcome was meant.
        outcome: outcome.and_then(|raw| enum_from_slug::<DebtSweepOutcome>(&raw)),
        open_items: u32::try_from(open).unwrap_or(u32::MAX),
    })
}

/// §32's answer, plus the one distinction R131/F7 requires §31 to keep: a lockfile the read could
/// not take is `notRead`, and an unreachable or unsynced source is `notSynced`.
fn deps(conn: &Connection, project: ProjectId, now: i64) -> Result<DepsReading, IndexError> {
    let reading = verdict_for(conn, project, now)?;
    let scored_open: i64 = conn.query_row(
        "SELECT count(*) FROM debt_item
          WHERE project_id = ?1 AND source = 'dependency_advisory'
            AND state = 'open' AND scoring = 'scored'",
        [project.0],
        |r| r.get(0),
    )?;
    let not_read: i64 = conn.query_row(
        "SELECT count(*) FROM project_lockfile WHERE project_id = ?1 AND read_state = 'notRead'",
        [project.0],
        |r| r.get(0),
    )?;
    Ok(DepsReading {
        verdict: reading.verdict,
        scored_open: u32::try_from(scored_open).unwrap_or(u32::MAX),
        lockfile_not_read: not_read > 0,
    })
}

/// The user's stored ruling per key, in `CompletionCheck::ALL` order.
///
/// **NULL is *the user has not ruled*** and is what lets a J3 re-run re-propose without erasing a
/// decision the user made.
pub(crate) fn stored_user_na(
    conn: &Connection,
    project: ProjectId,
) -> Result<[Option<bool>; 10], IndexError> {
    let mut out = [None; 10];
    let mut st = conn.prepare(
        "SELECT check_key, user_na FROM project_check WHERE project_id = ?1 AND user_na IS NOT NULL",
    )?;
    let rows = st.query_map([project.0], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
    })?;
    for row in rows {
        let (key, value) = row?;
        place_ruling(&mut out, &key, value);
    }
    Ok(out)
}

/// The debt sources that are N/A for this project (§30.3's `notApplicable`), from the two stored
/// facts §31.4's gate reads — J3's archetype and the user's rulings — through that gate.
///
/// # Errors
/// Fails when SQLite cannot be read.
pub fn not_applicable_sources(
    conn: &Connection,
    project: ProjectId,
) -> Result<Vec<DebtSource>, IndexError> {
    let archetype: Option<String> = conn
        .query_row(
            "SELECT archetype FROM project WHERE id = ?1",
            [project.0],
            |r| r.get(0),
        )
        .optional()?
        .flatten();
    let user_na = stored_user_na(conn, project)?;
    Ok(DebtSource::ALL
        .into_iter()
        .filter(|source| source_not_applicable(*source, archetype.as_deref(), &user_na))
        .collect())
}

/// Every project's stored rulings in **one** statement, for a caller projecting the whole library.
/// A project with no ruling has no entry, which reads the same as `[None; 10]`.
pub(crate) fn all_stored_user_na(
    conn: &Connection,
) -> Result<std::collections::BTreeMap<i64, [Option<bool>; 10]>, IndexError> {
    let mut out = std::collections::BTreeMap::new();
    let mut st = conn.prepare(
        "SELECT project_id, check_key, user_na FROM project_check WHERE user_na IS NOT NULL",
    )?;
    let rows = st.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, i64>(2)?,
        ))
    })?;
    for row in rows {
        let (project, key, value) = row?;
        place_ruling(out.entry(project).or_insert([None; 10]), &key, value);
    }
    Ok(out)
}

fn place_ruling(out: &mut [Option<bool>; 10], key: &str, value: i64) {
    if let Some(index) = CompletionCheck::ALL
        .iter()
        .position(|k| enum_slug(k) == key)
    {
        if let Some(slot) = out.get_mut(index) {
            *slot = Some(value != 0);
        }
    }
}

/// A generated enum's own wire spelling, read back through serde rather than restated (R24).
fn enum_slug<T: serde::Serialize>(value: &T) -> String {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(raw)) => raw,
        _ => String::new(),
    }
}

fn enum_from_slug<T: serde::de::DeserializeOwned>(raw: &str) -> Option<T> {
    serde_json::from_value(serde_json::Value::String(raw.to_owned())).ok()
}
