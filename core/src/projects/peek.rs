//! §8.4.1's panel. Peek **reports**; it does not phrase — no roast, no note, no sentence, and
//! no `COMPLETION`, which nothing in phase 1 writes and which would read `unknown` on every row.
//!
//! The five facts are `BIRTH`, `LANGUAGE`, `TRACKED`, `LAST COMMIT` and `PLAYTIME`. A fact whose
//! job has not run crosses as `null` and renders `—`, never `0`; `playtimeSeconds` is the one
//! exception and its zero is true, because that ledger starts at install.

use crate::art::compose::local_year;
use crate::projects::rows::{locations_of, pick_primary, LocationFacts};
use crate::projects::{ProjectsCtx, ProjectsError};
use crate::proto::dispatch::{parse_args, CommandFailure};
use crate::protocol::{
    CommitRef, LocationRef, Peek, ProjectId, ProjectsPeekArgs, ReadmeState, ReadmeStateKind,
    WorktreeObservation,
};
use rusqlite::OptionalExtension as _;

/// The five scalar facts, read in one statement. A merged-away row is not a project the shelf
/// can peek at — `projects.get` is the surface that follows §1.6's redirect.
type ProjectFacts = (
    Option<i64>,
    Option<i64>,
    Option<String>,
    Option<i64>,
    Option<i64>,
);

fn project_facts(
    conn: &rusqlite::Connection,
    project: ProjectId,
) -> Result<ProjectFacts, ProjectsError> {
    conn.query_row(
        "SELECT first_commit_at, first_commit_tz_offset_min, primary_language,
                size_tracked_bytes, last_commit_at
           FROM project WHERE id = ?1 AND merged_into IS NULL",
        rusqlite::params![project.0],
        |r| {
            Ok((
                r.get::<_, Option<i64>>(0)?,
                r.get::<_, Option<i64>>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, Option<i64>>(3)?,
                r.get::<_, Option<i64>>(4)?,
            ))
        },
    )
    .map_err(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => ProjectsError::UnknownProject(project.0),
        other => ProjectsError::Sqlite(other),
    })
}

/// **Two states, never one string.** `not_indexed` is a promise — nothing has looked yet;
/// `absent` is a fact — J6 looked and found none. `peek_cache.computed_at` is what tells them
/// apart, and `read_at` carries it so the shell can draw the age slot or draw none at all. One
/// merged string here would render unknown as zero on the surface built to triage.
/// `pub(crate)` because §8.5.3's README panel reads the same two states from the same column —
/// one rule, two surfaces. A second copy in `crate::detail` is R12's defect with an invariant
/// behind it rather than a formatter.
pub(crate) fn readme_and_commits(
    conn: &rusqlite::Connection,
    project: ProjectId,
) -> Result<(ReadmeState, Vec<CommitRef>), ProjectsError> {
    // `.optional()`, not `.ok()`: no row means J6 has not run, and an index fault means the core
    // does not know — collapsing the two would report "nothing looked" for a failed read.
    let cached: Option<(Option<String>, Option<String>, i64)> = conn
        .query_row(
            "SELECT readme_excerpt, recent_commits_json, computed_at
               FROM peek_cache WHERE project_id = ?1",
            rusqlite::params![project.0],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;

    let Some((excerpt, commits_json, computed_at)) = cached else {
        return Ok((
            ReadmeState {
                state: ReadmeStateKind::NotIndexed,
                text: None,
                read_at: None,
            },
            Vec::new(),
        ));
    };

    let readme = match excerpt {
        None => ReadmeState {
            state: ReadmeStateKind::Absent,
            text: None,
            read_at: Some(computed_at),
        },
        Some(text) => ReadmeState {
            state: ReadmeStateKind::Present,
            text: Some(text),
            read_at: Some(computed_at),
        },
    };

    // A sidecar this build cannot read is no commits, not a failed peek: the panel still has
    // five facts to report, and the column is J6's cache rather than the source of truth.
    let commits = commits_json
        .and_then(|json| serde_json::from_str::<Vec<CommitRef>>(&json).ok())
        .unwrap_or_default();
    Ok((readme, commits))
}

/// §6: an observation the primary copy does not carry is `None`, never a `0` and never a
/// `false`. Absence of dirty means "no changes as of T", and here there is no T.
fn worktree(primary: Option<&LocationFacts>) -> WorktreeObservation {
    WorktreeObservation {
        observed_at: primary.and_then(|l| l.worktree_observed_at),
        is_dirty: primary.and_then(|l| l.is_dirty),
        untracked_count: primary
            .and_then(|l| l.untracked_count)
            .and_then(|n| u32::try_from(n).ok()),
    }
}

/// §8.4.1's payload for one project.
///
/// # Errors
/// `UnknownProject` when the id names no live project; `Sqlite` for anything the index refuses.
pub fn load_peek(ctx: &ProjectsCtx<'_>, project: ProjectId) -> Result<Peek, ProjectsError> {
    let conn = ctx.index.conn();
    let (first_commit_at, first_commit_tz, primary_language, size_tracked_bytes, last_commit_at) =
        project_facts(conn, project)?;

    let locations = locations_of(conn, project)?;
    let primary = pick_primary(&locations);
    let (readme, commits) = readme_and_commits(conn, project)?;

    // [p2] §23.3: **Peek may not send `not_indexed`** for a project with no location. J6 can
    // never look at a repository that was never cloned, so the promise cannot be kept, and
    // `ReadmeStateKind` gains no fourth variant by ruling. What is left is `absent`, and the
    // surface renders **no README element at all** for such a row (§25.3a) — so the value
    // reaches no sentence. Recorded rather than assumed: neither remaining variant is a true
    // statement about a repository nothing has read, and the ruling picks the one that is not a
    // promise.
    let readme = if primary.is_none() {
        ReadmeState {
            state: ReadmeStateKind::Absent,
            text: None,
            read_at: None,
        }
    } else {
        readme
    };

    Ok(Peek {
        id: project,
        readme,
        commits,
        // §25.3a's positive half: what a not-cloned row renders **in place of** the five facts
        // it has no history for. NULL iff `remote_key` is NULL, the same predicate
        // `ProjectDetail.remote` carries — one producer, two surfaces.
        remote: crate::remote::facts::remote_facts(conn, project).map_err(ProjectsError::Index)?,
        location: primary.map(|l| LocationRef {
            id: l.id,
            path_display: l.path_display.clone(),
        }),
        worktree: worktree(primary),
        birth_year: first_commit_at.and_then(|at| {
            let offset = i32::try_from(first_commit_tz.unwrap_or(0)).unwrap_or(0);
            u32::try_from(local_year(at, offset)).ok()
        }),
        primary_language,
        size_tracked_bytes,
        last_commit_at,
        // §8.4.1's one honest zero: this ledger starts at install, so "nothing has been
        // launched" is a measurement and not an absence. It is never summed with a git fact.
        playtime_seconds: crate::session::store::playtime_seconds(conn, project).unwrap_or(0),
        interrupted_op: primary.and_then(|l| l.interrupted_op),
    })
}

/// `projects.peek`.
///
/// # Errors
/// `PROTOCOL` for an argument shape the schema does not admit or an id that names no project;
/// `INTERNAL` for an index fault.
pub fn handle(
    ctx: &ProjectsCtx<'_>,
    args: serde_json::Value,
) -> Result<serde_json::Value, CommandFailure> {
    let args: ProjectsPeekArgs = parse_args(args)?;
    let peek = load_peek(ctx, args.id)?;
    // §6: a Peek asks for a current worktree reading for the copy it is showing. The answer
    // below is still the **stored** one with its own `as_of`; the job updates it and publishes
    // a change. Nothing here waits, and nothing here claims currency it does not have.
    // §21.5, and **outside the location guard**: a not-cloned project has no location and is the
    // row whose Peek is made entirely of remote facts.
    ctx.sync.on_project_visible(peek.id);
    if let Some(location) = peek.location.as_ref() {
        crate::jobs::visible::notify_visible(ctx.index, ctx.mounts, ctx.jobs, peek.id, location.id);
    }
    serde_json::to_value(peek).map_err(|e| CommandFailure::internal(e.to_string()))
}
