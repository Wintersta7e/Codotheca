//! §38.3a: the XP function, computed at read time from `xp_events` and `project` and never
//! stored as a total.
//!
//! There is no read-time filter on hidden, archived or tombstoned projects, and rows with no
//! project count: eligibility is the writers'. The one read-time exclusion is a shallow clone's
//! history — a truncated log fakes gaps and first languages — so commit-days, revivals and
//! languages read only projects with `is_shallow = 0`.

use std::collections::{BTreeMap, BTreeSet};

use chrono::Datelike as _;
use rusqlite::Connection;

use super::price::{is_revival_gap, prices, PriceTable};
use crate::index::IndexError;
use crate::protocol::{ProjectId, XpTerm};

const DAY: i64 = 86_400;

/// A project (or none) and a calendar `(year, month)`: the unit the release cap counts in.
type ProjectMonth = (Option<i64>, Option<(i32, u32)>);

/// The XP, one entry per addend of the function.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XpTotals {
    /// One entry per `XpTerm`, in its declaration order.
    pub terms: [(XpTerm, i64); 9],
}

impl XpTotals {
    /// The XP: every term summed.
    #[must_use]
    pub fn total(&self) -> i64 {
        self.terms.iter().map(|(_, xp)| xp).sum()
    }

    /// What `term` contributes.
    #[must_use]
    pub fn of(&self, term: XpTerm) -> i64 {
        self.terms
            .iter()
            .find(|(t, _)| *t == term)
            .map_or(0, |(_, xp)| *xp)
    }
}

/// A language's first appearance: the earliest first commit-day of any project that qualifies
/// for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanguageFirst {
    /// `project.primary_language`, as stored.
    pub language: String,
    /// The local day (`ts div 86400`) of that project's first commit-day.
    pub day: i64,
    /// The project it is dated by; the lowest id on a tie.
    pub project_id: ProjectId,
}

/// The XP by term, under the price table in force.
///
/// # Errors
/// Returns [`IndexError`] when a read fails.
pub fn xp_by_term(conn: &Connection) -> Result<XpTotals, IndexError> {
    let p = prices();
    let (commit_days, revivals) = commit_day_terms(conn, &p)?;
    let releases = release_term(conn, &p)?;
    let languages = count(language_firsts(conn)?.len()) * p.language;
    let focus = focus_term(conn, &p)?;
    let debt_days = debt_day_term(conn, &p)?;
    let completion = rows_of(conn, "completion_check")? * p.completion_check;
    let verdicts = rows_of(conn, "amnesty_verdict")? * p.amnesty_verdict;
    let archives = rows_of(conn, "clean_archive")? * p.clean_archive;
    Ok(XpTotals {
        terms: XpTerm::ALL.map(|term| {
            let xp = match term {
                XpTerm::CommitDay => commit_days,
                XpTerm::Revival => revivals,
                XpTerm::Release => releases,
                XpTerm::Language => languages,
                XpTerm::Focus => focus,
                XpTerm::DebtDay => debt_days,
                XpTerm::CompletionCheck => completion,
                XpTerm::AmnestyVerdict => verdicts,
                XpTerm::CleanArchive => archives,
            };
            (term, xp)
        }),
    })
}

/// Every language that pays, in language order.
///
/// Each is dated at its earliest qualifying project's first commit-day. A project qualifies with
/// a non-NULL `primary_language`, not Reference, not merged away, a full history and at least
/// one commit-day.
///
/// # Errors
/// Returns [`IndexError`] when the read fails.
pub fn language_firsts(conn: &Connection) -> Result<Vec<LanguageFirst>, IndexError> {
    let mut stmt = conn.prepare(
        "SELECT p.primary_language, MIN(x.ts), p.id
           FROM project p
           JOIN xp_events x ON x.project_id = p.id AND x.kind = 'commit_day'
          WHERE p.primary_language IS NOT NULL
            AND p.is_reference = 0
            AND p.merged_into IS NULL
            AND p.is_shallow = 0
          GROUP BY p.id",
    )?;
    let mut firsts: BTreeMap<String, (i64, i64)> = BTreeMap::new();
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, i64>(2)?,
        ))
    })?;
    for row in rows {
        let (language, ts, project) = row?;
        let candidate = (ts.div_euclid(DAY), project);
        firsts
            .entry(language)
            .and_modify(|best| *best = (*best).min(candidate))
            .or_insert(candidate);
    }
    Ok(firsts
        .into_iter()
        .map(|(language, (day, project))| LanguageFirst {
            language,
            day,
            project_id: ProjectId(project),
        })
        .collect())
}

/// The commit-day and revival terms. Per local date at most `commit_days_per_date` projects
/// count, revivals first, then by `project_id`; a revival pays only when its project is counted.
fn commit_day_terms(conn: &Connection, p: &PriceTable) -> Result<(i64, i64), IndexError> {
    let mut stmt = conn.prepare(
        "SELECT x.project_id, x.ts
           FROM xp_events x
           JOIN project p ON p.id = x.project_id
          WHERE x.kind = 'commit_day' AND p.is_shallow = 0",
    )?;
    let mut days_of: BTreeMap<i64, BTreeSet<i64>> = BTreeMap::new();
    for row in stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))? {
        let (project, ts) = row?;
        days_of
            .entry(project)
            .or_default()
            .insert(ts.div_euclid(DAY));
    }

    // Per date, each project on it and whether that commit-day ends a dormancy. A project's
    // first commit-day has no previous one, so it is never a revival.
    let mut on_date: BTreeMap<i64, Vec<(bool, i64)>> = BTreeMap::new();
    for (project, days) in &days_of {
        let mut previous = None;
        for &day in days {
            let revival = previous.is_some_and(|prev| is_revival_gap(prev, day));
            on_date.entry(day).or_default().push((revival, *project));
            previous = Some(day);
        }
    }

    let cap = usize::try_from(p.commit_days_per_date).unwrap_or(0);
    let (mut counted, mut revived) = (0_usize, 0_usize);
    for projects in on_date.values_mut() {
        projects.sort_by_key(|&(revival, project)| (!revival, project));
        for &(revival, _) in projects.iter().take(cap) {
            counted += 1;
            revived += usize::from(revival);
        }
    }
    Ok((count(counted) * p.commit_day, count(revived) * p.revival))
}

/// The release term: per project and calendar month of the release's local date, distinct
/// dates, at most `releases_per_month` of them.
fn release_term(conn: &Connection, p: &PriceTable) -> Result<i64, IndexError> {
    let mut stmt = conn.prepare("SELECT project_id, ts FROM xp_events WHERE kind = 'release'")?;
    // A git row's `ts` is its local date at midnight UTC, so the UTC calendar of `ts` is the
    // local calendar of the date.
    let mut dates: BTreeMap<ProjectMonth, BTreeSet<i64>> = BTreeMap::new();
    for row in stmt.query_map([], |r| {
        Ok((r.get::<_, Option<i64>>(0)?, r.get::<_, i64>(1)?))
    })? {
        let (project, ts) = row?;
        let month = chrono::DateTime::from_timestamp(ts, 0).map(|d| (d.year(), d.month()));
        dates
            .entry((project, month))
            .or_default()
            .insert(ts.div_euclid(DAY));
    }
    let cap = usize::try_from(p.releases_per_month).unwrap_or(0);
    let counted: usize = dates.values().map(|d| d.len().min(cap)).sum();
    Ok(count(counted) * p.release)
}

/// The focus term: per local date of the row, its `meta.seconds` summed, one point per
/// `focus_seconds_per_xp`, at most `focus_per_date`.
fn focus_term(conn: &Connection, p: &PriceTable) -> Result<i64, IndexError> {
    let mut stmt =
        conn.prepare("SELECT ts, tz_offset_min, meta FROM xp_events WHERE kind = 'focus'")?;
    let mut seconds_on: BTreeMap<i64, i64> = BTreeMap::new();
    for row in stmt.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, Option<i64>>(1)?,
            r.get::<_, Option<String>>(2)?,
        ))
    })? {
        let (ts, offset_min, meta) = row?;
        let date = (ts + offset_min.unwrap_or(0) * 60).div_euclid(DAY);
        let seconds = parsed(meta.as_deref())
            .and_then(|m| m.get("seconds").and_then(serde_json::Value::as_i64))
            .unwrap_or(0);
        *seconds_on.entry(date).or_default() += seconds;
    }
    Ok(seconds_on
        .values()
        .map(|&s| (s.max(0) / p.focus_seconds_per_xp).min(p.focus_per_date))
        .sum())
}

/// The `debt_day` term. A row whose `meta.sources` is exactly `["abandoned_with_debt"]` is the
/// one class the writer before §38.8.1's gates paid for a shown-only closure, and prices 0.
fn debt_day_term(conn: &Connection, p: &PriceTable) -> Result<i64, IndexError> {
    let mut stmt = conn.prepare("SELECT meta FROM xp_events WHERE kind = 'debt_day'")?;
    let mut paying = 0_usize;
    for meta in stmt.query_map([], |r| r.get::<_, Option<String>>(0))? {
        let legacy = parsed(meta?.as_deref()).is_some_and(|m| {
            m.get("sources")
                .and_then(serde_json::Value::as_array)
                .is_some_and(|s| *s == [serde_json::Value::from("abandoned_with_debt")])
        });
        paying += usize::from(!legacy);
    }
    Ok(count(paying) * p.debt_day)
}

/// How many rows of `kind` the ledger holds.
fn rows_of(conn: &Connection, kind: &str) -> Result<i64, IndexError> {
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM xp_events WHERE kind = ?1",
        [kind],
        |r| r.get(0),
    )?)
}

/// A row's `meta`, parsed; `None` when it is absent or not JSON.
fn parsed(meta: Option<&str>) -> Option<serde_json::Value> {
    serde_json::from_str(meta?).ok()
}

/// A count of rows as XP's integer type. No ledger holds `i64::MAX` rows.
fn count(n: usize) -> i64 {
    i64::try_from(n).unwrap_or(i64::MAX)
}
