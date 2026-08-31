//! §2.4's four `targets.*` commands, over §4bis.2a's scope triple.
//!
//! **`to_target_row` is the whole outbound trust story.** A `StoredTarget` carries
//! `exec_bytes`, `args` and `env`; a `TargetRow` carries `exec_display` and none of the three.
//! Every command in this file returns rows through that one function and no other path exists.

use rusqlite::OptionalExtension as _;
use serde::Deserialize;
use serde_json::Value;

use crate::index::Index;
use crate::launch::catalogue::TargetKind;
use crate::launch::resolve::{self, StoredTarget};
use crate::launch::verify::VerifyState;
use crate::launch::LaunchError;
use crate::proto::dispatch::{parse_args, CommandFailure}; // R15: one helper, plan 03's
use crate::proto::EventSink; // R16: the trait, not plan 07's WalkSink
use crate::protocol::{LocationId, ProjectId, ResolvedTarget, TargetId, TargetList, TargetRow};

/// Everything §2.4's `targets.*` commands need.
///
/// `index` is `&mut` because `targets.verify` runs `verify_all`, which opens a transaction from
/// `&mut Connection`. `now` is unix **seconds**, passed in so nothing here reads the clock (R3).
pub struct TargetsCtx<'a> {
    pub index: &'a mut Index,
    pub events: &'a dyn EventSink,
    pub now: i64,
}

impl std::fmt::Debug for TargetsCtx<'_> {
    /// Hand-written because `&dyn EventSink` is not `Debug` and the index holds a live
    /// connection; neither is printable and neither is what a reader wants here.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TargetsCtx")
            .field("now", &self.now)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ListArgs {
    #[serde(default)]
    project_id: Option<ProjectId>,
    #[serde(default)]
    location_id: Option<LocationId>,
}

/// The only path from a stored row to the wire (§2.4, §4bis.5).
///
/// `verify_state` is a text column with a closed DDL CHECK. A value outside that set is a
/// corrupt row, not a state: reporting `unverified` for it would make a broken row look like a
/// row nobody has checked yet.
pub fn to_target_row(target: &StoredTarget) -> Result<TargetRow, LaunchError> {
    let verify_state = VerifyState::parse(&target.verify_state).ok_or_else(|| {
        LaunchError::Io(format!(
            "launch_target.verify_state holds {}, which is not a value of its enum",
            target.verify_state
        ))
    })?;
    Ok(TargetRow {
        id: TargetId(target.id),
        kind: target.kind,
        name: target.name.clone(),
        project_id: target.project_id.map(ProjectId),
        location_id: target.location_id.map(LocationId),
        language: target.language.clone(),
        sort_index: u32::try_from(target.sort_index.max(0)).unwrap_or(u32::MAX),
        detected: target.detected,
        verify_state,
        verified_at: target.verified_at,
        exec_display: resolve::exec_display(target),
    })
}

/// §4bis.2a: NULL until J3 runs, and NULL is *not computed*, never *any*. The caller passes it
/// straight to `resolve`, which skips tier 3 whole when it is `None`.
pub fn primary_language(
    conn: &rusqlite::Connection,
    project: ProjectId,
) -> Result<Option<String>, LaunchError> {
    let mut stmt = conn.prepare_cached("SELECT primary_language FROM project WHERE id = ?1")?;
    let value = stmt
        .query_row([project.0], |row| row.get::<_, Option<String>>(0))
        .optional()?;
    Ok(value.flatten())
}

/// §4bis.2a: resolution is computed at request time and never cached on `project`. A project
/// indexed seconds ago resolves at tier 4 and moves to tier 3 when J3 lands.
pub fn handle_list(ctx: &mut TargetsCtx<'_>, args: Value) -> Result<TargetList, CommandFailure> {
    let args: ListArgs = parse_args(args)?;
    let conn = ctx.index.conn();
    let project = args.project_id.map(|p| p.0);
    let location = args.location_id.map(|l| l.0);

    let stored = resolve::menu_rows(conn, project, location).map_err(|e| failure(&e))?;
    let mut rows = Vec::with_capacity(stored.len());
    for target in &stored {
        rows.push(to_target_row(target).map_err(|e| failure(&e))?);
    }

    let resolved = match args.project_id {
        None => None,
        Some(project_id) => {
            let language = primary_language(conn, project_id).map_err(|e| failure(&e))?;
            match resolve::resolve(
                conn,
                project_id.0,
                location,
                language.as_deref(),
                TargetKind::Editor,
            )
            .map_err(|e| failure(&e))?
            {
                None => None,
                Some(r) => Some(ResolvedTarget {
                    target: to_target_row(&r.target).map_err(|e| failure(&e))?,
                    tier: r.tier,
                }),
            }
        }
    };

    Ok(TargetList { resolved, rows })
}

/// `None` means "not mine". The assembling router chains the next dispatcher on it.
pub fn dispatch_targets_command(
    ctx: &mut TargetsCtx<'_>,
    command: &str,
    args: Value,
) -> Option<Result<Value, CommandFailure>> {
    match command {
        "targets.list" => Some(handle_list(ctx, args).and_then(|list| to_value(&list))),
        _ => None,
    }
}

fn to_value<T: serde::Serialize>(value: &T) -> Result<Value, CommandFailure> {
    serde_json::to_value(value).map_err(|e| CommandFailure::internal(e.to_string()))
}

fn failure(err: &LaunchError) -> CommandFailure {
    CommandFailure {
        code: err.code(),
        message: err.to_string(),
        // [R31] `Outcome` has only `unknown`; None = definitely did not take effect.
        outcome: None,
    }
}
