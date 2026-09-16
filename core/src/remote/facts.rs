//! §25.1 — one reader, four states.
//!
//! **The four states are a wire enum, never a rendering rule the renderer re-derives.** Staleness
//! is the one thing computed at render time, because it is a function of *now*; everything else
//! is a decision the core made with the row in front of it.
//!
//! The reader carries the **stored** values under every state, and only `state` changes. A fact
//! observed while an account was connected does not stop being an observation when the account
//! goes away, and §25.3's identity-line visibility and topic rail read it from here — what
//! `no_account` governs is §25.1's three forge blocks and the CI list, which the renderer draws
//! from the state and not from the presence of a number.

use rusqlite::{Connection, OptionalExtension as _};

use crate::index::IndexError;
use crate::protocol::{CiList, CiRun, ProjectId, RemoteFacts, RemoteFactsState, RemoteVisibility};
use crate::remote::weburl::{enterprise_hosts, is_allowlisted_host};

/// At most five runs per pair (§25.7). The renderer holds the same bound for its own list; this
/// one is the read's, so a row set that was never trimmed still cannot overflow the surface.
pub const CI_RUN_LIMIT: usize = 5;

/// The binding and the key a facts read starts from.
struct ProjectRemote {
    remote_key: String,
    provider: Option<String>,
    provider_repo_id: Option<String>,
}

/// One `remote_repo` row, in column order.
struct FactsRow {
    visibility: Option<String>,
    fork_parent_remote_key: Option<String>,
    stars: Option<i64>,
    open_issues: Option<i64>,
    good_first_issues: Option<i64>,
    open_prs: Option<i64>,
    open_prs_from_user: Option<i64>,
    permitted: i64,
    observed_at: Option<i64>,
    ci_observed_at: Option<i64>,
}

fn project_remote(
    conn: &Connection,
    project: ProjectId,
) -> Result<Option<ProjectRemote>, IndexError> {
    let row: Option<(Option<String>, Option<String>, Option<String>)> = conn
        .query_row(
            "SELECT remote_key, provider, provider_repo_id
               FROM project WHERE id = ?1 AND merged_into IS NULL",
            rusqlite::params![project.0],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    let Some((Some(remote_key), provider, provider_repo_id)) = row else {
        return Ok(None);
    };
    Ok(Some(ProjectRemote {
        remote_key,
        provider,
        provider_repo_id,
    }))
}

fn facts_row(
    conn: &Connection,
    provider: &str,
    provider_repo_id: &str,
) -> Result<Option<FactsRow>, IndexError> {
    conn.query_row(
        "SELECT visibility, fork_parent_remote_key, stars, open_issues, good_first_issues,
                open_prs, open_prs_from_user, permitted, observed_at, ci_observed_at
           FROM remote_repo WHERE provider = ?1 AND provider_repo_id = ?2",
        rusqlite::params![provider, provider_repo_id],
        |r| {
            Ok(FactsRow {
                visibility: r.get(0)?,
                fork_parent_remote_key: r.get(1)?,
                stars: r.get(2)?,
                open_issues: r.get(3)?,
                good_first_issues: r.get(4)?,
                open_prs: r.get(5)?,
                open_prs_from_user: r.get(6)?,
                permitted: r.get(7)?,
                observed_at: r.get(8)?,
                ci_observed_at: r.get(9)?,
            })
        },
    )
    .optional()
    .map_err(IndexError::from)
}

fn topics(
    conn: &Connection,
    provider: &str,
    provider_repo_id: &str,
) -> Result<Vec<String>, IndexError> {
    let mut stmt = conn.prepare(
        "SELECT topic FROM remote_topic
          WHERE provider = ?1 AND provider_repo_id = ?2 ORDER BY topic",
    )?;
    let rows = stmt.query_map(rusqlite::params![provider, provider_repo_id], |r| r.get(0))?;
    rows.collect::<Result<Vec<String>, _>>().map_err(Into::into)
}

/// Most recent first, bounded to [`CI_RUN_LIMIT`].
///
/// `started_at` DESC puts an unstarted run last in SQLite, which is where an unstarted run
/// belongs on a list headed *latest*; `run_id` breaks the tie so the order is total.
fn ci_runs(
    conn: &Connection,
    provider: &str,
    provider_repo_id: &str,
) -> Result<Vec<CiRun>, IndexError> {
    let mut stmt = conn.prepare(
        "SELECT run_id, workflow_name, conclusion, branch, run_number, started_at
           FROM remote_ci_run
          WHERE provider = ?1 AND provider_repo_id = ?2
          ORDER BY started_at DESC, run_id DESC
          LIMIT ?3",
    )?;
    let rows = stmt.query_map(
        rusqlite::params![
            provider,
            provider_repo_id,
            i64::try_from(CI_RUN_LIMIT).unwrap_or(5)
        ],
        |r| {
            Ok(CiRun {
                run_id: r.get(0)?,
                workflow: r.get(1)?,
                conclusion: r.get(2)?,
                branch: r.get(3)?,
                run_number: u32::try_from(r.get::<_, i64>(4)?).unwrap_or(0),
                started_at: r.get(5)?,
            })
        },
    )?;
    rows.collect::<Result<Vec<CiRun>, _>>().map_err(Into::into)
}

fn any_account(conn: &Connection) -> Result<bool, IndexError> {
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM account", [], |r| r.get(0))?;
    Ok(count > 0)
}

/// A count column into the wire's `u32?`. A stored negative is not a count and is not rendered
/// as one — it reads as unobserved rather than as a number nobody can explain.
fn count(value: Option<i64>) -> Option<u32> {
    value.and_then(|n| u32::try_from(n).ok())
}

/// §25.1's state for one read, given whether an account exists, whether the row permits the
/// read, and whether that read's own clock has ever moved.
fn state_of(connected: bool, permitted: bool, observed_at: Option<i64>) -> RemoteFactsState {
    if !connected {
        return RemoteFactsState::NoAccount;
    }
    if !permitted {
        return RemoteFactsState::NotPermitted;
    }
    if observed_at.is_none() {
        return RemoteFactsState::NotObserved;
    }
    RemoteFactsState::Observed
}

/// §25.1's facts for one project, or `None` when it has no remote at all.
///
/// `None` **iff** `project.remote_key` is NULL, which is the `REMOTE` tab's presence predicate
/// and the rule `ProjectDetail.remote` and `Peek.remote` both carry.
///
/// # Errors
/// Fails when the index cannot be read.
pub fn remote_facts(
    conn: &Connection,
    project: ProjectId,
) -> Result<Option<RemoteFacts>, IndexError> {
    let Some(binding) = project_remote(conn, project)? else {
        return Ok(None);
    };
    let host = binding.remote_key.split('/').next().unwrap_or_default();
    let linkable = is_allowlisted_host(host, &enterprise_hosts(conn)?);
    let connected = any_account(conn)?;

    // A12: no facts row means no clock, and the honest render is *not observed*. A project with
    // no binding has no row to find, which is the same answer for the same reason.
    let row = match (
        binding.provider.as_deref(),
        binding.provider_repo_id.as_deref(),
    ) {
        (Some(provider), Some(repo_id)) => facts_row(conn, provider, repo_id)?,
        _ => None,
    };
    let (topics, runs) = match (
        binding.provider.as_deref(),
        binding.provider_repo_id.as_deref(),
    ) {
        (Some(provider), Some(repo_id)) => (
            topics(conn, provider, repo_id)?,
            ci_runs(conn, provider, repo_id)?,
        ),
        _ => (Vec::new(), Vec::new()),
    };

    let permitted = row.as_ref().map_or(true, |r| r.permitted != 0);
    let observed_at = row.as_ref().and_then(|r| r.observed_at);
    let ci_observed_at = row.as_ref().and_then(|r| r.ci_observed_at);

    Ok(Some(RemoteFacts {
        key: binding.remote_key,
        linkable,
        state: state_of(connected, permitted, observed_at),
        visibility: row
            .as_ref()
            .and_then(|r| r.visibility.as_deref())
            .and_then(visibility_of),
        fork_parent_key: row.as_ref().and_then(|r| r.fork_parent_remote_key.clone()),
        stars: count(row.as_ref().and_then(|r| r.stars)),
        open_issues: count(row.as_ref().and_then(|r| r.open_issues)),
        good_first_issues: count(row.as_ref().and_then(|r| r.good_first_issues)),
        open_prs: count(row.as_ref().and_then(|r| r.open_prs)),
        open_prs_from_user: count(row.as_ref().and_then(|r| r.open_prs_from_user)),
        topics,
        observed_at,
        ci: CiList {
            state: state_of(connected, permitted, ci_observed_at),
            runs,
            observed_at: ci_observed_at,
        },
    }))
}

/// A stored visibility this build does not know is **unknown**, not a guess. §25.3 renders both
/// words or neither, so an unrecognised third value must render nothing rather than one of them.
fn visibility_of(stored: &str) -> Option<RemoteVisibility> {
    match stored {
        "public" => Some(RemoteVisibility::Public),
        "private" => Some(RemoteVisibility::Private),
        _ => None,
    }
}
