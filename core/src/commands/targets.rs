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
use crate::protocol::{
    LocationId, ProjectId, ResolvedTarget, TargetId, TargetList, TargetRow, TargetVerification,
};

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
        "targets.setDefault" => Some(handle_set_default(ctx, args)),
        "targets.upsert" => Some(handle_upsert(ctx, args).and_then(|row| to_value(&row))),
        "targets.verify" => Some(handle_verify(ctx, args).and_then(|rows| to_value(&rows))),
        _ => None,
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SetDefaultArgs {
    target_id: TargetId,
    #[serde(default)]
    project_id: Option<ProjectId>,
    #[serde(default)]
    location_id: Option<LocationId>,
    #[serde(default)]
    language: Option<String>,
}

/// §4bis.2a's scope triple. A row has exactly one, and `targets.setDefault` names one.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TargetScope {
    pub project_id: Option<ProjectId>,
    pub location_id: Option<LocationId>,
    pub language: Option<String>,
}

#[must_use]
pub fn scope_of(target: &StoredTarget) -> TargetScope {
    TargetScope {
        project_id: target.project_id.map(ProjectId),
        location_id: target.location_id.map(LocationId),
        language: target.language.clone(),
    }
}

/// Renumber one scope's rows of one kind densely from 0, with `head` first.
///
/// The obvious scheme is `MIN(sort_index) - 1`, which drives the column negative. `TargetRow`
/// carries `sortIndex: u32`, so two negative rows both reach the wire as 0 and the wire stops
/// agreeing with the column it is projected from. Renumbering keeps the column non-negative,
/// and the group renumbered is exactly the one `resolve` orders over: a scope triple and one
/// kind (`ORDER BY sort_index, id` inside `WHERE kind = ?`).
///
/// `IS` rather than `=` so NULL matches NULL — that comparison *is* the scope-triple predicate.
fn renumber_head(
    tx: &rusqlite::Transaction<'_>,
    kind: TargetKind,
    scope: &TargetScope,
    head: i64,
) -> Result<(), LaunchError> {
    let mut stmt = tx.prepare_cached(
        "SELECT id FROM launch_target
          WHERE kind = ?1 AND project_id IS ?2 AND location_id IS ?3 AND language IS ?4
          ORDER BY sort_index, id",
    )?;
    let ids = stmt.query_map(
        rusqlite::params![
            kind.as_str(),
            scope.project_id.map(|p| p.0),
            scope.location_id.map(|l| l.0),
            scope.language.as_deref(),
        ],
        |row| row.get::<_, i64>(0),
    )?;
    let mut ordered = vec![head];
    for id in ids {
        let id = id?;
        if id != head {
            ordered.push(id);
        }
    }
    let mut write = tx.prepare_cached("UPDATE launch_target SET sort_index = ?2 WHERE id = ?1")?;
    for (index, id) in ordered.iter().enumerate() {
        write.execute(rusqlite::params![
            id,
            i64::try_from(index).unwrap_or(i64::MAX)
        ])?;
    }
    Ok(())
}

/// L2. Returns the id of the row that now heads `scope`: the same row when it was already in
/// the scope, a fresh copy when it was not.
///
/// Re-heading in place would either move the global default out of the global scope, breaking
/// every other project, or leave tier 1 unwritable — which is the defect criterion 7 was
/// amended to close. §4bis.2a: "Within a scope the default is the row with the lowest
/// `sort_index`... No `is_default` column exists and none is added."
pub fn rehead_scope(
    tx: &rusqlite::Transaction<'_>,
    target_id: i64,
    scope: &TargetScope,
    now: i64,
) -> Result<i64, LaunchError> {
    let target = resolve::load_target(tx, target_id)?;

    if scope_of(&target) == *scope {
        renumber_head(tx, target.kind, scope, target_id)?;
        return Ok(target_id);
    }

    // A statement, not a detection: `detected = 0`. Re-detection's `rewrite_exec_for` still
    // repairs this row's `exec_bytes`, because it matches on (kind, name) and not on `detected`.
    tx.prepare_cached(
        "INSERT INTO launch_target
           (kind, name, exec_bytes, args_json, cwd_mode, env_json,
            project_id, location_id, language, sort_index, detected,
            verify_state, verified_at, disabled)
         SELECT kind, name, exec_bytes, args_json, cwd_mode, env_json,
                ?2, ?3, ?4, 0, 0, verify_state, ?5, 0
           FROM launch_target WHERE id = ?1",
    )?
    .execute(rusqlite::params![
        target_id,
        scope.project_id.map(|p| p.0),
        scope.location_id.map(|l| l.0),
        scope.language.as_deref(),
        now,
    ])?;
    let copied = tx.last_insert_rowid();
    renumber_head(tx, target.kind, scope, copied)?;
    Ok(copied)
}

/// §4bis.2a's tier-1 write: `{targetId, projectId}` with the other two null.
pub fn handle_set_default(ctx: &mut TargetsCtx<'_>, args: Value) -> Result<Value, CommandFailure> {
    let args: SetDefaultArgs = parse_args(args)?;
    let now = ctx.now;
    let _guard = crate::proto::txguard::TxGuard::enter();
    let tx = ctx
        .index
        .conn_mut()
        .transaction()
        .map_err(|e| CommandFailure::internal(e.to_string()))?;

    // A project can be merged between the menu painting and the choice (§1.5).
    let project_id = match args.project_id {
        None => None,
        Some(requested) => Some(ProjectId(
            crate::identity::redirect::resolve_project_id(&tx, requested.0)
                .map_err(|e| identity_failure(&e))?,
        )),
    };

    let scope = TargetScope {
        project_id,
        location_id: args.location_id,
        language: args.language,
    };
    rehead_scope(&tx, args.target_id.0, &scope, now).map_err(|e| failure(&e))?;
    tx.commit()
        .map_err(|e| CommandFailure::internal(e.to_string()))?;
    Ok(serde_json::json!({}))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UpsertArgs {
    #[serde(default)]
    target_id: Option<TargetId>,
    kind: TargetKind,
    name: String,
    exec_bytes: crate::protocol::Bytes,
    #[serde(default)]
    argv: Vec<String>,
    #[serde(default)]
    project_id: Option<ProjectId>,
    #[serde(default)]
    location_id: Option<LocationId>,
    #[serde(default)]
    language: Option<String>,
}

/// §2.4's trust rule, core-side, and the third of its three guards — the schema marks the
/// command `privileged`, the bridge refuses a privileged command from the renderer, and this
/// refuses an `execBytes` a native dialog could not have produced.
///
/// The rule is one sentence: **the executable must be an absolute path.** A dialog returns
/// nothing else. A relative program would be resolved by the OS through `PATH` or the working
/// directory at spawn time, and this process controls neither. `is_absolute` is host-correct on
/// both targets: a WSL target's program is `wsl.exe`, a Windows path, because §4bis.4 launches
/// into a distro through it rather than executing a Linux binary directly.
pub fn check_dialog_origin(exec: &std::path::Path) -> Result<(), CommandFailure> {
    if exec.is_absolute() {
        return Ok(());
    }
    Err(CommandFailure::protocol(
        "targets.upsert requires an absolute executable from a native dialog",
    ))
}

/// The one command in phase 1 whose argument *is* an executable.
///
/// Everything besides `execBytes` is a renderer-supplied string stored **verbatim, never
/// interpreted**: `name` is a label, and `argv` is arguments to the program named by
/// `execBytes` and can never itself be a program. There is no shell anywhere on the path from
/// this row to `std::process::Command`.
pub fn handle_upsert(ctx: &mut TargetsCtx<'_>, args: Value) -> Result<TargetRow, CommandFailure> {
    let args: UpsertArgs = parse_args(args)?;
    let exec = crate::paths::path_from_bytes(&args.exec_bytes.0);
    check_dialog_origin(&exec)?;
    if args.name.trim().is_empty() {
        return Err(CommandFailure::protocol(
            "targets.upsert requires a non-empty name",
        ));
    }
    let argv_json =
        serde_json::to_string(&args.argv).map_err(|e| CommandFailure::internal(e.to_string()))?;

    let id = {
        let _guard = crate::proto::txguard::TxGuard::enter();
        let tx = ctx
            .index
            .conn_mut()
            .transaction()
            .map_err(|e| CommandFailure::internal(e.to_string()))?;

        let project_id = match args.project_id {
            None => None,
            Some(requested) => Some(
                crate::identity::redirect::resolve_project_id(&tx, requested.0)
                    .map_err(|e| identity_failure(&e))?,
            ),
        };
        let scope = TargetScope {
            project_id: project_id.map(ProjectId),
            location_id: args.location_id,
            language: args.language,
        };

        // Rewriting the row's executable invalidates the verification that described the old
        // one: keeping `ok` would claim a currency the row no longer has (§6).
        let id = if let Some(existing) = args.target_id {
            {
                let changed = tx
                    .prepare_cached(
                        "UPDATE launch_target
                            SET kind = ?2, name = ?3, exec_bytes = ?4, args_json = ?5,
                                project_id = ?6, location_id = ?7, language = ?8,
                                verify_state = 'unverified', verified_at = NULL
                          WHERE id = ?1",
                    )
                    .and_then(|mut stmt| {
                        stmt.execute(rusqlite::params![
                            existing.0,
                            args.kind.as_str(),
                            args.name,
                            args.exec_bytes.0,
                            argv_json,
                            scope.project_id.map(|p| p.0),
                            scope.location_id.map(|l| l.0),
                            scope.language.as_deref(),
                        ])
                    })
                    .map_err(|e| failure(&LaunchError::Sqlite(e)))?;
                if changed == 0 {
                    return Err(failure(&LaunchError::NoSuchTarget(existing.0)));
                }
                existing.0
            }
        } else {
            {
                tx.prepare_cached(
                    "INSERT INTO launch_target
                       (kind, name, exec_bytes, args_json, cwd_mode, env_json,
                        project_id, location_id, language, sort_index, detected, verify_state)
                     VALUES (?1, ?2, ?3, ?4, 'location', '{}', ?5, ?6, ?7, 0, 0, 'unverified')",
                )
                .and_then(|mut stmt| {
                    stmt.execute(rusqlite::params![
                        args.kind.as_str(),
                        args.name,
                        args.exec_bytes.0,
                        argv_json,
                        scope.project_id.map(|p| p.0),
                        scope.location_id.map(|l| l.0),
                        scope.language.as_deref(),
                    ])
                })
                .map_err(|e| failure(&LaunchError::Sqlite(e)))?;
                let inserted = tx.last_insert_rowid();
                // A target the user just chose in a dialog is the one they meant to use.
                renumber_head(&tx, args.kind, &scope, inserted).map_err(|e| failure(&e))?;
                inserted
            }
        };

        tx.commit()
            .map_err(|e| CommandFailure::internal(e.to_string()))?;
        id
    };

    let stored = resolve::load_target(ctx.index.conn(), id).map_err(|e| failure(&e))?;
    to_target_row(&stored).map_err(|e| failure(&e))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct VerifyArgs {
    #[serde(default)]
    target_id: Option<TargetId>,
}

/// `TargetVerification.verifiedAt` is a non-optional `Timestamp`: a verification is produced
/// only by a check that just ran, so the stamp always exists. `TargetRow.verifiedAt` stays
/// optional, which is correct for a row that has never been verified.
#[must_use]
pub fn to_verification(v: &crate::launch::verify::Verification) -> TargetVerification {
    TargetVerification {
        target_id: TargetId(v.target_id),
        verify_state: v.verify_state,
        verified_at: v.verified_at,
        exec_display: v.exec_display.clone(),
    }
}

/// §4bis.2a: *"`targets.verify` is per row and language-blind."* No argument verifies every
/// stored row — override, language and global alike, disabled ones included — which is the
/// startup sweep; an argument verifies one, which is `projects.launch`'s before-spawn check.
pub fn handle_verify(
    ctx: &mut TargetsCtx<'_>,
    args: Value,
) -> Result<Vec<TargetVerification>, CommandFailure> {
    let args: VerifyArgs = parse_args(args)?;
    let now = ctx.now;
    let conn = ctx.index.conn_mut();
    let verified = match args.target_id {
        None => crate::launch::verify::verify_all(conn, now).map_err(|e| failure(&e))?,
        Some(id) => {
            vec![crate::launch::verify::verify_one(conn, id.0, now).map_err(|e| failure(&e))?]
        }
    };
    Ok(verified.iter().map(to_verification).collect())
}

fn identity_failure(err: &crate::identity::IdentityError) -> CommandFailure {
    CommandFailure {
        code: err.code(),
        message: format!("{err:?}"),
        // [R31] `Outcome` has only `unknown`; None = definitely did not take effect.
        outcome: None,
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
