#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! §46.9's path release, on real repositories through the scan's hand-off. A copy this app
//! removed is evidence of nothing on disk now: a different repository found later at its path is
//! not its project.

#[path = "support/handoff_rig.rs"]
mod handoff_rig;

use handoff_rig::Rig;

#[test]
fn a_different_repository_at_a_removed_copys_path_is_not_its_project() {
    let rig = Rig::new();
    let path = rig.repo("x", "first");
    let a = rig.hand_off(&path);
    rig.uninstall(a.location, &path);
    rig.repo("x", "another first");
    let b = rig.hand_off(&path);
    eprintln!("A at the path {a:?}, then B at the same path {b:?}");
    assert_ne!(
        b.project, a.project,
        "a different repository joined the removed copy's project"
    );
}

/// A location row's owner and its three path columns: `(project_id, path_bytes, path_key,
/// path_display)`.
fn path_row(rig: &Rig, location: i64) -> (i64, Vec<u8>, Vec<u8>, String) {
    rig.read(|conn| {
        conn.query_row(
            "SELECT project_id, path_bytes, path_key, path_display FROM location WHERE id = ?1",
            [location],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap()
    })
}

/// The `(project_id, location_id)` one row of `table` names.
fn names(rig: &Rig, table: &str) -> (i64, i64) {
    rig.read(|conn| {
        conn.query_row(
            &format!("SELECT project_id, location_id FROM {table}"),
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap()
    })
}

/// The removed copy's row keeps its path, its project, its session and its removal record; only
/// its key moves, to the found key, a NUL byte and its own id, which no path can produce. The
/// repository found at the path gets a row of its own, and finding it again finds that row.
#[test]
fn ac_p4_46_19_a_different_repository_at_a_removed_copys_path_gets_a_new_row() {
    let rig = Rig::new();
    let path = rig.repo("x", "first");
    let a = rig.hand_off(&path);
    let (pa, l1) = (a.project.0, a.location.0);
    let before = path_row(&rig, l1);
    rig.execute(
        "INSERT INTO session (project_id, location_id, started_at, ended_at, close_reason)
         VALUES (?1, ?2, 100, 200, 'stop')",
        [pa, l1],
    );
    rig.execute(
        "INSERT INTO removal_record
           (project_id, location_id, kind, state, path_bytes, planned, lineage_key,
            state_digest, recovery, session_nonce, started_at, ended_at)
         SELECT p.id, l.id, 'uninstall', 'done', l.path_bytes, 'trash', p.lineage_key,
                'digest', 'remote', X'00', 100, 200
           FROM location l JOIN project p ON p.id = l.project_id
          WHERE l.id = ?1",
        [l1],
    );
    rig.uninstall(a.location, &path);

    rig.repo("x", "another first");
    let b = rig.hand_off(&path);
    let (pb, l2) = (b.project.0, b.location.0);
    let old = path_row(&rig, l1);
    let new = path_row(&rig, l2);
    let mut released = before.2.clone();
    released.push(0);
    released.extend_from_slice(l1.to_string().as_bytes());
    eprintln!("A ({pa}, {l1}), then B ({pb}, {l2})");
    eprintln!("L1 before {before:?}");
    eprintln!("L1 after  {old:?}");
    eprintln!("L2        {new:?}");
    eprintln!(
        "session {:?}, removal record {:?}, {} location rows",
        names(&rig, "session"),
        names(&rig, "removal_record"),
        rig.count("location")
    );
    assert_ne!(l2, l1, "the removed copy's row moved to another project");
    assert_ne!(
        pb, pa,
        "a different repository joined the removed copy's project"
    );
    assert_eq!(
        (old.0, &old.1, &old.3),
        (pa, &before.1, &before.3),
        "the removed copy's row lost its project or its path"
    );
    assert_eq!(
        old.2, released,
        "the released key is not the found key, a NUL and its id"
    );
    assert_eq!(new.2, before.2, "the new row does not hold the found key");
    assert_eq!(names(&rig, "session"), (pa, l1));
    assert_eq!(names(&rig, "removal_record"), (pa, l1));
    assert_eq!(rig.count("location"), 2);

    let again = rig.hand_off(&path);
    eprintln!("B found again {again:?}");
    assert_eq!(again.location.0, l2, "finding B again did not find its row");
    assert_eq!(rig.count("location"), 2);
}
