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
    pub index: &'a mut Index,
    pub sessions: &'a mut SessionManager,
    pub spawner: &'a dyn Spawner,
    pub events: &'a dyn EventSink,
    pub mounts: &'a dyn MountResolver,
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
    pub id: LocationId,
    pub project_id: ProjectId,
    pub kind: crate::derive::LocationKind,
    pub distro: String,
    pub path_bytes: Vec<u8>,
    pub presence: Presence,
    pub store: crate::git::StoreKey,
    pub repo_kind: String,
    pub common_dir: Option<std::path::PathBuf>,
    /// §11.1's TRUST THIS REPOSITORY. Without it git refuses to read a repository it considers
    /// to have dubious ownership, which is what `check-ignore` would then hit.
    pub trusted: bool,
}

/// A row whose `kind` or `presence` column does not parse is [`LaunchError::Io`] with the
/// column named: a corrupt enum column is a bug, not a state, and it must not become a
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

/// `None` means "not mine". The assembling router chains the next dispatcher on it.
pub fn dispatch_launch_command(
    ctx: &mut LaunchCtx<'_>,
    command: &str,
    args: Value,
) -> Option<Result<Value, CommandFailure>> {
    match command {
        "projects.launch" => Some(handle_launch(ctx, args).and_then(|id| to_value(&id))),
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
