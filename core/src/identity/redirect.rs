//! Stale project ids keep resolving (§1.6).

use rusqlite::{params, OptionalExtension as _, Transaction};

use super::IdentityError;

/// Resolve a possibly-stale project id (§1.6). **One hop.**
///
/// Chaining with path compression is machinery for a sub-second race on a single-user local
/// database; [`write_redirect`] keeps one hop sufficient instead.
pub fn resolve_project_id(tx: &Transaction<'_>, requested: i64) -> Result<i64, IdentityError> {
    let merged_into: Option<Option<i64>> = tx
        .query_row(
            "SELECT merged_into FROM project WHERE id = ?1",
            params![requested],
            |r| r.get(0),
        )
        .optional()?;

    let Some(state) = merged_into else {
        return Err(IdentityError::UnknownProject(requested));
    };
    let Some(into) = state else {
        return Ok(requested); // A live row resolves to itself.
    };

    // Tombstoned: follow the redirect, once.
    let hop: Option<i64> = tx
        .query_row(
            "SELECT new_project_id FROM project_redirect WHERE old_project_id = ?1",
            params![requested],
            |r| r.get(0),
        )
        .optional()?;
    let Some(target) = hop else {
        // Tombstoned with no redirect: the caller holds a stale id and refreshes.
        return Err(IdentityError::ProjectMerged { requested, into });
    };

    let target_state: Option<Option<i64>> = tx
        .query_row(
            "SELECT merged_into FROM project WHERE id = ?1",
            params![target],
            |r| r.get(0),
        )
        .optional()?;
    match target_state {
        Some(None) => Ok(target),
        // The hop landed on another tombstone. `write_redirect` exists to make this
        // unreachable, so it is a defect in a writer, not a case to chain through.
        Some(Some(_)) => Err(IdentityError::RedirectChain {
            requested,
            via: target,
        }),
        None => Err(IdentityError::UnknownProject(target)),
    }
}

/// Write the redirect a merge leaves behind, and re-point every redirect that named the absorbed
/// row — which is the mechanism that keeps §1.6's single hop sufficient after a second merge.
///
/// Without the second statement, `A → B` followed by `B → C` leaves `A → B` pointing at a
/// tombstone, and a caller holding `A` gets an error where a valid project exists.
pub fn write_redirect(
    tx: &Transaction<'_>,
    old_project_id: i64,
    new_project_id: i64,
    merged_at: i64,
) -> Result<(), IdentityError> {
    tx.execute(
        "INSERT INTO project_redirect (old_project_id, new_project_id, merged_at)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(old_project_id) DO UPDATE
            SET new_project_id = excluded.new_project_id, merged_at = excluded.merged_at",
        params![old_project_id, new_project_id, merged_at],
    )?;
    tx.execute(
        "UPDATE project_redirect SET new_project_id = ?2, merged_at = ?3
          WHERE new_project_id = ?1 AND old_project_id <> ?2",
        params![old_project_id, new_project_id, merged_at],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::super::testutil::{insert_project, open_test_index, NewProject};
    use super::super::IdentityError;
    use super::{resolve_project_id, write_redirect};

    fn p(conn: &rusqlite::Connection, name: &'static str) -> i64 {
        insert_project(
            conn,
            NewProject {
                name,
                lineage_key: None,
                remote_key: None,
                created_at: 1,
            },
        )
    }

    fn tombstone(tx: &rusqlite::Transaction<'_>, absorbed: i64, into: i64) {
        tx.execute(
            "UPDATE project SET merged_into = ?2 WHERE id = ?1",
            rusqlite::params![absorbed, into],
        )
        .unwrap();
    }

    #[test]
    fn a_live_project_resolves_to_itself() {
        let mut conn = open_test_index();
        let a = p(&conn, "a");
        let tx = conn.transaction().unwrap();
        assert_eq!(resolve_project_id(&tx, a).unwrap(), a);
        tx.commit().unwrap();
    }

    #[test]
    fn a_stale_id_resolves_through_the_redirect_so_an_in_flight_request_stays_valid() {
        let mut conn = open_test_index();
        let a = p(&conn, "a");
        let b = p(&conn, "b");
        let tx = conn.transaction().unwrap();
        tombstone(&tx, a, b);
        write_redirect(&tx, a, b, 50).unwrap();
        assert_eq!(resolve_project_id(&tx, a).unwrap(), b);
        tx.commit().unwrap();
    }

    #[test]
    fn a_second_merge_repoints_the_first_redirect_rather_than_forming_a_chain() {
        let mut conn = open_test_index();
        let a = p(&conn, "a");
        let b = p(&conn, "b");
        let c = p(&conn, "c");
        let tx = conn.transaction().unwrap();
        tombstone(&tx, a, b);
        write_redirect(&tx, a, b, 50).unwrap();
        tombstone(&tx, b, c);
        write_redirect(&tx, b, c, 60).unwrap();

        assert_eq!(
            resolve_project_id(&tx, a).unwrap(),
            c,
            "one hop still suffices"
        );
        assert_eq!(resolve_project_id(&tx, b).unwrap(), c);
        let target: i64 = tx
            .query_row(
                "SELECT new_project_id FROM project_redirect WHERE old_project_id = ?1",
                rusqlite::params![a],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(target, c);
        tx.commit().unwrap();
    }

    #[test]
    fn a_chain_that_survived_anyway_errors_rather_than_being_followed() {
        // Two hops can only exist if a writer skipped write_redirect. Failing loudly beats
        // silently resolving through machinery §1.6 rules out.
        let mut conn = open_test_index();
        let a = p(&conn, "a");
        let b = p(&conn, "b");
        let c = p(&conn, "c");
        let tx = conn.transaction().unwrap();
        tombstone(&tx, a, b);
        tombstone(&tx, b, c);
        tx.execute(
            "INSERT INTO project_redirect (old_project_id, new_project_id, merged_at)
             VALUES (?1, ?2, 50)",
            rusqlite::params![a, b],
        )
        .unwrap();
        assert!(matches!(
            resolve_project_id(&tx, a),
            Err(IdentityError::RedirectChain { requested, via })
                if requested == a && via == b
        ));
        tx.commit().unwrap();
    }

    #[test]
    fn a_tombstone_with_no_redirect_tells_the_caller_to_refresh() {
        let mut conn = open_test_index();
        let a = p(&conn, "a");
        let b = p(&conn, "b");
        let tx = conn.transaction().unwrap();
        tombstone(&tx, a, b);
        let err = resolve_project_id(&tx, a).unwrap_err();
        assert!(matches!(
            err,
            IdentityError::ProjectMerged { requested, into }
                if requested == a && into == b
        ));
        assert_eq!(err.code(), crate::protocol::ErrorCode::ProjectMerged);
        tx.commit().unwrap();
    }

    #[test]
    fn an_id_that_never_existed_is_not_a_merge() {
        let mut conn = open_test_index();
        let tx = conn.transaction().unwrap();
        assert!(matches!(
            resolve_project_id(&tx, 4242),
            Err(IdentityError::UnknownProject(4242))
        ));
        tx.commit().unwrap();
    }
}
