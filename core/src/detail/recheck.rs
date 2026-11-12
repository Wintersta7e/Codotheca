//! `projects.recheck` (§54.2) — the user asks for a re-read of a project's current primary copy.
//!
//! Before this command no user action queued J1: the walk did, with a `Walk` origin, so an
//! opened page's J7 compared the **stored** head and a fix made in an editor since the last walk
//! never reached §34's restoration as `foreground`. This queues J1 at `Interactive` with an
//! `Interactive` origin, and the chain carries that origin to every closure it observes.
//!
//! **Reads only** (U34): no sync task, no write intent, no row written, and no job state reset —
//! it is not `projects.requeue`, which revives failed and deferred rows.

use crate::detail::DetailCtx;
use crate::projects::rows::{locations_of, pick_primary};
use crate::proto::dispatch::{parse_args, CommandFailure};
use crate::protocol::{ProjectId, ProjectsRecheckArgs, RecheckResult};

/// Answer `projects.recheck`: resolve the id, pick the primary copy the way `projects.get` does,
/// and ask the job sink to re-read it when it is current.
///
/// # Errors
/// `PROTOCOL` for an argument shape the schema does not admit or an id that names no project;
/// `PROJECT_MERGED` for a merged-away id with no redirect; `INTERNAL` for an index fault.
pub fn handle_project_recheck(
    ctx: &DetailCtx<'_>,
    args: serde_json::Value,
) -> Result<RecheckResult, CommandFailure> {
    let a: ProjectsRecheckArgs = parse_args(args)?;
    let conn = ctx.index.conn();
    // A stale link crossing a merge lands on the survivor, as `projects.get` does. The
    // transaction is read-only and dropped uncommitted.
    let id = {
        let _guard = crate::proto::txguard::TxGuard::enter();
        let tx = conn
            .unchecked_transaction()
            .map_err(|e| CommandFailure::internal(e.to_string()))?;
        crate::identity::redirect::resolve_project_id(&tx, a.id.0)
            .map_err(|e| super::get::identity_failure(&e))?
    };
    let locations = locations_of(conn, ProjectId(id))
        .map_err(|e| CommandFailure::internal(format!("{e:?}")))?;
    let Some(primary) = pick_primary(&locations) else {
        return Ok(RecheckResult { queued: false });
    };
    let queued = crate::jobs::visible::request_recheck(
        ctx.index,
        ctx.mount,
        ctx.jobs,
        ProjectId(id),
        primary.id,
    );
    Ok(RecheckResult { queued })
}
