//! §2.4's `projects.launch`, `session.stop` and `session.focus`, plus the core-side startup
//! that closes sessions orphaned by a crash.

use serde::Deserialize;
use serde_json::Value;

use crate::index::Index;
use crate::launch::spawn::Spawner;
use crate::launch::{argv, resolve, verify, LaunchError};
use crate::mount::{MountError, MountResolver, StoreClass};
use crate::proto::dispatch::{parse_args, CommandFailure}; // R15: one helper, plan 03's
use crate::proto::EventSink; // R16: the trait, not plan 07's WalkSink
use crate::protocol::{ErrorCode, LocationId, Presence, ProjectId, SessionId, TargetId};
use crate::session::manager::{LaunchedSession, SessionManager};

/// Everything §2.4's launch and session commands need.
///
/// `mounts` is here because `location` stores `store_key` but **not** the store's class: the
/// class is a scheduling fact (§3.4's per-store cap) that the scan resolves and the jobs carry
/// on the `Job`. A launch has neither, so it asks the resolver rather than defaulting to
/// `Local` and letting four git processes onto a network store.
pub struct LaunchCtx<'a> {
    /// The index the launch rows are read from and the session ledger is written to.
    pub index: &'a mut Index,
    /// The sessions this process is timing: a launch opens one, STOP closes one.
    pub sessions: &'a mut SessionManager,
    /// Starts a target's process from its argv, never through a shell.
    pub spawner: &'a dyn Spawner,
    /// Where these commands publish `projects/condition_changed`, and `session/ended` for the
    /// sessions `startup` recovers.
    pub events: &'a dyn EventSink,
    /// Resolves a location's store class at launch time, for the reason above.
    pub mounts: &'a dyn MountResolver,
    /// The caller's clock reading, in unix seconds.
    pub now: i64,
}

impl std::fmt::Debug for LaunchCtx<'_> {
    /// Hand-written: `&dyn EventSink` is not `Debug` and the index holds a live connection.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LaunchCtx")
            .field("now", &self.now)
            .finish_non_exhaustive()
    }
}

// `same_name_method`'s sibling: §4bis.5 names all three arguments `*Id`, and renaming one to
// satisfy a lint would put the wire and the struct out of step.
#[allow(clippy::struct_field_names)]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LaunchArgs {
    project_id: ProjectId,
    location_id: LocationId,
    target_id: TargetId,
}

/// One `location` row, in the shape both plan 11's `LaunchSite` and plan 05's `RepoHandle`
/// need it.
///
/// **C5: this reader belongs beside R1's `upsert_location` in `core/src/identity/store.rs`.**
/// It is here only because `projects.launch` cannot be written without it and plan 08 is not
/// this plan's to modify. Plan 09's `load_location_facts` returns `LocationFacts`, which
/// carries none of these five columns.
#[derive(Debug, Clone)]
pub struct LaunchLocation {
    /// The `location` row this was read from.
    pub id: LocationId,
    /// The project this copy belongs to.
    pub project_id: ProjectId,
    /// Which filesystem the path belongs to: Windows, Linux or a WSL distro.
    pub kind: crate::derive::LocationKind,
    /// The WSL distro name for a `wsl` location; the empty string for every other kind.
    pub distro: String,
    /// The working directory's path, as the bytes the scan stored.
    pub path_bytes: Vec<u8>,
    /// Whether the path was present at the last observation; only `Present` may launch.
    pub presence: Presence,
    /// The mounted device or share the path lives on, the git scheduler's grouping key.
    pub store: crate::git::StoreKey,
    /// `RepoKind`'s spelling for this copy; `"bare"` has no working tree to resolve.
    pub repo_kind: String,
    /// The repository's common git directory, when the scan recorded one.
    pub common_dir: Option<std::path::PathBuf>,
    /// §11.1's TRUST THIS REPOSITORY. Without it git refuses to read a repository it considers
    /// to have dubious ownership, which is what `check-ignore` would then hit.
    pub trusted: bool,
}

/// Reads one `location` row in the shape a launch needs.
///
/// # Errors
/// [`LaunchError::NoSuchLocation`] when no row has this id, and [`LaunchError::Sqlite`] when the
/// read fails. A row whose `kind` or `presence` column does not parse is [`LaunchError::Io`]
/// with the column named: a corrupt enum column is a bug, not a state, and it must not become a
/// plausible default.
pub fn load_launch_location(
    conn: &rusqlite::Connection,
    location: LocationId,
) -> Result<LaunchLocation, LaunchError> {
    let row = conn
        .query_row(
            "SELECT project_id, kind, distro, path_bytes, presence, store_key, repo_kind,
                    common_dir_bytes, trusted_at
               FROM location WHERE id = ?1",
            [location.0],
            |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Vec<u8>>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, String>(6)?,
                    r.get::<_, Option<Vec<u8>>>(7)?,
                    r.get::<_, Option<i64>>(8)?,
                ))
            },
        )
        .map_err(|err| match err {
            rusqlite::Error::QueryReturnedNoRows => LaunchError::NoSuchLocation(location.0),
            other => LaunchError::Sqlite(other),
        })?;
    let (project_id, kind, distro, path_bytes, presence, store_key, repo_kind, common, trusted) =
        row;

    let kind = parse_location_kind(&kind)
        .ok_or_else(|| LaunchError::Io(format!("location.kind holds {kind}")))?;
    let presence = parse_presence(&presence)
        .ok_or_else(|| LaunchError::Io(format!("location.presence holds {presence}")))?;

    Ok(LaunchLocation {
        id: location,
        project_id: ProjectId(project_id),
        kind,
        distro,
        path_bytes,
        presence,
        store: crate::git::StoreKey::new(store_key),
        repo_kind,
        common_dir: common.as_deref().map(crate::paths::path_from_bytes),
        trusted: trusted.is_some(),
    })
}

/// The `kind` column's spellings. Written out rather than derived: the generated enum has no
/// parser, and a wildcard would turn a new variant into a silent miss.
fn parse_location_kind(text: &str) -> Option<crate::derive::LocationKind> {
    use crate::derive::LocationKind;
    match text {
        "win" => Some(LocationKind::Win),
        "linux" => Some(LocationKind::Linux),
        "wsl" => Some(LocationKind::Wsl),
        _ => None,
    }
}

fn parse_presence(text: &str) -> Option<Presence> {
    match text {
        "present" => Some(Presence::Present),
        "offline" => Some(Presence::Offline),
        "missing" => Some(Presence::Missing),
        "unscanned" => Some(Presence::Unscanned),
        _ => None,
    }
}

/// §4bis.5, in order, and each step is where it is for a reason:
///
/// 1. `deny_unknown_fields` — three integers is the whole contract, and a fourth key is a
///    protocol error rather than something to ignore. Ignoring it is how a `path` field
///    arrives one release later and nobody notices.
/// 2. the merge redirect — a project can be merged between the shelf painting a tile and the
///    user pressing Play (§1.5).
/// 3. the location must belong to that project — a `locationId` from another project is a
///    renderer bug, not something to launch quietly.
/// 4. presence — §6: absence of a recent observation is not permission to open a path that may
///    be on an unmounted drive.
/// 5. `verify_one`, **before** the spawn, and the result is stored, so the menu row's state is
///    current the next time it is drawn.
/// 6. `argv::build` — §4bis.4's WSL translation happens in plan 11 and nothing here touches
///    the path.
/// 7. the spawn — argv array, null stdio, **never a shell**.
/// 8. the ledger, only after the spawn succeeded: a session that never started a process is
///    playtime the app did not observe.
///
/// # Errors
/// `PROTOCOL` for arguments that do not parse, a location that is not a copy of the project, or
/// a target with no invocation for that location; `STORE_OFFLINE` when the location is not
/// present or its drive is no longer mounted; `PATH_GONE` when the target does not verify. A
/// failed redirect, index write, row read, repository resolve, spawn or ledger write carries the
/// code of the error behind it.
pub fn handle_launch(ctx: &mut LaunchCtx<'_>, args: Value) -> Result<SessionId, CommandFailure> {
    let args: LaunchArgs = parse_args(args)?;

    let project = {
        let _guard = crate::proto::txguard::TxGuard::enter();
        let tx = ctx
            .index
            .conn_mut()
            .transaction()
            .map_err(|e| CommandFailure::internal(e.to_string()))?;
        let id = crate::identity::redirect::resolve_project_id(&tx, args.project_id.0)
            .map_err(|e| identity_failure(&e))?;
        // [p3] §30.5, beside the redirect resolve rather than in a transaction of its own:
        // launching a project acknowledges it, exactly as `acknowledges()` classifies it
        // (`app/src/renderer/firstrun/newArrivals.ts:45-47`). Write-once, so a second launch
        // cannot move the time the user first acknowledged it.
        crate::health::acknowledge::stamp_acknowledged(&tx, ProjectId(id), ctx.now)
            .map_err(|e| CommandFailure::internal(e.to_string()))?;
        tx.commit()
            .map_err(|e| CommandFailure::internal(e.to_string()))?;
        ProjectId(id)
    };

    let site_row =
        load_launch_location(ctx.index.conn(), args.location_id).map_err(|e| failure(&e))?;
    if site_row.project_id != project {
        return Err(CommandFailure::protocol(
            "locationId is not a copy of projectId",
        ));
    }
    if site_row.presence != Presence::Present {
        return Err(CommandFailure {
            code: ErrorCode::StoreOffline,
            message: format!("location {} is not present", site_row.id.0),
            // [R31] `Outcome` has only `unknown`; None = definitely did not take effect.
            outcome: None,
        });
    }

    let checked = verify::verify_one(ctx.index.conn_mut(), args.target_id.0, ctx.now)
        .map_err(|e| failure(&e))?;
    if checked.verify_state != verify::VerifyState::Ok {
        return Err(CommandFailure {
            code: ErrorCode::PathGone,
            // Diagnostic only. §2.4: the shell renders the prose, from the `execDisplay` it
            // already holds on the menu row.
            message: format!(
                "target {} is {}",
                args.target_id.0,
                checked.verify_state.as_str()
            ),
            outcome: None,
        });
    }

    let target =
        resolve::load_target(ctx.index.conn(), args.target_id.0).map_err(|e| failure(&e))?;
    let site = argv::LaunchSite {
        kind: site_row.kind,
        distro: site_row.distro.clone(),
        path_bytes: site_row.path_bytes.clone(),
    };
    let inv = argv::build(&target, &site)
        .ok_or_else(|| CommandFailure::protocol("no invocation for this target and location"))?;

    // The handle is resolved before the spawn: a repository the session cannot watch is worth
    // knowing about before a process exists to account for.
    let repo = repo_handle(ctx, &site_row)?;

    let spawned = ctx
        .spawner
        .spawn(&inv, &resolve::exec_display(&target))
        .map_err(|e| failure(&e))?;

    let session = ctx
        .sessions
        .launch(
            ctx.index,
            LaunchedSession {
                project_id: project,
                location_id: Some(site_row.id),
                target_id: Some(args.target_id),
                repo,
                waiter: spawned.waiter,
            },
        )
        .map_err(|e| session_failure(&e))?;

    publish_condition_changes(ctx);
    Ok(session)
}

/// The copy as plan 05 describes it, with the store class the resolver reports now.
///
/// `MountError::NotMounted` is the drive having been pulled since the presence column was
/// written, which is `STORE_OFFLINE` and not a class question. `Unsupported` and `Io` mean no
/// mapping exists, which is exactly what `StoreClass::Unknown` names.
fn repo_handle(
    ctx: &LaunchCtx<'_>,
    site_row: &LaunchLocation,
) -> Result<crate::git::RepoHandle, CommandFailure> {
    let work_dir = crate::paths::path_from_bytes(&site_row.path_bytes);
    let class = match ctx.mounts.resolve(&work_dir) {
        Ok(facts) => facts.class,
        Err(MountError::NotMounted) => {
            return Err(CommandFailure {
                code: ErrorCode::StoreOffline,
                message: format!("location {} is not mounted", site_row.id.0),
                outcome: None,
            })
        }
        Err(MountError::Unsupported(_) | MountError::Io(_)) => StoreClass::Unknown,
    };

    let handle = if site_row.repo_kind == "bare" {
        crate::git::RepoHandle::bare(&work_dir, site_row.store.clone(), class)
    } else {
        crate::git::RepoHandle::resolve(&work_dir, site_row.store.clone(), class)
            .map_err(|e| git_failure(&e))?
    };
    Ok(handle.with_trust(site_row.trusted))
}

/// Publishes `projects/condition_changed` for each project the session manager hands over.
///
/// §5.1's *last session end* term moves when a session **closes**, and that can move
/// `condition_signal`. Plan 11b emits the three `session/*` events; this is the fourth, and it
/// publishes exactly what the manager hands over — never a change the manager did not report.
pub fn publish_condition_changes(ctx: &mut LaunchCtx<'_>) {
    for project in ctx.sessions.take_condition_changes() {
        ctx.events.emit(
            "projects",
            "condition_changed",
            serde_json::json!({ "projectId": project }),
        );
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StopArgs {
    id: SessionId,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FocusArgs {
    #[serde(default)]
    project_id: Option<ProjectId>,
}

/// §7.8's STOP: close the ledger. It writes nothing to disk and kills nothing (§17).
///
/// **L3: stopping an already-closed session succeeds.** A user pressing STOP twice, or
/// pressing it on a tile whose wait-mode process exited a moment earlier, has asked for a
/// state that already holds, and closing a closed ledger is not a second close. An id naming
/// **no row at all** is a renderer bug rather than a race, so only that reports one — which is
/// why the row is looked up rather than inferred from `SessionManager::stop`, whose own
/// contract is to succeed for any session it is not holding.
///
/// # Errors
/// `PROTOCOL` for arguments that do not parse; the session error's code when the id names no
/// session row, or when the row cannot be read or the ledger cannot be closed.
pub fn handle_stop(ctx: &mut LaunchCtx<'_>, args: Value) -> Result<Value, CommandFailure> {
    let args: StopArgs = parse_args(args)?;
    crate::session::store::session_ref(ctx.index.conn(), args.id)
        .map_err(|e| session_failure(&e))?;
    ctx.sessions
        .stop(ctx.index, args.id)
        .map_err(|e| session_failure(&e))?;
    publish_condition_changes(ctx);
    Ok(serde_json::json!({}))
}

/// L4: the only command in §2.4's table that opens no transaction and issues no statement.
///
/// It arrives on every project-page entry and exit, and once per heartbeat while a page is
/// held. A handler that opened a write transaction would put `TxGuard` contention on a timer,
/// on the one command whose whole purpose is to be cheap enough to repeat.
///
/// # Errors
/// `PROTOCOL` when the arguments do not parse; nothing else can fail.
pub fn handle_focus(ctx: &mut LaunchCtx<'_>, args: Value) -> Result<Value, CommandFailure> {
    let args: FocusArgs = parse_args(args)?;
    ctx.sessions.set_focus(args.project_id);
    Ok(serde_json::json!({}))
}

/// Closes the sessions a previous process left open and publishes `session/ended` for each.
///
/// L1: called from the core's assembly **before `run_loop`**, so every open row it finds is by
/// definition from a previous process. §11.2's core lane and plan 11b Task 7's safety argument
/// both depend on that ordering, and a `JoinStep` could not provide it — by the time a step
/// runs, the loop is already accepting commands.
///
/// `close_orphans` returns counts and not ids, so the ids are read first. That ordering is the
/// only way to publish recovery without changing plan 11b's signature.
///
/// # Errors
/// The session error's code when the open sessions cannot be read, the orphans cannot be
/// closed, or a closed session cannot be read back for its event.
pub fn startup(
    ctx: &mut LaunchCtx<'_>,
) -> Result<crate::session::orphan::OrphanReport, CommandFailure> {
    let open: Vec<SessionId> = crate::session::store::open_sessions(ctx.index.conn())
        .map_err(|e| session_failure(&e))?
        .into_iter()
        .map(|row| row.session_id)
        .collect();

    let report = crate::session::orphan::close_orphans(ctx.index, ctx.now)
        .map_err(|e| session_failure(&e))?;

    for id in open {
        let session = crate::session::store::session_ref(ctx.index.conn(), id)
            .map_err(|e| session_failure(&e))?;
        ctx.events.emit(
            "session",
            "ended",
            serde_json::json!({ "session": session }),
        );
    }
    Ok(report)
}

/// Scheduled by the core's assembly every `DEFAULT_TICK_SECS`. Never awaited — a driver that
/// slept on the clock would spin under a fake one.
///
/// # Errors
/// The session error's code when the manager's §9 pass over the live sessions fails.
pub fn tick(ctx: &mut LaunchCtx<'_>) -> Result<(), CommandFailure> {
    ctx.sessions
        .tick(ctx.index)
        .map_err(|e| session_failure(&e))?;
    publish_condition_changes(ctx);
    Ok(())
}

/// `None` means "not mine". The assembling router chains the next dispatcher on it.
pub fn dispatch_launch_command(
    ctx: &mut LaunchCtx<'_>,
    command: &str,
    args: Value,
) -> Option<Result<Value, CommandFailure>> {
    match command {
        "projects.launch" => Some(handle_launch(ctx, args).and_then(|id| to_value(&id))),
        "session.stop" => Some(handle_stop(ctx, args)),
        "session.focus" => Some(handle_focus(ctx, args)),
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
        outcome: None,
    }
}

fn session_failure(err: &crate::session::SessionError) -> CommandFailure {
    CommandFailure {
        code: err.code(),
        message: err.to_string(),
        outcome: None,
    }
}

fn git_failure(err: &crate::git::GitError) -> CommandFailure {
    session_failure(&crate::session::SessionError::Git(err.clone()))
}

fn identity_failure(err: &crate::identity::IdentityError) -> CommandFailure {
    CommandFailure {
        code: err.code(),
        message: format!("{err:?}"),
        outcome: None,
    }
}
