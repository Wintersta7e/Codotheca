//! Submodules are their own projects (§4.4).

use rusqlite::{params, Transaction};

use super::IdentityError;

/// Read the commit the superproject pins the submodule at, from the **parent** repository.
/// Byte argv, because a path is bytes (§1.3); the git backend turns it into an `OsStr`.
///
/// `-q --verify` so a path that is not a gitlink exits non-zero with no output rather than
/// printing a diagnostic that would then be parsed as an OID.
#[must_use]
pub fn gitlink_oid_argv(submodule_path: &[u8]) -> Vec<Vec<u8>> {
    let trimmed: &[u8] = match submodule_path.split_last() {
        Some((b'/', head)) => head,
        _ => submodule_path,
    };
    let mut revspec = b"HEAD:".to_vec();
    revspec.extend_from_slice(trimmed);
    vec![
        b"rev-parse".to_vec(),
        b"-q".to_vec(),
        b"--verify".to_vec(),
        revspec,
    ]
}

/// One superproject-to-submodule relationship. §1.2 deprecates the singular
/// `parent_project_id` column in favour of `submodule_edge`, because the same library can be a
/// submodule of two parents, or appear twice under one — and singular columns lose all but one.
/// Both are written: the edge is the record, the column is what the project page reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubmoduleLink {
    pub parent_project_id: i64,
    /// Which copy of the parent this gitlink was read at.
    pub parent_location_id: i64,
    pub child_project_id: i64,
    /// Path within the parent's working tree.
    pub path_bytes: Vec<u8>,
    /// `None` when the parent has no resolvable `HEAD`, or the path is not a gitlink.
    pub gitlink_oid: Option<String>,
}

/// Record the relationship. **The parent gains no location**: a submodule is a checkout of a
/// different repository with its own root commit, so attaching it to the superproject would put
/// a location whose lineage differs from its project's under one row (§4.4).
///
/// The edge upserts on its own primary key rather than being deleted and re-inserted, so a
/// rescan updates the pinned commit in place and no row is ever removed.
pub fn link_submodule(
    tx: &Transaction<'_>,
    link: &SubmoduleLink,
    now: i64,
) -> Result<(), IdentityError> {
    tx.execute(
        "INSERT INTO submodule_edge (parent_project_id, child_project_id, parent_location_id,
                                     path_bytes, gitlink_oid)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT (parent_project_id, child_project_id, path_bytes) DO UPDATE SET
            parent_location_id = excluded.parent_location_id,
            gitlink_oid        = excluded.gitlink_oid",
        params![
            link.parent_project_id,
            link.child_project_id,
            link.parent_location_id,
            link.path_bytes,
            link.gitlink_oid,
        ],
    )?;

    // `submodule_path` is display-only and lossy by construction; `path_bytes` on the edge is
    // the operational value.
    let display = String::from_utf8_lossy(&link.path_bytes).into_owned();
    tx.execute(
        "UPDATE project SET parent_project_id = ?2, submodule_path = ?3, updated_at = ?4
          WHERE id = ?1",
        params![link.child_project_id, link.parent_project_id, display, now],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::super::testutil::{insert_location, insert_project, open_test_index, NewProject};
    use super::{gitlink_oid_argv, link_submodule, SubmoduleLink};

    #[test]
    fn the_gitlink_is_read_at_the_parent_as_a_byte_revspec() {
        assert_eq!(
            gitlink_oid_argv(b"vendor/lib"),
            vec![
                b"rev-parse".to_vec(),
                b"-q".to_vec(),
                b"--verify".to_vec(),
                b"HEAD:vendor/lib".to_vec()
            ]
        );
        // A trailing separator would make the revspec name a tree that does not exist.
        assert_eq!(
            gitlink_oid_argv(b"vendor/lib/").last().unwrap(),
            &b"HEAD:vendor/lib".to_vec()
        );
    }

    #[test]
    fn a_submodule_is_its_own_project_with_a_parent_link_never_a_location_of_the_parent() {
        let mut conn = open_test_index();
        let parent = insert_project(
            &conn,
            NewProject {
                name: "super",
                lineage_key: Some("L1"),
                remote_key: None,
                created_at: 100,
            },
        );
        let parent_loc = insert_location(&conn, parent, "/w/super", None);
        // The submodule has a different root commit, so it is a different lineage.
        let child = insert_project(
            &conn,
            NewProject {
                name: "lib",
                lineage_key: Some("L2"),
                remote_key: None,
                created_at: 101,
            },
        );
        insert_location(&conn, child, "/w/super/vendor/lib", None);

        let tx = conn.transaction().unwrap();
        link_submodule(
            &tx,
            &SubmoduleLink {
                parent_project_id: parent,
                parent_location_id: parent_loc,
                child_project_id: child,
                path_bytes: b"vendor/lib".to_vec(),
                gitlink_oid: Some("deadbeef".to_owned()),
            },
            200,
        )
        .unwrap();
        tx.commit().unwrap();

        // Its own project, with the link.
        let (pid, path): (Option<i64>, Option<String>) = conn
            .query_row(
                "SELECT parent_project_id, submodule_path FROM project WHERE id=?1",
                rusqlite::params![child],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(pid, Some(parent));
        assert_eq!(path.as_deref(), Some("vendor/lib"));

        // The parent gained no location — a location whose lineage differs from its project's
        // is the corruption identity exists to prevent.
        let parent_locations: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM location WHERE project_id=?1",
                rusqlite::params![parent],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(parent_locations, 1);

        let (edge_child, oid): (i64, Option<String>) = conn
            .query_row(
                "SELECT child_project_id, gitlink_oid FROM submodule_edge
                  WHERE parent_project_id=?1",
                rusqlite::params![parent],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(edge_child, child);
        assert_eq!(oid.as_deref(), Some("deadbeef"));
    }

    #[test]
    fn relinking_the_same_submodule_does_not_duplicate_the_edge() {
        let mut conn = open_test_index();
        let parent = insert_project(
            &conn,
            NewProject {
                name: "super",
                lineage_key: Some("L1"),
                remote_key: None,
                created_at: 100,
            },
        );
        let parent_loc = insert_location(&conn, parent, "/w/super", None);
        let child = insert_project(
            &conn,
            NewProject {
                name: "lib",
                lineage_key: Some("L2"),
                remote_key: None,
                created_at: 101,
            },
        );

        let link = SubmoduleLink {
            parent_project_id: parent,
            parent_location_id: parent_loc,
            child_project_id: child,
            path_bytes: b"vendor/lib".to_vec(),
            gitlink_oid: None,
        };
        let tx = conn.transaction().unwrap();
        link_submodule(&tx, &link, 200).unwrap();
        link_submodule(
            &tx,
            &SubmoduleLink {
                gitlink_oid: Some("beef".to_owned()),
                ..link
            },
            201,
        )
        .unwrap();
        tx.commit().unwrap();

        let edges: i64 = conn
            .query_row("SELECT COUNT(*) FROM submodule_edge", [], |r| r.get(0))
            .unwrap();
        assert_eq!(edges, 1);
        let oid: Option<String> = conn
            .query_row("SELECT gitlink_oid FROM submodule_edge", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            oid.as_deref(),
            Some("beef"),
            "a rescan updates the pinned commit"
        );
    }
}
