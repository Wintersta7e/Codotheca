//! §6 — asking for a current worktree reading when something is looked at.
//!
//! `JobSink::on_visible` had no caller at all, which is what made "no changes as of T" a stale
//! answer rather than a current one: the reading was whatever the last scan happened to see.
//!
//! **This does not make the answer synchronous, and must not pretend to.** The command answers
//! from what is stored, with its `as_of`; the job updates it and publishes a change. Absence of
//! dirty stays "no changes as of T" — never "clean".

use crate::mount::{MountResolver, StoreClass};
use crate::paths::path_from_bytes;
use crate::protocol::{LocationId, ProjectId};

use super::JobSink;

/// Tell the scheduler that one copy of a project is being looked at.
///
/// Called from `projects.peek` and `projects.get`, and deliberately **not** from
/// `projects.list`: a shelf of a thousand rows would queue a thousand status jobs on every
/// keystroke, and §8's virtualizer means "in the page" is not "on screen".
///
/// A location that cannot be read is not an error the user should see. §6 is a request for
/// freshness, so failing to make one leaves the stored answer exactly as honest as it was.
pub fn notify_visible(
    conn: &rusqlite::Connection,
    mounts: &dyn MountResolver,
    jobs: &dyn JobSink,
    project: ProjectId,
    location: LocationId,
) {
    let Ok((store_key, path_bytes)) = conn.query_row(
        "SELECT store_key, path_bytes FROM location WHERE id = ?1",
        [location.0],
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, Vec<u8>>(1)?)),
    ) else {
        return;
    };
    // The class is **not** a column (R27): it is a property of the mount right now, and a
    // persisted copy goes stale the moment a drive is remounted elsewhere. This is not R4's
    // forbidden second resolve either — R4 forbids re-resolving a location the walk resolved in
    // the same run, and looking at a tile is a different act at a different time.
    //
    // `Unknown` is a real answer, not a fabricated one: it is what "this mount could not be
    // identified" means, and `per_store_cap` treats it as the cautious default it is.
    let class = mounts
        .resolve(&path_from_bytes(&path_bytes))
        .map_or(StoreClass::Unknown, |facts| facts.class);
    jobs.on_visible(project, location, &store_key, class);
}
