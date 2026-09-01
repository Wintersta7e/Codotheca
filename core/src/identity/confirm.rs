//! §1.4's confirmation. Confirming the identity set changes `authored_by_user`, and therefore
//! changes numbers the user has already been shown — so the effect is stated first.

use std::collections::BTreeSet;

use crate::index::IndexError;
use crate::protocol::IdentityConfirm;

/// What confirming this set would do, or has just done.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Delta {
    pub moved_to_reference: u32,
    pub commit_days_removed: i64,
    /// The projects whose authorship changes. Their history jobs are re-queued so the
    /// git-derived ledger is rebuilt rather than left missing.
    pub affected: Vec<i64>,
}

/// One project's authorship, before and after.
struct Verdict {
    id: i64,
    was_reference: bool,
    any_mine: bool,
}

impl Verdict {
    /// True when this confirmation changes the project's side of the line.
    ///
    /// A project becomes Reference exactly when none of its committers is the user, so the row
    /// changes when its current flag agrees with "the user authored here".
    fn changes(&self) -> bool {
        self.any_mine == self.was_reference
    }

    /// True when the change is *into* Reference, which is the only direction that loses days.
    fn moves_in(&self) -> bool {
        self.changes() && !self.any_mine
    }
}

/// The join `project_committer` exists to make cheap. Writes nothing.
///
/// # Errors
/// Returns [`IndexError`] when a read fails.
pub fn preview(conn: &rusqlite::Connection, emails: &[String]) -> Result<Delta, IndexError> {
    let mine: BTreeSet<String> = emails.iter().map(|e| e.to_lowercase()).collect();

    let mut stmt = conn.prepare(
        "SELECT p.id, p.is_reference, c.email
           FROM project p
           JOIN project_committer c ON c.project_id = p.id
          WHERE p.is_hidden = 0 AND p.merged_into IS NULL
          ORDER BY p.id",
    )?;
    let rows: Vec<(i64, i64, String)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get::<_, String>(2)?)))?
        .collect::<Result<Vec<_>, _>>()?;

    // A project with no committer row at all appears in no group: no history is not someone
    // else's history, and it must not be swept into Reference.
    let mut verdicts: Vec<Verdict> = Vec::new();
    for (id, is_reference, email) in rows {
        let is_mine = mine.contains(&email.to_lowercase());
        match verdicts.last_mut() {
            Some(current) if current.id == id => current.any_mine = current.any_mine || is_mine,
            _ => verdicts.push(Verdict {
                id,
                was_reference: is_reference != 0,
                any_mine: is_mine,
            }),
        }
    }

    let affected: Vec<i64> = verdicts
        .iter()
        .filter(|v| v.changes())
        .map(|v| v.id)
        .collect();
    let moved_to_reference =
        u32::try_from(verdicts.iter().filter(|v| v.moves_in()).count()).unwrap_or(u32::MAX);

    // Only a project moving *into* Reference loses days; one moving out gains them back on the
    // recompute, and reporting that as a removal would be false.
    let mut commit_days_removed: i64 = 0;
    for verdict in verdicts.iter().filter(|v| v.moves_in()) {
        let removed: i64 = conn.query_row(
            "SELECT COUNT(*) FROM xp_events WHERE project_id = ?1 AND kind = 'commit_day'",
            rusqlite::params![verdict.id],
            |r| r.get(0),
        )?;
        commit_days_removed = commit_days_removed.saturating_add(removed);
    }

    Ok(Delta {
        moved_to_reference,
        commit_days_removed,
        affected,
    })
}

/// Apply the set: one transaction, and the floor stamped before anything is deleted.
///
/// # Errors
/// Returns [`IndexError`] when a read or write fails.
pub fn apply(
    conn: &mut rusqlite::Connection,
    emails: &[String],
    now: i64,
) -> Result<IdentityConfirm, IndexError> {
    let delta = preview(conn, emails)?;
    let _guard = crate::proto::txguard::TxGuard::enter();
    let tx = conn.transaction()?;

    // Before the delete, or it is unrecoverable.
    stamp_level_floor(&tx)?;

    let mine: Vec<String> = emails.to_vec();
    let placeholders = if mine.is_empty() {
        "''".to_owned()
    } else {
        mine.iter().map(|_| "?").collect::<Vec<_>>().join(",")
    };
    let params: Vec<&dyn rusqlite::ToSql> =
        mine.iter().map(|e| e as &dyn rusqlite::ToSql).collect();
    let now_param = &now as &dyn rusqlite::ToSql;

    // authored_by_user and is_reference recompute from the join, not from a history walk.
    tx.execute(
        &format!(
            "UPDATE project
                SET authored_by_user = CASE
                      WHEN NOT EXISTS (SELECT 1 FROM project_committer c WHERE c.project_id = project.id)
                        THEN NULL
                      WHEN EXISTS (SELECT 1 FROM project_committer c
                                    WHERE c.project_id = project.id
                                      AND c.email COLLATE NOCASE IN ({placeholders}))
                        THEN 1 ELSE 0 END,
                    is_reference = CASE
                      WHEN NOT EXISTS (SELECT 1 FROM project_committer c WHERE c.project_id = project.id)
                        THEN is_reference
                      WHEN EXISTS (SELECT 1 FROM project_committer c
                                    WHERE c.project_id = project.id
                                      AND c.email COLLATE NOCASE IN ({placeholders}))
                        THEN 0 ELSE 1 END,
                    updated_at = ?{n}
              WHERE is_hidden = 0 AND merged_into IS NULL",
            n = mine.len() * 2 + 1
        ),
        rusqlite::params_from_iter(
            params
                .iter()
                .copied()
                .chain(params.iter().copied())
                .chain(std::iter::once(now_param)),
        ),
    )?;

    // §1.7: git-derived events are a pure function of history and are deleted and recomputed,
    // never migrated. Session-derived rows are untouched — two ledgers, never merged.
    for id in &delta.affected {
        for kind in crate::identity::merge::GIT_DERIVED_XP_KINDS {
            tx.execute(
                "DELETE FROM xp_events WHERE project_id = ?1 AND kind = ?2 AND track = 'git'",
                rusqlite::params![id, kind],
            )?;
        }
        crate::jobs::state::reset_for(
            &tx,
            crate::protocol::ProjectId(*id),
            crate::jobs::state::ResetCause::UserRequested,
            now,
        )?;
    }

    tx.execute(
        &format!(
            "UPDATE identity
                SET is_user = CASE WHEN email COLLATE NOCASE IN ({placeholders}) THEN 1 ELSE 0 END,
                    confirmed_at = ?{n}",
            n = mine.len() + 1
        ),
        rusqlite::params_from_iter(params.iter().copied().chain(std::iter::once(now_param))),
    )?;

    tx.commit()?;
    Ok(IdentityConfirm {
        moved_to_reference: delta.moved_to_reference,
        commit_days_removed: delta.commit_days_removed,
        applied: true,
    })
}

/// Record the pre-change ledger's weight, monotonically.
///
/// §1.4 asks for "the level implied by the pre-change ledger", and phase 1 defines no level:
/// §1.7 defers points, level, rings and badges entirely to phase 4. What phase 4 cannot
/// recover, and what this therefore preserves, is the **size of the git-derived ledger before
/// the delete**. It is stored as `max(existing, computed)` so nothing earned is ever taken
/// away. Phase 4 owns the curve that turns it into a level; it does not own a time machine.
///
/// # Errors
/// Returns [`IndexError`] when a read or write fails.
pub fn stamp_level_floor(tx: &rusqlite::Transaction<'_>) -> Result<i64, IndexError> {
    let before: i64 = tx.query_row(
        "SELECT COUNT(*) FROM xp_events WHERE track = 'git'",
        [],
        |r| r.get(0),
    )?;
    let existing: i64 = tx
        .query_row("SELECT v FROM app_meta WHERE k = 'level_floor'", [], |r| {
            r.get::<_, String>(0)
        })
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let floor = existing.max(before);
    tx.execute(
        "INSERT INTO app_meta (k, v) VALUES ('level_floor', ?1)
           ON CONFLICT(k) DO UPDATE SET v = excluded.v",
        rusqlite::params![floor.to_string()],
    )?;
    Ok(floor)
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

    const NOW: i64 = 1_760_000_000;

    fn open() -> (tempfile::TempDir, crate::index::Index) {
        let dir = tempfile::tempdir().unwrap();
        let index = crate::index::Index::open_at(dir.path(), NOW).unwrap();
        (dir, index)
    }

    fn project(conn: &rusqlite::Connection, name: &str) -> i64 {
        conn.execute(
            "INSERT INTO project (name, seed_basename, is_reference, created_at, updated_at)
             VALUES (?1, ?1, 0, ?2, ?2)",
            rusqlite::params![name, NOW],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    fn committer(conn: &rusqlite::Connection, project: i64, email: &str) {
        conn.execute(
            "INSERT INTO project_committer (project_id, email, commits) VALUES (?1, ?2, 5)",
            rusqlite::params![project, email],
        )
        .unwrap();
    }

    fn commit_day(conn: &rusqlite::Connection, project: i64, key: &str) {
        conn.execute(
            "INSERT INTO xp_events (ts, project_id, subject_key, kind, dedupe_key, track)
             VALUES (?1, ?2, 's', 'commit_day', ?3, 'git')",
            rusqlite::params![NOW, project, key],
        )
        .unwrap();
    }

    fn seeded(conn: &rusqlite::Connection, email: &str) {
        conn.execute(
            "INSERT INTO identity (email, is_user, source) VALUES (?1, 1, 'gitconfig')",
            rusqlite::params![email],
        )
        .unwrap();
    }

    // §1.4 / §2.4: apply:false returns {movedToReference, commitDaysRemoved} and writes nothing.
    #[test]
    fn the_preview_writes_nothing() {
        let (_dir, index) = open();
        let conn = index.conn();
        let mine = project(conn, "mine");
        let theirs = project(conn, "theirs");
        committer(conn, mine, "a@example.invalid");
        committer(conn, theirs, "other@example.invalid");
        commit_day(conn, theirs, "k1");
        commit_day(conn, theirs, "k2");
        seeded(conn, "a@example.invalid");
        seeded(conn, "other@example.invalid");

        let delta = preview(conn, &["a@example.invalid".to_owned()]).unwrap();
        assert_eq!(delta.moved_to_reference, 1);
        assert_eq!(delta.commit_days_removed, 2);
        assert_eq!(delta.affected, vec![theirs]);

        let still: i64 = conn
            .query_row("SELECT COUNT(*) FROM xp_events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(still, 2, "the preview must not delete anything");
        let reference: i64 = conn
            .query_row(
                "SELECT is_reference FROM project WHERE id = ?1",
                [theirs],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(reference, 0, "the preview must not move anything");
    }

    #[test]
    fn applying_moves_the_projects_and_removes_their_commit_days() {
        let (_dir, mut index) = open();
        let (mine, theirs) = {
            let conn = index.conn();
            let mine = project(conn, "mine");
            let theirs = project(conn, "theirs");
            committer(conn, mine, "a@example.invalid");
            committer(conn, theirs, "other@example.invalid");
            commit_day(conn, theirs, "k1");
            commit_day(conn, mine, "k2");
            seeded(conn, "a@example.invalid");
            seeded(conn, "other@example.invalid");
            (mine, theirs)
        };

        let out = apply(index.conn_mut(), &["a@example.invalid".to_owned()], NOW).unwrap();
        assert!(out.applied);
        assert_eq!(out.moved_to_reference, 1);
        assert_eq!(out.commit_days_removed, 1);

        let conn = index.conn();
        let flag: i64 = conn
            .query_row(
                "SELECT is_reference FROM project WHERE id = ?1",
                [theirs],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(flag, 1);
        let authored: Option<i64> = conn
            .query_row(
                "SELECT authored_by_user FROM project WHERE id = ?1",
                [mine],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(authored, Some(1));
        let kept: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM xp_events WHERE project_id = ?1",
                [mine],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(kept, 1, "an untouched project keeps its ledger");
    }

    // §1.4 / §1.7: level_floor is stamped before the delete, or it is unrecoverable.
    #[test]
    fn the_level_floor_is_stamped_before_the_delete_and_never_falls() {
        let (_dir, mut index) = open();
        {
            let conn = index.conn();
            let mine = project(conn, "mine");
            let theirs = project(conn, "theirs");
            committer(conn, mine, "a@example.invalid");
            committer(conn, theirs, "other@example.invalid");
            commit_day(conn, theirs, "k1");
            commit_day(conn, theirs, "k2");
            commit_day(conn, mine, "k3");
            seeded(conn, "a@example.invalid");
            seeded(conn, "other@example.invalid");
        }

        apply(index.conn_mut(), &["a@example.invalid".to_owned()], NOW).unwrap();
        let floor: String = index
            .conn()
            .query_row("SELECT v FROM app_meta WHERE k = 'level_floor'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(
            floor, "3",
            "the floor is the pre-change ledger, not the post-change one"
        );

        // Monotone: a second, wider confirmation cannot lower it.
        apply(
            index.conn_mut(),
            &[
                "a@example.invalid".to_owned(),
                "other@example.invalid".to_owned(),
            ],
            NOW,
        )
        .unwrap();
        let floor: String = index
            .conn()
            .query_row("SELECT v FROM app_meta WHERE k = 'level_floor'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(floor, "3");
    }

    #[test]
    fn confirming_stamps_every_surviving_row_and_unticks_the_rest() {
        let (_dir, mut index) = open();
        {
            let conn = index.conn();
            seeded(conn, "a@example.invalid");
            seeded(conn, "other@example.invalid");
        }
        apply(index.conn_mut(), &["a@example.invalid".to_owned()], NOW).unwrap();
        let conn = index.conn();
        let kept: (i64, Option<i64>) = conn
            .query_row(
                "SELECT is_user, confirmed_at FROM identity WHERE email = 'a@example.invalid'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(kept, (1, Some(NOW)));
        let dropped: (i64, Option<i64>) = conn
            .query_row(
                "SELECT is_user, confirmed_at FROM identity WHERE email = 'other@example.invalid'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(dropped, (0, Some(NOW)));
    }

    #[test]
    fn a_project_with_no_committers_at_all_is_not_moved_to_reference() {
        let (_dir, index) = open();
        let conn = index.conn();
        let empty = project(conn, "empty");
        seeded(conn, "a@example.invalid");
        let delta = preview(conn, &["a@example.invalid".to_owned()]).unwrap();
        assert_eq!(delta.moved_to_reference, 0);
        assert!(
            !delta.affected.contains(&empty),
            "no history is not someone else's history"
        );
    }
}
