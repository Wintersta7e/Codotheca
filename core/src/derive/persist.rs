//! Writing what §5 derives.
//!
//! Every column here is NULL-able with no default, and the writer is forbidden to write 0 for
//! unknown (§1.10) — which is enforced by the types rather than by care: an `Option` that is
//! `None` becomes SQL NULL, and nothing below coalesces.
//!
//! Without this module `last_touched_at` is never set, §8.1 sections every project into the same
//! era, and `condition_signal` stays NULL forever — which §5.4a renders as *no dot at all*,
//! correctly, for a library that has in fact been fully indexed.

use rusqlite::{Connection, Transaction};

use super::condition::{signal, ConditionSignal, Inputs};
use super::{aggregate, Aggregated, LocationFacts, LocationKind};
use crate::index::IndexError;
use crate::protocol::{LocationId, Presence, ProjectId};

/// What one recompute concluded, and whether the wire should hear about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recomputed {
    /// §5.1's clocks and the primary location.
    pub aggregated: Aggregated,
    /// §5.4's band, off `last_interaction_at`.
    pub condition_signal: Option<ConditionSignal>,
    /// §5.4's second band, off `last_commit_at`. Stored, and not rendered in phase 1.
    pub condition_material: Option<ConditionSignal>,
    /// Whether `projects/condition_changed` should fire (§2.4).
    pub changed_condition: bool,
}

fn kind_of(s: &str) -> LocationKind {
    LocationKind::parse(s).unwrap_or(LocationKind::Win)
}

fn presence_of(s: &str) -> Presence {
    match s {
        "offline" => Presence::Offline,
        "missing" => Presence::Missing,
        "unscanned" => Presence::Unscanned,
        _ => Presence::Present,
    }
}

/// Read every copy of one project.
///
/// # Errors
/// Fails when SQLite refuses the read of the project's `location` rows.
pub fn load_location_facts(
    conn: &Connection,
    project: ProjectId,
) -> Result<Vec<LocationFacts>, IndexError> {
    let mut stmt = conn.prepare(
        "SELECT id, kind, presence, is_dirty, worktree_newest_mtime, reflog_tail_at,
                    branch, ahead, behind
             FROM location WHERE project_id = ?1
             ORDER BY id",
    )?;
    let rows = stmt.query_map([project.0], |r| {
        Ok(LocationFacts {
            location_id: LocationId(r.get(0)?),
            kind: kind_of(&r.get::<_, String>(1)?),
            presence: presence_of(&r.get::<_, String>(2)?),
            is_dirty: r.get::<_, Option<i64>>(3)?.map(|v| v != 0),
            worktree_newest_mtime: r.get(4)?,
            reflog_tail_at: r.get(5)?,
            branch: r.get(6)?,
            ahead: r
                .get::<_, Option<i64>>(7)?
                .and_then(|v| u32::try_from(v).ok()),
            behind: r
                .get::<_, Option<i64>>(8)?
                .and_then(|v| u32::try_from(v).ok()),
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

/// Recompute and write §5's derived values for one project.
///
/// # Errors
/// Fails when the project has no row, or SQLite refuses a read of its locations, sessions or job
/// states, or the `project` update.
pub fn recompute(
    tx: &Transaction<'_>,
    project: ProjectId,
    now: i64,
) -> Result<Recomputed, IndexError> {
    let locations = load_location_facts(tx, project)?;

    let (last_user_commit_at, last_commit_at, first_commit_at, previous): (
        Option<i64>,
        Option<i64>,
        Option<i64>,
        Option<String>,
    ) = tx.query_row(
        "SELECT last_user_commit_at, last_commit_at, first_commit_at, condition_signal
             FROM project WHERE id = ?1",
        [project.0],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
    )?;

    // Two ledgers, never merged: this reads the *session* clock only to place the project on a
    // timeline. Nothing here sums playtime with anything git-derived.
    let last_session_end_at: Option<i64> = tx.query_row(
        "SELECT MAX(ended_at) FROM session WHERE project_id = ?1",
        [project.0],
        |r| r.get(0),
    )?;

    let aggregated = aggregate(&locations, last_user_commit_at, last_session_end_at);

    let jobs = crate::jobs::state::load(tx, project)?;
    // "Has commits" is knowable only once J4 has resolved a root set: `first_commit_at` NULL
    // with J4 done is a measured zero-commit repository, and NULL with J4 not done is *not
    // computed*. The two must not collapse (§5.4a).
    let j4_done = jobs.iter().any(|r| {
        r.job == crate::jobs::JobKind::J4History && r.state == crate::jobs::JobState::Done
    });
    let has_commits = j4_done.then(|| first_commit_at.is_some());
    let any_job_succeeded = jobs.iter().any(|r| r.state == crate::jobs::JobState::Done);

    let all_offline = !locations.is_empty()
        && locations
            .iter()
            .all(|l| matches!(l.presence, Presence::Offline));

    let sig = signal(&Inputs {
        clock_at: aggregated.last_interaction_at,
        now,
        has_commits,
        all_locations_offline: all_offline,
        any_job_succeeded,
    });
    let material = signal(&Inputs {
        clock_at: last_commit_at,
        now,
        has_commits,
        all_locations_offline: all_offline,
        any_job_succeeded,
    });

    tx.execute(
        "UPDATE project
            SET last_touched_at = ?2, last_interaction_at = ?3,
                condition_signal = ?4, condition_material = ?5, updated_at = ?6
          WHERE id = ?1",
        rusqlite::params![
            project.0,
            aggregated.last_touched_at,
            aggregated.last_interaction_at,
            sig.map(ConditionSignal::slug),
            material.map(ConditionSignal::slug),
            now,
        ],
    )?;

    let changed_condition = previous.as_deref() != sig.map(ConditionSignal::slug);
    Ok(Recomputed {
        aggregated,
        condition_signal: sig,
        condition_material: material,
        changed_condition,
    })
}

#[cfg(all(test, feature = "testkit"))]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
mod tests {
    use super::*;
    use crate::testing::TempIndex;

    const NOW: i64 = 1_800_000_000;

    #[test]
    fn a_project_no_job_has_touched_gets_no_band_and_no_zero() {
        let mut db = TempIndex::new();
        let project = db.insert_project();
        db.insert_location(project, "/w/a");
        let out = db
            .index_mut()
            .with_tx(|tx| recompute(tx, project, NOW))
            .unwrap();
        assert_eq!(out.condition_signal, None);
        assert_eq!(out.aggregated.last_touched_at, None);
        // The column must be NULL, not 0. A 0 here sorts a never-indexed project to 1970.
        assert_eq!(db.project_i64("last_touched_at", project), None);
        assert_eq!(db.project_text("condition_signal", project), None);
    }

    #[test]
    fn a_worktree_edit_alone_sets_both_clocks_and_lights_the_band() {
        let mut db = TempIndex::new();
        let project = db.insert_project();
        let loc = db.insert_location(project, "/w/a");
        db.set_location_i64(loc, "worktree_newest_mtime", NOW - 3 * 86_400);
        db.mark_job_done(project, crate::jobs::JobKind::J1Refstate);

        let out = db
            .index_mut()
            .with_tx(|tx| recompute(tx, project, NOW))
            .unwrap();
        assert_eq!(out.aggregated.last_touched_at, Some(NOW - 3 * 86_400));
        assert_eq!(out.aggregated.last_interaction_at, Some(NOW - 3 * 86_400));
        assert_eq!(out.condition_signal, Some(ConditionSignal::Live));
        assert_eq!(
            db.project_text("condition_signal", project).as_deref(),
            Some("live")
        );
    }

    #[test]
    fn the_two_clocks_are_two_columns_and_are_never_merged() {
        // §5.4: condition_signal runs off last_interaction_at, condition_material off
        // last_commit_at. Two columns, two clocks; the second is stored and not rendered.
        let mut db = TempIndex::new();
        let project = db.insert_project();
        let loc = db.insert_location(project, "/w/a");
        db.set_location_i64(loc, "worktree_newest_mtime", NOW - 2 * 86_400);
        db.set_project_i64(project, "last_commit_at", NOW - 200 * 86_400);
        db.mark_job_done(project, crate::jobs::JobKind::J1Refstate);

        let out = db
            .index_mut()
            .with_tx(|tx| recompute(tx, project, NOW))
            .unwrap();
        assert_eq!(out.condition_signal, Some(ConditionSignal::Live));
        assert_eq!(out.condition_material, Some(ConditionSignal::Neglected));
    }

    #[test]
    fn a_project_whose_every_location_is_offline_freezes_rather_than_decaying() {
        let mut db = TempIndex::new();
        let project = db.insert_project();
        let loc = db.insert_location(project, "/w/a");
        db.set_location_text(loc, "presence", "offline");
        db.mark_job_done(project, crate::jobs::JobKind::J1Refstate);
        let out = db
            .index_mut()
            .with_tx(|tx| recompute(tx, project, NOW))
            .unwrap();
        assert_eq!(out.condition_signal, Some(ConditionSignal::Offline));
    }

    #[test]
    fn a_band_that_does_not_move_reports_no_change() {
        let mut db = TempIndex::new();
        let project = db.insert_project();
        let loc = db.insert_location(project, "/w/a");
        db.set_location_i64(loc, "worktree_newest_mtime", NOW - 86_400);
        db.mark_job_done(project, crate::jobs::JobKind::J1Refstate);
        assert!(
            db.index_mut()
                .with_tx(|tx| recompute(tx, project, NOW))
                .unwrap()
                .changed_condition
        );
        assert!(
            !db.index_mut()
                .with_tx(|tx| recompute(tx, project, NOW))
                .unwrap()
                .changed_condition
        );
    }

    /// A project with a clock but no finished job still has no band.
    ///
    /// The plan proposed proving `any_job_succeeded` with
    /// `a_project_no_job_has_touched_gets_no_band_and_no_zero`, and that test **passes with the
    /// check forced to `true`** — its project has no clock either, so `signal` returns `None`
    /// from the `clock_at?` line regardless. Isolating the two is the whole point: a worktree
    /// mtime is on disk whether or not the app has ever read the repository, so a band derived
    /// from it alone would claim an observation that never happened (§1.2).
    #[test]
    fn a_clock_without_a_finished_job_is_still_not_computed() {
        let mut db = TempIndex::new();
        let project = db.insert_project();
        let loc = db.insert_location(project, "/w/a");
        db.set_location_i64(loc, "worktree_newest_mtime", NOW - 86_400);

        let out = db
            .index_mut()
            .with_tx(|tx| recompute(tx, project, NOW))
            .unwrap();
        assert_eq!(out.condition_signal, None);
        assert_eq!(db.project_text("condition_signal", project), None);
        // The clock itself is still derived and written — it is the *band* that needs a job.
        assert_eq!(out.aggregated.last_touched_at, Some(NOW - 86_400));
    }

    /// A zero-commit repository is `empty` only once J4 has *measured* that. Before then
    /// `first_commit_at` is NULL for the same reason a fully-indexed empty repository's is, and
    /// collapsing the two would call every project empty on first run.
    #[test]
    fn empty_needs_j4_to_have_run_not_merely_a_null_first_commit() {
        let mut db = TempIndex::new();
        let project = db.insert_project();
        let loc = db.insert_location(project, "/w/a");
        db.set_location_i64(loc, "worktree_newest_mtime", NOW - 86_400);
        db.mark_job_done(project, crate::jobs::JobKind::J1Refstate);
        assert_eq!(
            db.index_mut()
                .with_tx(|tx| recompute(tx, project, NOW))
                .unwrap()
                .condition_signal,
            Some(ConditionSignal::Live),
        );

        db.mark_job_done(project, crate::jobs::JobKind::J4History);
        assert_eq!(
            db.index_mut()
                .with_tx(|tx| recompute(tx, project, NOW))
                .unwrap()
                .condition_signal,
            Some(ConditionSignal::Empty),
        );
    }

    /// A session that ended feeds `last_touched_at` beside the git clocks — side by side, never
    /// summed. This asserts the read happens at all; the two-ledger rule is about not adding
    /// them, not about ignoring one.
    #[test]
    fn a_finished_session_counts_towards_last_touched() {
        let mut db = TempIndex::new();
        let project = db.insert_project();
        db.insert_location(project, "/w/a");
        db.mark_job_done(project, crate::jobs::JobKind::J1Refstate);
        db.index()
            .conn()
            .execute(
                "INSERT INTO session (project_id, started_at, ended_at, close_reason)
                 VALUES (?1, ?2, ?3, 'stop')",
                rusqlite::params![project.0, NOW - 4 * 86_400, NOW - 4 * 86_400],
            )
            .unwrap();
        let out = db
            .index_mut()
            .with_tx(|tx| recompute(tx, project, NOW))
            .unwrap();
        assert_eq!(out.aggregated.last_touched_at, Some(NOW - 4 * 86_400));
    }
}
