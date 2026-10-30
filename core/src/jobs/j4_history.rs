//! J4 — history (§4.1): the root set, the first commit, commit-**days** and release days.
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

use std::collections::{BTreeMap, BTreeSet};

use rusqlite::Transaction;

use super::JobError;
use crate::clock::{local_date, local_day, LocalInstant};
use crate::git::{Authorship, GitBackend, JobContext, RepoHandle, TagFact, TagKind};
use crate::identity::user::IdentitySet;
use crate::index::IndexError;
use crate::ledger::git_rows::{reconcile_git_rows, GitRow};
use crate::ledger::receipt::Receipt;
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
    /// §38.8.6: each local day on which a tag of the user's peels to a commit, to that day's
    /// first such tag name in byte order. **One per day, never a tag count.** `None` when the
    /// tags were not read — a Reference project's — and then no `release` row is written or
    /// deleted on their account.
    pub releases: Option<BTreeMap<i64, String>>,
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

/// §38.2's key for one release date, in §1.7's git shape — the commit-day key's, under
/// `release:`.
#[must_use]
pub fn release_dedupe_key(lineage: &str, remote: Option<&str>, date: &str) -> String {
    format!("release:{lineage}:{}:{date}", remote.unwrap_or(""))
}

/// §38.8.6: the local days on which a qualifying tag falls, each to the day's first qualifying
/// tag name in byte order.
///
/// An annotated tag qualifies by its tagger and is dated in the tagger's zone; a lightweight one
/// by the tagged commit's committer, dated as that commit's commit-day is. Pure, like
/// [`fold_authorship`], so the rule is testable without a repository.
#[must_use]
pub fn release_days(tags: &[TagFact], ids: &IdentitySet) -> BTreeMap<i64, String> {
    let mut days: BTreeMap<i64, String> = BTreeMap::new();
    for tag in tags.iter().filter(|tag| !is_product_archive_tag(tag)) {
        let (email, at, offset) = match &tag.kind {
            TagKind::Annotated {
                tagger_email,
                tagged_at,
                tz_offset_min,
            } => (tagger_email, *tagged_at, *tz_offset_min),
            TagKind::Lightweight {
                committer_email,
                authored_at,
                tz_offset_min,
            } => (committer_email, *authored_at, *tz_offset_min),
        };
        if !ids.contains(email) {
            continue;
        }
        days.entry(local_day(at, offset))
            .and_modify(|first| {
                if tag.name < *first {
                    first.clone_from(&tag.name);
                }
            })
            .or_insert_with(|| tag.name.clone());
    }
    days
}

/// §45.2a's exclusion: the tag the product itself writes when it archives a project is not a
/// release. It is decided by the name grammar and re-derivation together, never by the name
/// alone — a user's own tag named `archived` is a release — and until that predicate exists no
/// tag is excluded.
const fn is_product_archive_tag(_tag: &TagFact) -> bool {
    false
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

/// Read one project's history, its tags included unless `reference` — a Reference project
/// writes no git row, so its tags are never read.
///
/// A tag read that fails fails the whole read: the settle reconciles the git rows to what the
/// history implies, so a history read without its tags would delete every standing `release`
/// row. Tags that could not be read are not tags that are absent.
///
/// # Errors
///
/// `JobError::Git` carrying the failure of the root, authorship, subject or tag read.
pub fn observe(
    git: &dyn GitBackend,
    repo: &RepoHandle,
    identities: &IdentitySet,
    reference: bool,
    ctx: &JobContext<'_>,
) -> Result<(RootFacts, HistoryFacts), JobError> {
    let roots = read_root_facts(git, repo, ctx)?;
    let mut facts = fold_authorship(&git.authorship(repo, ctx)?, identities);
    facts.recent_subjects = git
        .commit_subjects(repo, SUBJECT_LIMIT, ctx)?
        .into_iter()
        .map(|c| c.subject)
        .collect();
    if !reference {
        facts.releases = Some(release_days(&git.release_tags(repo, ctx)?, identities));
    }
    Ok((roots, facts))
}

/// §1.1's roots, with `first_commit_at` taken from the **earliest** of them.
///
/// # Errors
///
/// `JobError::Git` carrying the root read's failure.
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

/// The `commit_day` rows `facts` implies, one per day, keyed by [`dedupe_key`].
///
/// §1.7: git-derived events are a pure function of history. The settle reconciles to these; they
/// are deleted-and-recomputed, never migrated.
///
/// `track = 'git'` is the class that recomputes; `track = 'session'` is reparented and never
/// recomputed, and the two must never be confused.
///
/// Keyed on a lineage, so a project without one has none: no lineage yet means no stable key,
/// and the settle writes the rows when the roots resolve.
#[must_use]
pub fn commit_days(lineage: &str, remote_key: Option<&str>, facts: &HistoryFacts) -> Vec<GitRow> {
    let subject = subject_key(lineage, remote_key);
    facts
        .days
        .iter()
        .map(|&day| {
            let date = local_date(day);
            GitRow {
                kind: "commit_day",
                dedupe_key: dedupe_key(lineage, remote_key, &date),
                subject_key: subject.clone(),
                ts: day_ts(day),
                meta: serde_json::json!({ "when": date }).to_string(),
            }
        })
        .collect()
}

/// §38.8.6: the `release` rows `facts` implies, one per release day, each carrying its receipt —
/// the day's first tag name, the date, and `project_name` as it stands at the write.
#[must_use]
pub fn releases(
    lineage: &str,
    remote: Option<&str>,
    project_name: &str,
    facts: &HistoryFacts,
) -> Vec<GitRow> {
    let subject = subject_key(lineage, remote);
    facts
        .releases
        .iter()
        .flatten()
        .map(|(&day, name)| {
            let at = LocalInstant {
                unix: day_ts(day),
                offset_min: 0,
            };
            GitRow {
                kind: "release",
                dedupe_key: release_dedupe_key(lineage, remote, &local_date(day)),
                subject_key: subject.clone(),
                ts: at.unix,
                meta: Receipt::new(name, at, project_name).to_meta(),
            }
        })
        .collect()
}

/// `project`'s `release` rows as they stand, implied again by a settle that did not read tags.
fn standing_releases(tx: &Transaction<'_>, project: ProjectId) -> Result<Vec<GitRow>, IndexError> {
    let mut statement = tx.prepare(
        "SELECT dedupe_key, subject_key, ts, meta FROM xp_events
          WHERE project_id = ?1 AND track = 'git' AND kind = 'release'",
    )?;
    let rows = statement
        .query_map([project.0], |r| {
            Ok(GitRow {
                kind: "release",
                dedupe_key: r.get(0)?,
                subject_key: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                ts: r.get(2)?,
                meta: r.get(3)?,
            })
        })?
        .collect::<Result<_, _>>()?;
    Ok(rows)
}

/// The day's midnight in the day's own frame. The offset is not recoverable from a day number,
/// and 0 is the honest answer for "this is a day, not a moment".
const fn day_ts(day: i64) -> i64 {
    day.saturating_mul(86_400)
}

/// §28.4a: **one subject key, not two.** This rendered `{lineage}:{remote}` while
/// `ProjectSubject::to_key()` — the sidecar's writer, and `debt_day`'s — rendered
/// `lineage:{k}|remote:{r}` for the same logical subject, so two ledgers keyed one project two
/// ways. It does **not** switch to `subject_for_project`: that returns a `Path` subject for a
/// project with no lineage, and a git row is written only under a lineage, which is what makes
/// §1.7's keys safe. The dedupe keys are different strings and deliberately unchanged.
fn subject_key(lineage: &str, remote: Option<&str>) -> String {
    crate::index::subject::ProjectSubject::Lineage {
        lineage_key: lineage.to_owned(),
        remote_key: remote.map(str::to_owned),
    }
    .to_key()
}

impl HistoryFacts {
    /// The newest subject, for `project.last_commit_subject`.
    #[must_use]
    pub fn last_commit_subject(&self) -> Option<&str> {
        self.recent_subjects.first().map(String::as_str)
    }
}

/// Write everything J4 owns: the history columns §1.2 declares, the git rows, and the subject
/// index.
///
/// §38.7.4: the git rows are reconciled to exactly what the history implies — `commit_day` and
/// `release` alike — so a dropped address, a rewritten history or a returning copy leaves no
/// row nothing implies. A Reference project implies none, so its rows empty while its history is
/// still walked. A project with no lineage has no stable key, and its rows are left alone.
///
/// Reads `lineage_key`, `remote_key`, `is_reference` and `name` from the row rather than taking
/// them, so the dedupe key cannot be built from a stale copy the caller was holding.
///
/// # Errors
///
/// `IndexError::Sqlite` when the project row cannot be read (including a project that is not
/// there) or any write fails.
pub fn commit_history(
    tx: &Transaction<'_>,
    project: ProjectId,
    roots: &RootFacts,
    facts: &HistoryFacts,
    now: i64,
) -> Result<(), IndexError> {
    let (lineage, remote, reference, name): (Option<String>, Option<String>, bool, String) = tx
        .query_row(
            "SELECT lineage_key, remote_key, is_reference, name FROM project WHERE id = ?1",
            [project.0],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )?;
    if let Some(lineage) = lineage.as_deref() {
        let implied = if reference {
            Vec::new()
        } else {
            let mut rows = commit_days(lineage, remote.as_deref(), facts);
            if facts.releases.is_some() {
                rows.extend(releases(lineage, remote.as_deref(), &name, facts));
            } else {
                // The tags were not read — the project read as Reference and is authored again
                // by this write — so the standing `release` rows stand, and none is deleted.
                rows.extend(standing_releases(tx, project)?);
            }
            rows
        };
        // First, so the settle reads whether it is silent before its own writes.
        reconcile_git_rows(tx, project, &implied, now)?;
    }
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
    use crate::git::CommitterTally;

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
        assert_eq!(facts.days, std::iter::once(18_518).collect());
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
