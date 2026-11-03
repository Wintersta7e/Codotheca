//! §40.5's one evaluator: every award is written here, in the caller's transaction.
//!
//! It reads the latched set first and never evaluates a held kind again, so a badge costs
//! nothing once it is earned. Writers never read quiet mode: an award before day one, or while
//! the user has motivation hidden, is written all the same and shown when it may be.

use rusqlite::Transaction;

use crate::clock::{local_date, LocalZone};
use crate::index::subject::subject_for_project;
use crate::index::IndexError;
use crate::jobs::JobKind;
use crate::ledger::price::is_revival_gap;
use crate::protocol::{BadgeEvidence, BadgeKind, ProjectId};

use super::evidence::encode;
use super::population::h_sql;
use super::store::{insert_award, latched, NewAward};

/// Seconds in a day: a day-grain row's `ts` is its day's midnight.
const DAY_SECONDS: i64 = 86_400;

/// What every evaluation is dated by: the caller's instant, and the zone that frames it.
#[derive(Debug, Clone, Copy)]
pub struct EvalCtx<'a> {
    /// The caller's instant, in Unix seconds — `awarded_at`.
    pub now: i64,
    /// The machine's zone; `awarded_on` is `now`'s local date in it.
    pub zone: &'a dyn LocalZone,
}

/// Evaluate `kinds` over the whole library, writing each one not held whose predicate holds.
///
/// Answers the kinds written — the write is itself the signal a reader of the
/// connection's changes sees.
///
/// # Errors
/// Fails when SQLite refuses a read or the write.
pub fn evaluate(
    tx: &Transaction<'_>,
    kinds: &[BadgeKind],
    ctx: &EvalCtx<'_>,
) -> Result<Vec<BadgeKind>, IndexError> {
    let held = latched(tx)?;
    let mut wrote = Vec::new();
    for &kind in kinds {
        if held.contains(&kind) {
            continue;
        }
        let Some(award) = predicate(tx, kind, ctx)? else {
            continue;
        };
        if insert_award(tx, &award)? {
            wrote.push(kind);
        }
    }
    Ok(wrote)
}

/// The badge evaluator's job-settle site: the kinds `job` can make true, for its project.
///
/// Each is checked over the settled project alone first — one indexed read — and only one that
/// qualifies there runs the library-wide predicate, so a first scan's thousand settles pay a
/// thousand one-project reads, not a thousand library scans.
///
/// # Errors
/// Fails when SQLite refuses a read or the write.
pub fn on_settle(
    tx: &Transaction<'_>,
    job: JobKind,
    project: ProjectId,
    ctx: &EvalCtx<'_>,
) -> Result<Vec<BadgeKind>, IndexError> {
    let held = latched(tx)?;
    let mut kinds = Vec::new();
    if job == JobKind::J4History
        && !held.contains(&BadgeKind::Necromancer)
        && revival(tx, Some(project))?.is_some()
    {
        kinds.push(BadgeKind::Necromancer);
    }
    if kinds.is_empty() {
        return Ok(kinds);
    }
    evaluate(tx, &kinds, ctx)
}

/// `kind`'s predicate over the library: the award it would write now, or `None`.
fn predicate(
    tx: &Transaction<'_>,
    kind: BadgeKind,
    ctx: &EvalCtx<'_>,
) -> Result<Option<NewAward>, IndexError> {
    match kind {
        BadgeKind::Necromancer => necromancer(tx, ctx),
        // Not evaluated yet: each of these lands with its own production caller.
        BadgeKind::Polyglot
        | BadgeKind::Comeback
        | BadgeKind::Curator
        | BadgeKind::Marathon
        | BadgeKind::Undertaker
        | BadgeKind::Firefighter
        | BadgeKind::Lifeline => Ok(None),
    }
}

/// The earliest revival among the commit-days of projects in **H** — of `project` alone when
/// given — as `(project, its name, previous day, day)`: two consecutive commit-days of one
/// project whose gap is §38's revival gap. The earliest second day wins; a tie goes to the lowest
/// project id.
fn revival(
    tx: &Transaction<'_>,
    project: Option<ProjectId>,
) -> Result<Option<(ProjectId, String, i64, i64)>, IndexError> {
    // One project's read is `idx_xp_events_project_ts`; the library's is every commit-day.
    let one = if project.is_some() {
        "AND x.project_id = ?1"
    } else {
        ""
    };
    let mut st = tx.prepare_cached(&format!(
        "SELECT p.id, p.name, LAG(x.ts) OVER (PARTITION BY x.project_id ORDER BY x.ts), x.ts
           FROM xp_events x JOIN project p ON p.id = x.project_id
          WHERE x.kind = 'commit_day' AND {} {one}",
        h_sql()
    ))?;
    let pairs = st
        .query_map(rusqlite::params_from_iter(project.map(|p| p.0)), |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<i64>>(2)?,
                r.get::<_, i64>(3)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(pairs
        .into_iter()
        .filter_map(|(id, name, prev, ts)| {
            let day = ts.div_euclid(DAY_SECONDS);
            let prev = prev?.div_euclid(DAY_SECONDS);
            is_revival_gap(prev, day).then_some((ProjectId(id), name, prev, day))
        })
        .min_by_key(|(id, _, _, day)| (*day, id.0)))
}

/// §40.3's Necromancer: a project of **H** whose commit-days hold §38's revival gap, earned on
/// the earliest such second day and naming that project.
fn necromancer(tx: &Transaction<'_>, ctx: &EvalCtx<'_>) -> Result<Option<NewAward>, IndexError> {
    let Some((project, name, prev, day)) = revival(tx, None)? else {
        return Ok(None);
    };
    let evidence = BadgeEvidence {
        project_name: Some(name),
        gap_days: u32::try_from(day - prev).ok(),
        year: None,
        languages: None,
        credited_seconds: None,
        projects: None,
        run_start_on: None,
    };
    Ok(Some(NewAward {
        kind: BadgeKind::Necromancer,
        earned_at: day * DAY_SECONDS,
        tz_offset_min: 0,
        earned_on: local_date(day),
        project: Some(project),
        subject_key: subject_for_project(tx, project)?.map(|s| s.to_key()),
        evidence: encode(&evidence)?,
        awarded_at: ctx.now,
        awarded_on: ctx.zone.at(ctx.now).date(),
    }))
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
mod tests {
    use super::*;
    use crate::ledger::price::REVIVAL_GAP_DAYS;
    use rusqlite::types::Value;

    const NOW: i64 = 1_800_000_000;
    const D1: i64 = 19_724;

    #[derive(Debug)]
    struct Utc;

    impl LocalZone for Utc {
        fn offset_min_at(&self, _unix: i64) -> i32 {
            0
        }
    }

    fn ctx() -> EvalCtx<'static> {
        EvalCtx {
            now: NOW,
            zone: &Utc,
        }
    }

    fn open() -> (tempfile::TempDir, crate::index::Index) {
        let dir = tempfile::tempdir().unwrap();
        let index = crate::index::Index::open_at(dir.path(), NOW).unwrap();
        (dir, index)
    }

    /// A live, deep, non-Reference project with a commit-day on each of `days`.
    fn project_with_days(index: &mut crate::index::Index, name: &str, days: &[i64]) -> i64 {
        index
            .with_tx(|tx| {
                tx.execute(
                    "INSERT INTO project (name, seed_basename, lineage_key, created_at, updated_at)
                     VALUES (?1, ?1, ?1, 0, 0)",
                    [name],
                )?;
                let id = tx.last_insert_rowid();
                for day in days {
                    tx.execute(
                        "INSERT INTO xp_events (ts, tz_offset_min, project_id, subject_key, kind,
                                                dedupe_key, track, meta)
                         VALUES (?1, 0, ?2, ?3, 'commit_day', ?4, 'git', '{}')",
                        rusqlite::params![
                            day * DAY_SECONDS,
                            id,
                            format!("lineage:{name}|remote:"),
                            format!("commit_day:{name}::{day}")
                        ],
                    )?;
                }
                Ok(id)
            })
            .unwrap()
    }

    fn rows(index: &crate::index::Index) -> Vec<Vec<Value>> {
        let mut st = index
            .conn()
            .prepare("SELECT * FROM badge_award ORDER BY kind")
            .unwrap();
        let width = st.column_count();
        st.query_map([], |r| (0..width).map(|i| r.get::<_, Value>(i)).collect())
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    }

    #[test]
    fn a_latched_kind_is_never_evaluated_again() {
        let (_dir, mut index) = open();
        project_with_days(&mut index, "late", &[D1, D1 + REVIVAL_GAP_DAYS + 50]);
        let first = index
            .with_tx(|tx| evaluate(tx, &[BadgeKind::Necromancer], &ctx()))
            .unwrap();
        assert_eq!(first, [BadgeKind::Necromancer]);
        let before = rows(&index);

        // A project whose revival is earlier would now win the predicate.
        project_with_days(&mut index, "early", &[D1, D1 + REVIVAL_GAP_DAYS]);
        let second = index
            .with_tx(|tx| evaluate(tx, &[BadgeKind::Necromancer], &ctx()))
            .unwrap();
        eprintln!("second evaluation wrote {second:?}");
        assert_eq!(second, []);
        assert_eq!(rows(&index), before, "the held row changed");
    }

    #[test]
    fn an_award_is_a_change_on_the_connection() {
        let (_dir, mut index) = open();
        project_with_days(&mut index, "revived", &[D1, D1 + REVIVAL_GAP_DAYS]);
        let before = index.conn().total_changes();
        let wrote = index
            .with_tx(|tx| evaluate(tx, &[BadgeKind::Necromancer], &ctx()))
            .unwrap();
        let after = index.conn().total_changes();
        eprintln!("total_changes {before} -> {after}, wrote {wrote:?}");
        assert_eq!(wrote, [BadgeKind::Necromancer]);
        assert!(after > before);
    }
}
