//! `projects.get` (§8.5). One project, everything the page draws, and **no git at all**: this
//! returns stored observations with their stored timestamps.
//!
//! **Every nullable column maps `Option` → `Option`.** No observation is defaulted, because a
//! `0` written into a nullable column's place is a lie the renderer cannot detect and cannot
//! undo. Two zeros below are deliberate and are not that:
//!
//! - `sort_index` is `NOT NULL` in the DDL and is an ordering hint, not a measurement.
//! - the session lane's `COALESCE(SUM(credited_seconds), 0)` is a **measured** zero: this
//!   install's own ledger is complete by construction, which is why `Activity.sessions` is
//!   `LaneState::Measured` while the commit-day lane may be `NotComputed`.
//!
//! Anything else reaching for `unwrap_or(0)` or `COALESCE` here is the invariant breaking.

use rusqlite::OptionalExtension as _;

use crate::detail::DetailCtx;
use crate::projects::rows::{
    enum_from_column, load_project_row, locations_of, pick_primary, LocationFacts,
};
use crate::proto::dispatch::{parse_args, CommandFailure};
use crate::protocol::{
    Activity, ActivityWeek, AssociationKind, HeadComparison, LaneState, LocationDetail, LocationId,
    LocationRef, ProjectDetail, ProjectId, ProjectsGetArgs, ResolvedTarget, SessionRef, TargetRow,
    VerifyState,
};

/// §8.5.5's chart: 26 weekly slots, the axis running `26 WEEKS AGO` → `THIS WEEK`.
const ACTIVITY_WEEKS: i64 = 26;
const WEEK: i64 = 7 * 86_400;

fn internal(e: impl std::fmt::Display) -> CommandFailure {
    CommandFailure::internal(e.to_string())
}

/// §2.4's enum is closed and has no `NOT_FOUND`. A caller naming a project that is not there is
/// a `PROTOCOL` fault — `INTERNAL` would claim a core defect that is not there — and a caller
/// holding an id that was merged away gets `PROJECT_MERGED`, which the shell has prose for.
fn identity_failure(e: &crate::identity::IdentityError) -> CommandFailure {
    match e {
        crate::identity::IdentityError::UnknownProject(_) => {
            CommandFailure::protocol(format!("{e:?}"))
        }
        crate::identity::IdentityError::ProjectMerged { .. } => CommandFailure {
            code: crate::protocol::ErrorCode::ProjectMerged,
            message: format!("{e:?}"),
            outcome: None,
        },
        other => CommandFailure::internal(format!("{other:?}")),
    }
}

/// The page can be opened from a stale link while a scan merges two tiles underneath it, so a
/// request that crossed in flight lands on the survivor rather than refusing. Plan 08's
/// redirect takes a `Transaction`; this one is read-only and is never committed.
fn resolved_id(conn: &rusqlite::Connection, requested: ProjectId) -> Result<i64, CommandFailure> {
    let tx = conn.unchecked_transaction().map_err(internal)?;
    let id = crate::identity::redirect::resolve_project_id(&tx, requested.0)
        .map_err(|e| identity_failure(&e))?;
    drop(tx);
    Ok(id)
}

/// The scalar columns §8.5's identity block and hero read, in one statement.
struct ProjectScalars {
    notes: Option<String>,
    remote_key: Option<String>,
    lineage_key: Option<String>,
    association_kind: Option<String>,
    first_commit_sha: Option<String>,
    first_commit_tz_offset_min: Option<i64>,
    size_worktree_bytes: Option<i64>,
    is_shallow: bool,
}

fn project_scalars(conn: &rusqlite::Connection, id: i64) -> Result<ProjectScalars, CommandFailure> {
    conn.query_row(
        "SELECT notes, remote_key, lineage_key, association_kind, first_commit_sha,
                first_commit_tz_offset_min, size_worktree_bytes, is_shallow
           FROM project WHERE id = ?1",
        [id],
        |r| {
            Ok(ProjectScalars {
                notes: r.get(0)?,
                remote_key: r.get(1)?,
                lineage_key: r.get(2)?,
                association_kind: r.get(3)?,
                first_commit_sha: r.get(4)?,
                first_commit_tz_offset_min: r.get(5)?,
                size_worktree_bytes: r.get(6)?,
                is_shallow: r.get::<_, i64>(7)? != 0,
            })
        },
    )
    .map_err(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => {
            CommandFailure::protocol(format!("no project {id}"))
        }
        other => internal(other),
    })
}

/// §8.5.2's per-copy HEAD line. Two copies whose recorded HEADs agree is a fact; two that
/// disagree is a different fact; **a copy nobody has read is neither**, so a project with one
/// location, or with any unread HEAD, compares nothing rather than reporting agreement.
fn head_comparison(locations: &[LocationFacts]) -> HeadComparison {
    let oids: Vec<&String> = locations
        .iter()
        .filter_map(|l| l.head_oid.as_ref())
        .collect();
    if oids.len() < 2 || oids.len() != locations.len() {
        return HeadComparison::NotCompared;
    }
    match oids.split_first() {
        Some((first, rest)) if rest.iter().all(|o| o == first) => HeadComparison::SameCommit,
        _ => HeadComparison::DifferentCommit,
    }
}

fn count_u32(value: Option<i64>) -> Option<u32> {
    value.and_then(|v| u32::try_from(v).ok())
}

/// One `location` row on the wire.
///
/// `volume_key` and `store_key` are not fields here and are not selected by the projection this
/// reads (criterion 63): `volume_key` is a serial or a UUID chosen to be stable rather than
/// readable, and it is unreadable in the one state — unmounted — that draws this row. The facts
/// an offline copy carries are exactly `last seen <age>` and the branch as last observed.
///
/// `coveringRootId` is `None`: no column records which `scan_root` covers a location, and
/// deriving it is a path-prefix question the scanner owns. `None` renders "not computed"; a
/// fabricated id would render a root that may not cover this path at all.
fn to_location_detail(
    facts: &LocationFacts,
    is_primary: bool,
    comparison: HeadComparison,
) -> LocationDetail {
    LocationDetail {
        location: LocationRef {
            id: facts.id,
            path_display: facts.path_display.clone(),
        },
        kind: facts.kind,
        distro: facts.distro.clone(),
        presence: facts.presence,
        is_primary,
        branch: facts.branch.clone(),
        head_oid: facts.head_oid.clone(),
        head_comparison: comparison,
        ahead: count_u32(facts.ahead),
        behind: count_u32(facts.behind),
        is_dirty: facts.is_dirty,
        untracked_count: count_u32(facts.untracked_count),
        stash_count: count_u32(facts.stash_count),
        interrupted_op: facts.interrupted_op,
        last_seen_at: facts.last_seen_at,
        refstate_observed_at: facts.refstate_observed_at,
        worktree_observed_at: facts.worktree_observed_at,
        // A content clock, not an observation clock. NULL is *no fetch recorded* — never 0,
        // never an age, and never one recovered from `refstate_observed_at`, which dates a
        // different thing, or from `refstate_basis`, which is a hash and yields no age at all.
        fetch_head_at: facts.fetch_head_at,
        trusted_at: facts.trusted_at,
        covering_root_id: None,
    }
}

fn location_details(
    conn: &rusqlite::Connection,
    id: i64,
) -> Result<Vec<LocationDetail>, CommandFailure> {
    let mut facts = locations_of(conn, ProjectId(id)).map_err(internal)?;
    facts.sort_by_key(|l| l.id.0);
    let primary = pick_primary(&facts).map(|l| l.id);
    let comparison = head_comparison(&facts);
    Ok(facts
        .iter()
        .map(|l| to_location_detail(l, primary == Some(l.id), comparison))
        .collect())
}

/// The reply RELOCATE returns: the same projection the page already reads, not a second one.
pub(crate) fn location_detail(
    conn: &rusqlite::Connection,
    location: LocationId,
) -> Result<LocationDetail, CommandFailure> {
    let project: i64 = conn
        .query_row(
            "SELECT project_id FROM location WHERE id = ?1",
            [location.0],
            |r| r.get(0),
        )
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => {
                CommandFailure::protocol(format!("no location {}", location.0))
            }
            other => internal(other),
        })?;
    location_details(conn, project)?
        .into_iter()
        .find(|d| d.location.id == location)
        .ok_or_else(|| CommandFailure::protocol(format!("no location {}", location.0)))
}

/// A stored target on the wire.
///
/// **This is a stand-in for plan 11c's `to_target_row`, which is the declared owner and does not
/// exist in this tree.** It drops `exec_bytes`, `args_json`, `cwd_mode` and `env_json` — the
/// wire carries `execDisplay` and nothing a renderer could launch from. When 11c lands, delete
/// this and import that one: two mappings from `StoredTarget` to `TargetRow` is R12's defect,
/// and returning an empty `targets` list instead would have been a lie the renderer cannot see.
fn target_row(target: &crate::launch::resolve::StoredTarget) -> TargetRow {
    TargetRow {
        id: crate::protocol::TargetId(target.id),
        kind: target.kind,
        name: target.name.clone(),
        project_id: target.project_id.map(ProjectId),
        location_id: target.location_id.map(LocationId),
        language: target.language.clone(),
        sort_index: u32::try_from(target.sort_index).unwrap_or(0),
        detected: target.detected,
        // A stored word this build does not know is `unverified`, not a guess at what it meant.
        verify_state: enum_from_column(&target.verify_state).unwrap_or(VerifyState::Unverified),
        verified_at: target.verified_at,
        exec_display: crate::launch::resolve::exec_display(target),
    }
}

fn target_rows(
    conn: &rusqlite::Connection,
    id: i64,
    primary_location: Option<i64>,
) -> Result<Vec<TargetRow>, CommandFailure> {
    let stored =
        crate::launch::resolve::menu_rows(conn, Some(id), primary_location).map_err(internal)?;
    Ok(stored.iter().map(target_row).collect())
}

/// §4bis.2a's tiers, resolved through plan 11's `resolve` rather than re-ranked here.
fn resolved_target(
    conn: &rusqlite::Connection,
    id: i64,
    primary_location: Option<i64>,
    primary_language: Option<&str>,
) -> Result<Option<ResolvedTarget>, CommandFailure> {
    let found = crate::launch::resolve::resolve(
        conn,
        id,
        primary_location,
        primary_language,
        crate::protocol::TargetKind::Editor,
    )
    .map_err(internal)?;
    Ok(found.map(|r| ResolvedTarget {
        target: target_row(&r.target),
        tier: r.tier,
    }))
}

/// §9: the one open session on this project, if this install has one running.
fn live_session(
    conn: &rusqlite::Connection,
    id: i64,
) -> Result<Option<SessionRef>, CommandFailure> {
    let open = crate::session::store::open_sessions(conn).map_err(internal)?;
    let Some(row) = open.into_iter().find(|s| s.project_id.0 == id) else {
        return Ok(None);
    };
    crate::session::store::session_ref(conn, row.session_id)
        .map(Some)
        .map_err(internal)
}

/// The 26 slot boundaries, oldest first. The newest slot ends at `now`, which is what the
/// axis's `THIS WEEK` label names.
fn week_starts(now: i64) -> Vec<i64> {
    (0..ACTIVITY_WEEKS)
        .map(|i| now - (ACTIVITY_WEEKS - i) * WEEK)
        .collect()
}

fn window_count(
    conn: &rusqlite::Connection,
    sql: &str,
    id: i64,
    from: i64,
    to: i64,
) -> Result<i64, CommandFailure> {
    conn.query_row(sql, rusqlite::params![id, from, to], |r| r.get(0))
        .map_err(internal)
}

/// §8.5.5's two lanes, side by side and never summed. The commit-day lane's state is what
/// separates *no commit days* from *not computed*; a `Some(0)` in a `not_computed` lane is the
/// invariant this whole module exists to protect, and 26 zero-height bars is the canonical
/// picture of it — a flat, complete, honest-looking record of doing nothing.
///
/// **Nothing here counts commits.** J4 produces commit-*days*, `xp_events` stores one row per
/// day, and no field on this payload could hold a sum of the two lanes even if something tried.
fn activity(
    conn: &rusqlite::Connection,
    id: i64,
    now: i64,
    is_shallow: bool,
) -> Result<Activity, CommandFailure> {
    let j4_done: bool = conn
        .query_row(
            "SELECT 1 FROM project_job_state WHERE project_id = ?1 AND job = 'j4' AND state = 'ok'",
            [id],
            |_| Ok(true),
        )
        .optional()
        .map_err(internal)?
        .unwrap_or(false);

    let commit_days = if is_shallow {
        // §10.4: permanently excluded, not pending. The lane says so rather than drawing a
        // flat record of nothing.
        LaneState::ShallowExcluded
    } else if j4_done {
        LaneState::Measured
    } else {
        LaneState::NotComputed
    };
    let measured = commit_days == LaneState::Measured;

    let mut weeks = Vec::with_capacity(usize::try_from(ACTIVITY_WEEKS).unwrap_or_default());
    for start in week_starts(now) {
        let end = start + WEEK;
        let days = if measured {
            count_u32(Some(window_count(
                conn,
                "SELECT COUNT(*) FROM xp_events
                  WHERE project_id = ?1 AND kind = 'commit_day' AND ts >= ?2 AND ts < ?3",
                id,
                start,
                end,
            )?))
        } else {
            // NULL unless the lane is measured. Not `0`.
            None
        };
        weeks.push(ActivityWeek {
            week_start: start,
            commit_days: days,
            session_count: count_u32(Some(window_count(
                conn,
                "SELECT COUNT(*) FROM session
                  WHERE project_id = ?1 AND started_at >= ?2 AND started_at < ?3",
                id,
                start,
                end,
            )?)),
            session_seconds: Some(window_count(
                conn,
                "SELECT COALESCE(SUM(credited_seconds), 0) FROM session
                  WHERE project_id = ?1 AND started_at >= ?2 AND started_at < ?3",
                id,
                start,
                end,
            )?),
        });
    }

    Ok(Activity {
        weeks,
        commit_days,
        // This install's own ledger is complete by construction: a week with no launch is a
        // measured zero, which is a different fact from an uncomputed one.
        sessions: LaneState::Measured,
    })
}

/// §8.5's payload for one project.
///
/// # Errors
/// `PROTOCOL` for an argument shape the schema does not admit or an id that names no project,
/// `PROJECT_MERGED` for a stale id with no redirect, `INTERNAL` for an index fault.
pub fn handle_project_get(
    ctx: &DetailCtx<'_>,
    args: serde_json::Value,
) -> Result<ProjectDetail, CommandFailure> {
    let a: ProjectsGetArgs = parse_args(args)?;
    let conn = ctx.index.conn();
    let id = resolved_id(conn, a.id)?;

    let loaded = load_project_row(conn, ProjectId(id)).map_err(|e| match e.code() {
        crate::protocol::ErrorCode::Protocol => CommandFailure::protocol(e.to_string()),
        _ => internal(e),
    })?;
    // `era_section_id` stays empty. §8.1's bands cut on the **local** calendar year and this
    // context carries no UTC offset — plan 13's `projects.list` owns the band rules and takes
    // one. Stamping it here from a guessed offset would file a project under the wrong section
    // at a year boundary; empty is the loader's own "not sectioned", which is true.
    let row = loaded.row;
    let scalars = project_scalars(conn, id)?;
    let (readme, recent_commits) =
        crate::projects::peek::readme_and_commits(conn, ProjectId(id)).map_err(internal)?;
    let locations = location_details(conn, id)?;
    let primary_location = locations
        .iter()
        .find(|l| l.is_primary)
        .map(|l| l.location.id.0);

    Ok(ProjectDetail {
        resolved_target: resolved_target(
            conn,
            id,
            primary_location,
            row.primary_language.as_deref(),
        )?,
        targets: target_rows(conn, id, primary_location)?,
        readme,
        notes: scalars.notes,
        remote_key: scalars.remote_key,
        lineage_key: scalars.lineage_key,
        association_kind: scalars
            .association_kind
            .as_deref()
            .and_then(enum_from_column::<AssociationKind>),
        seed_basename: row.seed_basename.clone(),
        reroll_offset: row.reroll_offset,
        playtime_seconds: crate::session::store::playtime_seconds(conn, ProjectId(id))
            .map_err(internal)?,
        live_session: live_session(conn, id)?,
        activity: activity(conn, id, ctx.now, scalars.is_shallow)?,
        recent_commits,
        first_commit_sha: scalars.first_commit_sha,
        first_commit_tz_offset_min: scalars
            .first_commit_tz_offset_min
            .and_then(|v| i32::try_from(v).ok()),
        size_worktree_bytes: scalars.size_worktree_bytes,
        locations,
        row,
    })
}
