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
/// `wants_content` is **the caller's** (§29.7, R131/F15): this body has two callers and no way
/// to tell them apart, and the reason `projects.get` asks and `projects.peek` does not is which
/// command it is, not which lock is held. The *"where the index is already open"* justification
/// belongs to `needs_art` below and does not transfer.
pub fn notify_visible(
    index: &crate::index::Index,
    mounts: &dyn MountResolver,
    jobs: &dyn JobSink,
    project: ProjectId,
    location: LocationId,
    wants_content: bool,
) {
    let conn = index.conn();
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
    // §7.5's redraw question is answered **here**, where the index is already open, and never
    // inside the sink: `Assembly` calls both of this function's callers with the one index guard
    // held, and `std::sync::Mutex` is not reentrant, so a sink that re-locked it would wedge the
    // guard for the life of the process. A read that fails is `true` — redrawing a card that did
    // not need it costs one job; skipping one that did leaves §7.5's plate on the tile.
    let needs_art =
        crate::art::job::needs_art_at(conn, index.data_dir(), project.0).unwrap_or(true);
    jobs.on_visible(
        project,
        location,
        &store_key,
        class,
        needs_art,
        wants_content,
    );
}
