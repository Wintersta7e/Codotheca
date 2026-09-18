//! J4 — history (§4.1): the root set, the first commit, and commit-**days**.
//!
//! **Never reward volume.** J4 produces commit-days and nothing here stores a commit count:
//! three commits on one day is one day, and deleting code counts as writing it.
//!
//! Two deviations from the plan body, both because plan 05 already owns the walk:
//!
//! * `parse_tz_offset`, `history_slice` and `read_subjects` are gone. The offset parser is
//!   `core::git::parse_tz_offset_min`, and the walks are `GitBackend::{authorship,
//!   root_commits, commit_subjects}`. Notably git has **no `%az` placeholder** — the plan's
//!   format string would have emitted the literal text `%az` and every commit-day would have
//!   silently vanished; plan 05 already found and fixed that.
//! * §4.1's chunking is not reachable, for the same reason as J3: the backend's walks are
//!   atomic calls with no yield point. `HistoryAcc.consumed` therefore does not exist rather
//!   than existing and never advancing.
//!
//! J4 and J1.5 read the same `authorship` walk, so scheduling J1.5 first costs J4 nothing.

use std::collections::BTreeSet;

use rusqlite::Transaction;

use super::JobError;
use crate::git::{Authorship, GitBackend, JobContext, RepoHandle};
use crate::identity::user::IdentitySet;
use crate::index::IndexError;
use crate::protocol::ProjectId;

/// How many recent subjects `fts_commits` and `peek_cache` keep (§1.9).
pub const SUBJECT_LIMIT: u32 = 200;

/// §1.1's root set and the dates that follow from it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RootFacts {
    /// The root commits, sorted and deduplicated.
    pub root_oids: Vec<String>,
    /// The **earliest** root by commit date, never the first listed.
    pub first_commit_at: Option<i64>,
    /// Its offset, so §1.2's local day survives an INTEGER epoch.
    pub first_commit_tz_offset_min: Option<i32>,
    /// Its object id.
    pub first_commit_sha: Option<String>,
}

/// What the history walk concluded.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HistoryFacts {
    /// Local day numbers on which a user identity committed. **Days, never counts.**
    pub days: BTreeSet<i64>,
    /// Any author. §5.1 uses it for "dusty but lit".
    pub last_commit_at: Option<i64>,
    /// §5.1's first input to `last_touched_at` — the user's own last commit, which is not
    /// `last_commit_at`: a CI bot's commit must not read as the user having touched the project.
    pub last_user_commit_at: Option<i64>,
    /// The most recent subjects, newest first, for `fts_commits` and `peek_cache` (§1.9).
    ///
    /// `project.last_commit_subject` is the first of these rather than a field of its own: two
    /// places holding the same subject is how they end up disagreeing.
    pub recent_subjects: Vec<String>,
}

/// Civil date, `YYYY-MM-DD`, from a day number as [`local_day`] produces one.
///
/// The day number is the storable form and this is the human one; the dedupe key needs a stable
/// string, and a bare integer in it would be unreadable in the one place a person ever looks.
#[must_use]
pub fn local_date(day: i64) -> String {
    // Howard Hinnant's civil_from_days.
    let z = day + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

/// §1.7's idempotency key for one commit-day.
///
/// Keyed on the remote as well as the lineage: v2 keyed on `lineage_key` alone — the one value a
/// fork and its upstream deliberately share — so a commit-day in each on the same date collided
/// and one of them silently vanished.
#[must_use]
pub fn dedupe_key(lineage_key: &str, remote_key: Option<&str>, local_date: &str) -> String {
    format!(
        "commit_day:{lineage_key}:{}:{local_date}",
        remote_key.unwrap_or("")
    )
}

/// Fold a committer walk against the identity set.
///
/// Pure, so the whole "days not counts" rule is testable without a repository.
#[must_use]
pub fn fold_authorship(walked: &Authorship, identities: &IdentitySet) -> HistoryFacts {
    let mut facts = HistoryFacts::default();
    for tally in &walked.committers {
        facts.last_commit_at = Some(
            facts
                .last_commit_at
                .map_or(tally.last_commit_at, |c| c.max(tally.last_commit_at)),
        );
        if !identities.contains(&tally.email) {
            continue;
        }
        facts.last_user_commit_at = Some(
            facts
                .last_user_commit_at
                .map_or(tally.last_commit_at, |c| c.max(tally.last_commit_at)),
        );
        facts.days.extend(tally.days.iter().copied());
    }
    facts
}

/// Read one project's history.
pub fn observe(
    git: &dyn GitBackend,
    repo: &RepoHandle,
    identities: &IdentitySet,
    ctx: &JobContext<'_>,
) -> Result<(RootFacts, HistoryFacts), JobError> {
    let roots = read_root_facts(git, repo, ctx)?;
    let mut facts = fold_authorship(&git.authorship(repo, ctx)?, identities);
    facts.recent_subjects = git
        .commit_subjects(repo, SUBJECT_LIMIT, ctx)?
        .into_iter()
        .map(|c| c.subject)
        .collect();
    Ok((roots, facts))
}

/// §1.1's roots, with `first_commit_at` taken from the **earliest** of them.
pub fn read_root_facts(
    git: &dyn GitBackend,
    repo: &RepoHandle,
    ctx: &JobContext<'_>,
) -> Result<RootFacts, JobError> {
    let mut roots = git.root_commits(repo, ctx)?;
    roots.sort_by(|a, b| a.oid.cmp(&b.oid));
    roots.dedup_by(|a, b| a.oid == b.oid);

    let earliest = roots.iter().min_by_key(|r| r.committed_at);
    Ok(RootFacts {
        first_commit_at: earliest.map(|r| r.committed_at),
        first_commit_tz_offset_min: earliest.map(|r| r.tz_offset_min),
        first_commit_sha: earliest.map(|r| r.oid.clone()),
        root_oids: roots.into_iter().map(|r| r.oid).collect(),
    })
}

/// §1.7: git-derived events are a pure function of history. They are inserted idempotently on
/// [`dedupe_key`] and are deleted-and-recomputed, never migrated.
///
/// `track = 'git'` is the class that recomputes; `track = 'session'` is reparented and never
/// recomputed, and the two must never be confused.
pub fn commit_days(
    tx: &Transaction<'_>,
    project: ProjectId,
    lineage_key: Option<&str>,
    remote_key: Option<&str>,
    facts: &HistoryFacts,
) -> Result<usize, IndexError> {
    let Some(lineage) = lineage_key else {
        // No lineage yet means no stable key; the rows are written when the roots resolve.
        return Ok(0);
    };
    // §28.4a: **one subject key, not two.** This rendered `{lineage}:{remote}` while
    // `ProjectSubject::to_key()` — the sidecar's writer, and `debt_day`'s — rendered
    // `lineage:{k}|remote:{r}` for the same logical subject, so two ledgers keyed one project two
    // ways. It does **not** switch to `subject_for_project`: that returns a `Path` subject for a
    // project with no lineage, and the early return above is what makes §1.7's `commit_day` key
    // safe. `dedupe_key` is a different string and is deliberately unchanged.
    let subject = crate::index::subject::ProjectSubject::Lineage {
        lineage_key: lineage.to_owned(),
        remote_key: remote_key.map(str::to_owned),
    }
    .to_key();
    let mut stmt = tx.prepare(
        "INSERT INTO xp_events
            (ts, tz_offset_min, project_id, subject_key, kind, dedupe_key, track, meta)
         VALUES (?1, ?2, ?3, ?4, 'commit_day', ?5, 'git', ?6)
         ON CONFLICT(dedupe_key) DO NOTHING",
    )?;
    let mut written = 0;
    for day in &facts.days {
        let date = local_date(*day);
        // The day's midnight in the day's own frame. The offset is not recoverable from a day
        // number, and 0 is the honest answer for "this is a day, not a moment".
        let ts = day.saturating_mul(86_400);
        let meta = serde_json::json!({ "when": date }).to_string();
        written += stmt.execute(rusqlite::params![
            ts,
            0_i64,
            project.0,
            subject,
            dedupe_key(lineage, remote_key, &date),
            meta,
        ])?;
    }
    Ok(written)
}

impl HistoryFacts {
    /// The newest subject, for `project.last_commit_subject`.
    #[must_use]
    pub fn last_commit_subject(&self) -> Option<&str> {
        self.recent_subjects.first().map(String::as_str)
    }
}

/// Write everything J4 owns: the history columns §1.2 declares, the commit-days, and the
/// subject index.
///
/// Reads `lineage_key` and `remote_key` from the row rather than taking them, so the dedupe key
/// cannot be built from a stale copy the caller was holding.
pub fn commit_history(
    tx: &Transaction<'_>,
    project: ProjectId,
    roots: &RootFacts,
    facts: &HistoryFacts,
) -> Result<(), IndexError> {
    let (lineage, remote): (Option<String>, Option<String>) = tx.query_row(
        "SELECT lineage_key, remote_key FROM project WHERE id = ?1",
        [project.0],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    commit_days(tx, project, lineage.as_deref(), remote.as_deref(), facts)?;
    tx.execute(
        "INSERT INTO fts_commits (project_id, subjects) VALUES (?1, ?2)
         ON CONFLICT(project_id) DO UPDATE SET subjects = excluded.subjects",
        rusqlite::params![project.0, facts.recent_subjects.join("\n")],
    )?;
    tx.execute(
        "UPDATE project
            SET first_commit_at = ?2, first_commit_tz_offset_min = ?3, first_commit_sha = ?4,
                last_commit_at = ?5, last_commit_subject = ?6, last_user_commit_at = ?7
          WHERE id = ?1",
        rusqlite::params![
            project.0,
            roots.first_commit_at,
            roots.first_commit_tz_offset_min,
            roots.first_commit_sha,
            facts.last_commit_at,
            facts.last_commit_subject(),
            facts.last_user_commit_at,
        ],
    )?;
    Ok(())
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
    use crate::git::{local_day, CommitterTally};

    fn ids(list: &[&str]) -> IdentitySet {
        IdentitySet::from_emails(list.iter().map(|s| (*s).to_owned()))
    }

    fn tally(email: &str, commits: u32, days: &[i64], last: i64) -> CommitterTally {
        CommitterTally {
            email: email.to_owned(),
            commits,
            days: days.iter().copied().collect(),
            last_commit_at: last,
        }
    }

    #[test]
    fn a_day_is_the_local_day_which_is_why_the_offset_is_stored() {
        // §1.2: an INTEGER epoch alone cannot preserve a local day.
        // 1700000000 is 2023-11-14 22:13:20 UTC — already the 15th at +0530.
        assert_eq!(local_date(local_day(1_700_000_000, 0)), "2023-11-14");
        assert_eq!(local_date(local_day(1_700_000_000, 330)), "2023-11-15");
        assert_eq!(local_date(0), "1970-01-01");
        assert_eq!(local_date(-1), "1969-12-31");
    }

    #[test]
    fn commit_days_are_days_and_nothing_stores_a_commit_count() {
        // Never reward volume: J4 produces commit-DAYS. Three commits on one day is one day.
        let walked = Authorship {
            committers: vec![tally("me@x", 3, &[19_675], 1_700_000_000)],
        };
        let facts = fold_authorship(&walked, &ids(&["me@x"]));
        assert_eq!(facts.days.len(), 1);
    }

    #[test]
    fn only_the_users_commits_become_commit_days() {
        let walked = Authorship {
            committers: vec![tally("someone@else", 1, &[19_675], 1_700_000_000)],
        };
        let facts = fold_authorship(&walked, &ids(&["me@x"]));
        assert!(facts.days.is_empty());
        assert_eq!(
            facts.last_commit_at,
            Some(1_700_000_000),
            "any author sets last_commit_at"
        );
        assert_eq!(
            facts.last_user_commit_at, None,
            "§5.1's first input is the user's own"
        );
    }

    #[test]
    fn last_user_commit_at_is_not_last_commit_at() {
        let walked = Authorship {
            committers: vec![
                tally("me@x", 1, &[18_518], 1_600_000_000),
                tally("bot@ci", 1, &[19_675], 1_700_000_000),
            ],
        };
        let facts = fold_authorship(&walked, &ids(&["me@x"]));
        assert_eq!(facts.last_commit_at, Some(1_700_000_000));
        assert_eq!(facts.last_user_commit_at, Some(1_600_000_000));
        assert_eq!(facts.days, [18_518].into_iter().collect());
    }

    /// Two of the user's addresses committing on one day is still one day.
    #[test]
    fn two_user_addresses_on_one_day_are_one_commit_day() {
        let walked = Authorship {
            committers: vec![
                tally("me@x", 4, &[19_675], 1_700_000_000),
                tally("me@work", 9, &[19_675], 1_700_000_500),
            ],
        };
        let facts = fold_authorship(&walked, &ids(&["me@x", "me@work"]));
        assert_eq!(facts.days.len(), 1);
        assert_eq!(facts.last_user_commit_at, Some(1_700_000_500));
    }

    #[test]
    fn an_empty_history_leaves_every_date_not_computed() {
        let facts = fold_authorship(&Authorship::default(), &ids(&["me@x"]));
        assert_eq!(facts.last_commit_at, None);
        assert_eq!(facts.last_user_commit_at, None);
        assert!(facts.days.is_empty());
    }

    #[test]
    fn the_dedupe_key_separates_a_fork_from_its_upstream() {
        // §1.7: v2 keyed on lineage_key alone — the one value a fork and its upstream
        // deliberately share — so a commit-day in each on the same date collided.
        let a = dedupe_key("lin", Some("host/owner-a/repo"), "2024-01-02");
        let b = dedupe_key("lin", Some("host/owner-b/repo"), "2024-01-02");
        assert_ne!(a, b);
        assert_eq!(a, "commit_day:lin:host/owner-a/repo:2024-01-02");
        assert_eq!(
            dedupe_key("lin", None, "2024-01-02"),
            "commit_day:lin::2024-01-02"
        );
    }
}
