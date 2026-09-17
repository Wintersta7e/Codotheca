//! §11.1's scan summary. Six groups read `scan_problem`, one reads
//! `project_job_state`, and the eighth is a live query over `project`.

use crate::index::path::DisplayPathTable;
use crate::index::IndexError;
use crate::proto::dispatch::{parse_args, CommandFailure}; // R15: one helper, plan 03's
use crate::protocol::{
    LocationId, ProblemGroup, ProblemHeader, ProblemItem, ProblemKind, Problems, ProblemsListArgs,
    ProjectId, ScanRunId,
};
use crate::surfaces::{display_map, SurfaceCtx};

/// §11.1: *did not index* first, *indexed with a qualification* last.
///
/// **[p2] Nine, not eight.** `abandoned_install` is §24.3c's leftover staging directory. Like
/// `deferred_slow` and `ambiguous_lineage` it is a group `scan_problem` never stores — it reads
/// `install_run` — so R26 stays closed: no CHECK constraint moves, no migration is needed, and
/// `problem_kind_from_storage` gains nothing.
pub const GROUP_ORDER: [ProblemKind; 9] = [
    ProblemKind::PermissionDenied,
    ProblemKind::UntrustedRepo,
    ProblemKind::UnreadableRepo,
    ProblemKind::DeferredSlow,
    ProblemKind::ClockSkew,
    ProblemKind::NonUtf8Path,
    ProblemKind::OfflineStore,
    ProblemKind::AmbiguousLineage,
    ProblemKind::AbandonedInstall,
];

/// §11.1's detail line names two and counts the rest.
pub const AMBIGUOUS_NAMES_SHOWN: usize = 2;

/// §1.1: ambiguity is two or more surviving candidates. Deliberately not
/// `AMBIGUOUS_NAMES_SHOWN` — one is how many names the sentence prints, the other is when
/// there is a sentence at all, and changing the first must not move the second.
const AMBIGUOUS_AT_CANDIDATES: usize = 2;

/// # Errors
/// Fails when the arguments do not parse, or when the index cannot be read.
pub fn handle(ctx: &SurfaceCtx<'_>, args: serde_json::Value) -> Result<Problems, CommandFailure> {
    let args: ProblemsListArgs = parse_args(args)?;
    list(ctx.index.conn(), args.run).map_err(|e| CommandFailure::internal(e.to_string()))
}

/// The six spellings `scan_problem.kind`'s CHECK constraint admits, and no others.
///
/// `deferred_slow` and `ambiguous_lineage` are groups this table never stores: one reads
/// `project_job_state`, the other is recomputed from `project` on every open.
#[must_use]
pub fn problem_kind_from_storage(stored: &str) -> Option<ProblemKind> {
    match stored {
        "permission_denied" => Some(ProblemKind::PermissionDenied),
        "untrusted_repo" => Some(ProblemKind::UntrustedRepo),
        "unreadable_repo" => Some(ProblemKind::UnreadableRepo),
        "clock_skew" => Some(ProblemKind::ClockSkew),
        "non_utf8_path" => Some(ProblemKind::NonUtf8Path),
        "offline_store" => Some(ProblemKind::OfflineStore),
        _ => None,
    }
}

/// The run every group is reported against.
struct RunFacts {
    id: ScanRunId,
    walked_dirs: i64,
    repositories: u32,
    finished: bool,
}

/// # Errors
/// Fails when the index cannot be read.
pub fn list(conn: &rusqlite::Connection, run: Option<ScanRunId>) -> Result<Problems, IndexError> {
    let Some(run) = latest_run(conn, run)? else {
        return Ok(Problems {
            run_id: None,
            header: ProblemHeader {
                walked_dirs: 0,
                repositories: 0,
                problem_count: None,
                ambiguous_lineage_count: None,
            },
            groups: Vec::new(),
        });
    };

    let mut groups: Vec<ProblemGroup> = Vec::with_capacity(GROUP_ORDER.len());
    let mut problem_count: u32 = 0;
    for kind in GROUP_ORDER {
        let items = match kind {
            ProblemKind::DeferredSlow => deferred_slow(conn)?,
            ProblemKind::AmbiguousLineage => ambiguous(conn)?,
            // **Its own arm, and the one thing here a compiler will not catch.** Falling through
            // to `other` would query `scan_problem` for a kind that table never stores, so the
            // ninth group would report zero rows forever and vanish — a bar written past its
            // own defect.
            ProblemKind::AbandonedInstall => abandoned_installs(conn)?,
            other => scan_problems(conn, run.id, other)?,
        };
        if items.is_empty() {
            continue; // §11.1: count = 0 removes the group. Never a zeroed header.
        }
        let count: u32 = items.iter().map(|i| i.count).sum();
        if kind != ProblemKind::AmbiguousLineage {
            problem_count = problem_count.saturating_add(count);
        }
        groups.push(ProblemGroup { kind, count, items });
    }

    let ambiguous_count = groups
        .iter()
        .find(|g| g.kind == ProblemKind::AmbiguousLineage)
        .map_or(0, |g| g.count);

    Ok(Problems {
        run_id: Some(run.id),
        header: ProblemHeader {
            walked_dirs: run.walked_dirs,
            repositories: run.repositories,
            // §11.1: while a scan is in flight the clauses are omitted, never zeroed.
            problem_count: run.finished.then_some(problem_count),
            ambiguous_lineage_count: run.finished.then_some(ambiguous_count),
        },
        groups,
    })
}

fn latest_run(
    conn: &rusqlite::Connection,
    run: Option<ScanRunId>,
) -> Result<Option<RunFacts>, IndexError> {
    let sql = if run.is_some() {
        "SELECT id, walked_dirs, found_repos, ended_at FROM scan_run WHERE id = ?1"
    } else {
        "SELECT id, walked_dirs, found_repos, ended_at FROM scan_run ORDER BY id DESC LIMIT 1"
    };
    let mut stmt = conn.prepare(sql)?;
    let mut rows = match run {
        Some(id) => stmt.query([id.0])?,
        None => stmt.query([])?,
    };
    let Some(row) = rows.next()? else {
        return Ok(None);
    };
    let ended_at: Option<i64> = row.get(3)?;
    Ok(Some(RunFacts {
        id: ScanRunId(row.get(0)?),
        walked_dirs: row.get(1)?,
        repositories: row.get(2)?,
        finished: ended_at.is_some(),
    }))
}

/// One `scan_problem` row, before its stored kind has been recognised. It carries the row id
/// rather than the display string: §1.10 keeps `path_display` out of every other query.
struct StoredProblem {
    id: i64,
    detail: Option<String>,
    count: u32,
    kind: String,
}

fn scan_problems(
    conn: &rusqlite::Connection,
    run: ScanRunId,
    kind: ProblemKind,
) -> Result<Vec<ProblemItem>, IndexError> {
    let mut stmt = conn.prepare(
        "SELECT id, detail, count, kind FROM scan_problem WHERE scan_run_id = ?1 ORDER BY id",
    )?;
    let rows = stmt
        .query_map([run.0], |r| {
            Ok(StoredProblem {
                id: r.get(0)?,
                detail: r.get(1)?,
                count: r.get(2)?,
                kind: r.get(3)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let mine: Vec<StoredProblem> = rows
        .into_iter()
        .filter(|r| problem_kind_from_storage(&r.kind) == Some(kind))
        .collect();

    let ids: Vec<i64> = mine.iter().map(|r| r.id).collect();
    let displays = display_map(conn, DisplayPathTable::ScanProblem, &ids)?;

    Ok(mine
        .into_iter()
        .map(|row| ProblemItem {
            path_display: displays.get(&row.id).cloned().unwrap_or_default(),
            detail: row.detail,
            count: row.count,
            project_id: None,
            location_id: None,
            // `scan_problem` carries no location_id, so there is no observation clock to
            // reach. Absent, never 0.
            last_seen_at: None,
            candidate_project_ids: Vec::new(),
            candidate_names: Vec::new(),
        })
        .collect())
}

/// One deferred job, with whichever location the project happens to own. The location's
/// display string is resolved separately (§1.10), so only its id appears here.
struct DeferredRow {
    project_id: i64,
    location_id: Option<i64>,
    last_seen_at: Option<i64>,
    job: String,
    reason: Option<String>,
}

/// §4.1 put deferred-slow in `project_job_state`, so this group reads it there.
fn deferred_slow(conn: &rusqlite::Connection) -> Result<Vec<ProblemItem>, IndexError> {
    let mut stmt = conn.prepare(
        "SELECT s.project_id, l.id, l.last_seen_at, s.job, s.reason
         FROM project_job_state s
         LEFT JOIN location l ON l.project_id = s.project_id
         WHERE s.state = 'deferred_slow'
         GROUP BY s.project_id, s.job
         ORDER BY s.project_id, s.job",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok(DeferredRow {
                project_id: r.get(0)?,
                location_id: r.get(1)?,
                last_seen_at: r.get(2)?,
                job: r.get(3)?,
                reason: r.get(4)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    let ids: Vec<i64> = rows.iter().filter_map(|r| r.location_id).collect();
    let displays = display_map(conn, DisplayPathTable::Location, &ids)?;

    Ok(rows
        .into_iter()
        .map(|row| ProblemItem {
            path_display: location_display(&displays, row.location_id),
            detail: Some(match row.reason {
                Some(reason) => format!("{} · {reason}", row.job),
                None => row.job,
            }),
            count: 1,
            project_id: Some(ProjectId(row.project_id)),
            location_id: row.location_id.map(LocationId),
            last_seen_at: row.last_seen_at,
            candidate_project_ids: Vec::new(),
            candidate_names: Vec::new(),
        })
        .collect())
}

/// A project with no location has no path to show — an empty string, not a fabricated one.
fn location_display(
    displays: &std::collections::BTreeMap<i64, String>,
    location_id: Option<i64>,
) -> String {
    location_id
        .and_then(|id| displays.get(&id).cloned())
        .unwrap_or_default()
}

/// One project flagged ambiguous, before its candidates are counted.
struct AmbiguousSubject {
    project_id: i64,
    lineage_key: Option<String>,
    location_id: Option<i64>,
    last_seen_at: Option<i64>,
}

/// §11.1: recomputed at render time, never stored. A project whose candidates
/// have fallen back to one stops appearing, with no repair pass.
fn ambiguous(conn: &rusqlite::Connection) -> Result<Vec<ProblemItem>, IndexError> {
    let mut flagged = conn.prepare(
        "SELECT p.id, p.lineage_key, l.id, l.last_seen_at
         FROM project p LEFT JOIN location l ON l.project_id = p.id
         WHERE p.ambiguous_lineage = 1 AND p.merged_into IS NULL
         GROUP BY p.id ORDER BY p.id",
    )?;
    let subjects = flagged
        .query_map([], |r| {
            Ok(AmbiguousSubject {
                project_id: r.get(0)?,
                lineage_key: r.get(1)?,
                location_id: r.get(2)?,
                last_seen_at: r.get(3)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    let ids: Vec<i64> = subjects.iter().filter_map(|s| s.location_id).collect();
    let displays = display_map(conn, DisplayPathTable::Location, &ids)?;

    let mut out = Vec::new();
    for subject in subjects {
        let Some(lineage_key) = subject.lineage_key else {
            continue;
        };
        let mut candidates = conn.prepare(
            "SELECT id, name FROM project
             WHERE lineage_key = ?1 AND remote_key IS NOT NULL
               AND id <> ?2 AND merged_into IS NULL
             ORDER BY created_at, id",
        )?;
        let rows = candidates
            .query_map(rusqlite::params![lineage_key, subject.project_id], |r| {
                Ok((ProjectId(r.get::<_, i64>(0)?), r.get::<_, String>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        if rows.len() < AMBIGUOUS_AT_CANDIDATES {
            continue;
        }
        out.push(ProblemItem {
            path_display: location_display(&displays, subject.location_id),
            detail: None, // the sentence is shell prose, composed from the names below
            count: 1,
            project_id: Some(ProjectId(subject.project_id)),
            location_id: subject.location_id.map(LocationId),
            last_seen_at: subject.last_seen_at,
            candidate_names: rows
                .iter()
                .take(AMBIGUOUS_NAMES_SHOWN)
                .map(|(_, n)| n.clone())
                .collect(),
            candidate_project_ids: rows.into_iter().map(|(id, _)| id).collect(),
        });
    }
    Ok(out)
}

/// §24.3c: staging directories a start-up sweep could not warrant, left in place and named.
///
/// Reads `install_run`, never `scan_problem`. A run whose state is not `done` and whose staging
/// directory is still on disk is an install this app made and cannot clean up — which is a
/// problem in §11.1's ordinary sense, so it **counts** toward `problem_count` like
/// `deferred_slow` and unlike `ambiguous_lineage`.
///
/// # Errors
/// Fails when `install_run` cannot be read.
pub fn abandoned_installs(conn: &rusqlite::Connection) -> Result<Vec<ProblemItem>, IndexError> {
    let mut stmt = conn.prepare(
        "SELECT staging_bytes FROM install_run
          WHERE state IN ('running', 'failed', 'cancelled')
          ORDER BY id",
    )?;
    let rows = stmt.query_map([], |row| row.get::<_, Vec<u8>>(0))?;
    let mut items = Vec::new();
    for row in rows {
        let staging = crate::paths::path_from_bytes(&row?);
        // Only what is actually still there. A row whose directory is gone is a run that ended
        // untidily, not a directory the user has to remove.
        if !staging.exists() {
            continue;
        }
        items.push(ProblemItem {
            path_display: crate::paths::path_display(&staging),
            detail: Some(
                "an install left this behind and could not remove it; delete it by hand".to_owned(),
            ),
            count: 1,
            // `scan_problem` has no `location_id` and this group has no location at all — the
            // directory is not a location, which is the whole reason it is a problem. NULL is
            // *not observed*, never zero.
            project_id: None,
            location_id: None,
            last_seen_at: None,
            candidate_project_ids: Vec::new(),
            candidate_names: Vec::new(),
        });
    }
    Ok(items)
}
