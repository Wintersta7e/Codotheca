//! §10.4's resolution: every figure carries the population it was computed over.
//!
//! *Show me what you've got* and *no fabricated number anywhere* were both load-bearing and
//! mutually exclusive — a span computed over 60% of the shelf is not partial, it is wrong. The
//! answer is not to hold numbers back; it is to make each one carry its basis.

use crate::index::IndexError;
use crate::protocol::{ProjectId, Reveal, RevealBasis, RevealFigure, RevealOldest};

/// Projects that count, and projects whose slow jobs have returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Population {
    /// Every indexed project the shelf shows. Shallow and reference rows included — they are
    /// what the coverage line is about.
    pub total: u32,
    /// Projects whose history job has completed.
    pub history_covered: u32,
    /// Projects whose inventory job has completed.
    pub inventory_covered: u32,
    /// True when every project that could yield history has yielded it.
    pub history_complete: bool,
}

const LIVE: &str = "is_hidden = 0 AND merged_into IS NULL";

/// Count the population and the two coverages.
///
/// # Errors
/// Returns [`IndexError`] when a read fails.
pub fn population(conn: &rusqlite::Connection) -> Result<Population, IndexError> {
    let total: i64 = conn.query_row(
        &format!("SELECT COUNT(*) FROM project WHERE {LIVE}"),
        [],
        |r| r.get(0),
    )?;
    let covered = |job: &str| -> Result<i64, IndexError> {
        Ok(conn.query_row(
            &format!(
                "SELECT COUNT(*) FROM project p
                   JOIN project_job_state s ON s.project_id = p.id
                  WHERE {LIVE} AND s.job = ?1 AND s.state = 'ok'"
            ),
            rusqlite::params![job],
            |r| r.get(0),
        )?)
    };
    let history_covered = covered("j4")?;
    let inventory_covered = covered("j3")?;
    Ok(Population {
        total: u32::try_from(total).unwrap_or(u32::MAX),
        history_covered: u32::try_from(history_covered).unwrap_or(u32::MAX),
        inventory_covered: u32::try_from(inventory_covered).unwrap_or(u32::MAX),
        history_complete: history_covered >= total,
    })
}

const fn basis(covered: u32, pop: &Population) -> RevealBasis {
    RevealBasis {
        projects_covered: covered,
        projects_total: pop.total,
        history_complete: pop.history_complete,
    }
}

/// Seconds, as a wire `f64`.
///
/// `f64` loses integer precision above 2^53; that is 285 million years of playtime and 285
/// million days of span, so the cast is lossless for every value either figure can hold.
// `i64` to `f64` has no conversion function, so this helper holds the module's one `as`.
#[allow(clippy::as_conversions, clippy::cast_precision_loss)]
const fn seconds_as_f64(seconds: i64) -> f64 {
    seconds as f64
}

/// The whole reveal.
///
/// # Errors
/// Returns [`IndexError`] when a read fails.
pub fn reveal(conn: &rusqlite::Connection, now: i64) -> Result<Reveal, IndexError> {
    let pop = population(conn)?;

    // §10.4 / criterion 23: shallow repositories are excluded from every history figure. A
    // shallow clone's first commit is the truncation point, not the beginning of anything.
    let earliest: Option<i64> = conn.query_row(
        &format!(
            "SELECT MIN(first_commit_at) FROM project
              WHERE {LIVE} AND is_shallow = 0 AND is_reference = 0 AND first_commit_at IS NOT NULL"
        ),
        [],
        |r| r.get(0),
    )?;

    let language_count: i64 = conn.query_row(
        &format!(
            "SELECT COUNT(DISTINCT primary_language) FROM project
              WHERE {LIVE} AND primary_language IS NOT NULL"
        ),
        [],
        |r| r.get(0),
    )?;

    let mut starts_stmt = conn.prepare(&format!(
        "SELECT first_commit_at, first_commit_tz_offset_min FROM project
          WHERE {LIVE} AND is_shallow = 0 AND is_reference = 0 AND first_commit_at IS NOT NULL"
    ))?;
    let starts: Vec<i32> = starts_stmt
        .query_map([], |r| {
            let at: i64 = r.get(0)?;
            let tz: Option<i64> = r.get(1)?;
            Ok(local_year(at, i32::try_from(tz.unwrap_or(0)).unwrap_or(0)))
        })?
        .collect::<Result<Vec<i32>, _>>()?;

    let playtime: i64 = conn.query_row(
        "SELECT COALESCE(SUM(credited_seconds), 0) FROM session",
        [],
        |r| r.get(0),
    )?;

    let oldest: Option<(i64, i64)> = conn
        .query_row(
            &format!(
                "SELECT id, first_commit_at FROM project
                  WHERE {LIVE} AND is_shallow = 0 AND is_reference = 0 AND is_archived = 0
                    AND first_commit_at IS NOT NULL
                  ORDER BY first_commit_at ASC, id ASC LIMIT 1"
            ),
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .ok();

    Ok(Reveal {
        span_days: RevealFigure {
            value: span_days(earliest, now),
            basis: basis(pop.history_covered, &pop),
        },
        project_count: RevealFigure {
            // Complete by construction: this counts what is indexed, and what is indexed is
            // what the scan headline counted.
            value: Some(f64::from(pop.total)),
            basis: RevealBasis {
                projects_covered: pop.total,
                projects_total: pop.total,
                history_complete: pop.history_complete,
            },
        },
        language_count: RevealFigure {
            value: Some(u32::try_from(language_count).unwrap_or(u32::MAX).into()),
            basis: basis(pop.inventory_covered, &pop),
        },
        best_year: RevealFigure {
            value: best_year(&starts),
            basis: basis(pop.history_covered, &pop),
        },
        playtime_seconds: RevealFigure {
            value: Some(seconds_as_f64(playtime)),
            // §10.4a: PLAYTIME takes neither honesty string. It counts launched sessions and
            // nothing else, so it is complete the moment it is read.
            basis: RevealBasis {
                projects_covered: pop.total,
                projects_total: pop.total,
                history_complete: true,
            },
        },
        oldest_still_alive: RevealOldest {
            project_id: oldest.map(|(id, _)| ProjectId(id)),
            first_commit_at: oldest.map(|(_, at)| at),
            basis: basis(pop.history_covered, &pop),
        },
    })
}

/// Days from the earliest first commit to now, fractional.
///
/// Fractional rather than whole because the renderer recovers the earliest commit's calendar
/// year from it, and a rounded figure can cross a new year's boundary and print the wrong one.
#[must_use]
pub fn span_days(earliest_first_commit_at: Option<i64>, now: i64) -> Option<f64> {
    let earliest = earliest_first_commit_at?;
    let seconds = now.saturating_sub(earliest).max(0);
    // The figure is fractional by definition (see above), and no integer form yields the same
    // correctly rounded quotient, so this one division stays in floating point.
    #[allow(clippy::float_arithmetic)]
    let days = seconds_as_f64(seconds) / 86_400.0;
    Some(days)
}

/// The calendar year in which the most projects had their first commit.
///
/// A tie goes to the later year: two years of equal weight are not equally interesting, and the
/// recent one is the one the user can act on.
#[must_use]
pub fn best_year(starts: &[i32]) -> Option<f64> {
    if starts.is_empty() {
        return None;
    }
    let mut counts: std::collections::BTreeMap<i32, u32> = std::collections::BTreeMap::new();
    for year in starts {
        *counts.entry(*year).or_insert(0) += 1;
    }
    counts
        .into_iter()
        .max_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)))
        .map(|(year, _)| f64::from(year))
}

/// The calendar year of an instant in the commit's own local time.
fn local_year(at: i64, tz_offset_min: i32) -> i32 {
    let local = at + i64::from(tz_offset_min) * 60;
    let days = local.div_euclid(86_400);
    // Civil-from-days, the standard branchless conversion.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let year = if mp >= 10 { y + 1 } else { y };
    i32::try_from(year).unwrap_or(0)
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

    const DAY: i64 = 86_400;
    const NOW: i64 = 1_760_000_000;

    /// A migrated, empty database. The `TempDir` is returned so it outlives the `Index`.
    fn open() -> (tempfile::TempDir, crate::index::Index) {
        let dir = tempfile::tempdir().unwrap();
        let index = crate::index::Index::open_at(dir.path(), NOW).unwrap();
        (dir, index)
    }

    #[allow(clippy::too_many_arguments)]
    fn project(
        conn: &rusqlite::Connection,
        name: &str,
        first_commit_at: Option<i64>,
        language: Option<&str>,
        shallow: bool,
        reference: bool,
        archived: bool,
    ) -> i64 {
        conn.execute(
            "INSERT INTO project
               (name, seed_basename, first_commit_at, primary_language, is_shallow,
                is_reference, is_archived, is_hidden, created_at, updated_at)
             VALUES (?1, ?1, ?2, ?3, ?4, ?5, ?6, 0, ?7, ?7)",
            rusqlite::params![
                name,
                first_commit_at,
                language,
                i64::from(shallow),
                i64::from(reference),
                i64::from(archived),
                NOW
            ],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    fn history_done(conn: &rusqlite::Connection, project: i64) {
        conn.execute(
            "INSERT INTO project_job_state (project_id, job, state, fail_count, at)
             VALUES (?1, 'j4', 'ok', 0, ?2)",
            rusqlite::params![project, NOW],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO project_job_state (project_id, job, state, fail_count, at)
             VALUES (?1, 'j3', 'ok', 0, ?2)",
            rusqlite::params![project, NOW],
        )
        .unwrap();
    }

    #[test]
    fn span_is_fractional_days_so_the_first_commit_date_is_recoverable() {
        assert_eq!(span_days(None, NOW), None);
        let earliest = NOW - (DAY * 3 + 3_600);
        let days = span_days(Some(earliest), NOW).unwrap();
        assert!((days - 3.041_666_6).abs() < 0.000_1, "got {days}");
        // A future-dated commit cannot produce a negative span.
        assert_eq!(span_days(Some(NOW + DAY), NOW), Some(0.0));
    }

    #[test]
    fn the_best_year_is_the_densest_year_of_starts_and_ties_go_to_the_later_year() {
        assert_eq!(best_year(&[]), None);
        assert_eq!(best_year(&[2019, 2021, 2021, 2023]), Some(2021.0));
        assert_eq!(best_year(&[2019, 2019, 2023, 2023]), Some(2023.0));
    }

    // §10.4a: the eyebrow's <n> PROJECTS and the PROJECTS panel are one number from one call.
    #[test]
    fn project_count_matches_what_the_scan_counted_reference_rows_included() {
        let (_dir, index) = open();
        let conn = index.conn();
        project(
            conn,
            "a",
            Some(NOW - DAY * 900),
            Some("Rust"),
            false,
            false,
            false,
        );
        project(
            conn,
            "b",
            Some(NOW - DAY * 100),
            Some("TypeScript"),
            false,
            true,
            false,
        );
        let out = reveal(conn, NOW).unwrap();
        assert_eq!(out.project_count.value, Some(2.0));
        assert_eq!(out.project_count.basis.projects_total, 2);
        assert_eq!(out.project_count.basis.projects_covered, 2);
        // The figure is complete by construction while the *population's* history is not:
        // coverage is per figure, and `historyComplete` describes the population.
        assert!(!out.project_count.basis.history_complete);
    }

    // §10.4 / criterion 23: shallow repositories are excluded from history statistics and
    // counted in the coverage line.
    #[test]
    fn a_shallow_repository_cannot_move_the_span_but_is_counted_in_the_basis() {
        let (_dir, index) = open();
        let conn = index.conn();
        let deep = project(
            conn,
            "deep",
            Some(NOW - DAY * 400),
            Some("Rust"),
            false,
            false,
            false,
        );
        history_done(conn, deep);
        let shallow = project(
            conn,
            "shal",
            Some(NOW - DAY * 9_000),
            None,
            true,
            false,
            false,
        );
        history_done(conn, shallow);

        let out = reveal(conn, NOW).unwrap();
        let span = out.span_days.value.unwrap();
        assert!(
            (span - 400.0).abs() < 1.0,
            "shallow history leaked into span: {span}"
        );
        assert_eq!(out.span_days.basis.projects_total, 2);
    }

    #[test]
    fn a_figure_with_nothing_behind_it_is_null_and_never_zero() {
        let (_dir, index) = open();
        let conn = index.conn();
        project(conn, "a", None, None, false, false, false);
        let out = reveal(conn, NOW).unwrap();
        assert_eq!(
            out.span_days.value, None,
            "an uncomputed span must not render as 0"
        );
        assert_eq!(out.best_year.value, None);
        assert_eq!(
            out.language_count.value,
            Some(0.0),
            "a real zero: no language is known yet"
        );
        assert_eq!(out.oldest_still_alive.project_id, None);
    }

    #[test]
    fn coverage_is_the_share_of_projects_whose_history_has_returned() {
        let (_dir, index) = open();
        let conn = index.conn();
        let a = project(
            conn,
            "a",
            Some(NOW - DAY * 300),
            Some("Rust"),
            false,
            false,
            false,
        );
        history_done(conn, a);
        project(conn, "b", None, None, false, false, false);
        let out = reveal(conn, NOW).unwrap();
        assert_eq!(out.span_days.basis.projects_covered, 1);
        assert_eq!(out.span_days.basis.projects_total, 2);
        assert!(!out.span_days.basis.history_complete);
    }

    // §10.4a: PLAYTIME takes neither string — 0h is complete by construction.
    #[test]
    fn playtime_starts_at_zero_and_reports_itself_complete() {
        let (_dir, index) = open();
        let conn = index.conn();
        project(
            conn,
            "a",
            Some(NOW - DAY * 10),
            Some("Rust"),
            false,
            false,
            false,
        );
        let out = reveal(conn, NOW).unwrap();
        assert_eq!(out.playtime_seconds.value, Some(0.0));
        assert!(out.playtime_seconds.basis.history_complete);
        assert_eq!(
            out.playtime_seconds.basis.projects_covered,
            out.playtime_seconds.basis.projects_total
        );
    }

    #[test]
    fn the_oldest_still_alive_is_neither_archived_nor_reference_nor_shallow() {
        let (_dir, index) = open();
        let conn = index.conn();
        let ancient = project(
            conn,
            "arch",
            Some(NOW - DAY * 4_000),
            Some("C"),
            false,
            false,
            true,
        );
        history_done(conn, ancient);
        let refd = project(
            conn,
            "ref",
            Some(NOW - DAY * 3_500),
            Some("C"),
            false,
            true,
            false,
        );
        history_done(conn, refd);
        let alive = project(
            conn,
            "live",
            Some(NOW - DAY * 2_000),
            Some("Rust"),
            false,
            false,
            false,
        );
        history_done(conn, alive);

        let out = reveal(conn, NOW).unwrap();
        assert_eq!(out.oldest_still_alive.project_id, Some(ProjectId(alive)));
        assert_eq!(
            out.oldest_still_alive.first_commit_at,
            Some(NOW - DAY * 2_000)
        );
    }
}
