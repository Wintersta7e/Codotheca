//! §46.14's two sidecar sections: a parcel with its refs and a removal record with its log travel
//! by the key of the copy they hang off, keep their ids, and wait rather than guess.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use codotheca_core::index::pending::{match_pending, stage_pending, PendingRecord};
use codotheca_core::index::rebuild::{rebuild_in_place, RebuildOutcome};
use codotheca_core::index::sidecar::{export, RestoreCtx, RestoreOutcome, SectionRow};
use codotheca_core::index::subject::subject_for_project;
use codotheca_core::index::{Index, IndexError};
use codotheca_core::protocol::{LocationId, ProjectId};
use codotheca_core::removal::sidecar::{
    export_parcels, export_parcels_section, export_removal_records, export_removal_records_section,
    reserve_ids, restore_parcel, restore_parcels_section, restore_removal_record,
    restore_removal_records_section, SidecarParcel, SidecarRemovalRecord,
};
use rusqlite::types::Value;
use rusqlite::{params, Connection, OptionalExtension as _};

const NOW: i64 = 1_760_000_000;

/// The exported parcel's id and the record's: neither is a table's first, so a restore that let
/// the table choose would show.
const PARCEL: i64 = 7;
const RECORD: i64 = 9;

/// A repository path no UTF-8 decoder accepts: a nested repository named in raw bytes.
const NON_UTF8_REPO: &[u8] = b"vendor/\xff\xfe-lib";

fn open() -> (tempfile::TempDir, Index) {
    let dir = tempfile::tempdir().unwrap();
    let index = Index::open_at(dir.path(), NOW).unwrap();
    (dir, index)
}

fn project(conn: &Connection, name: &str) -> i64 {
    conn.execute(
        "INSERT INTO project (name, seed_basename, lineage_key, created_at, updated_at)
         VALUES (?1, ?1, ?2, 1, 1)",
        params![name, format!("{name}-lineage")],
    )
    .unwrap();
    conn.last_insert_rowid()
}

/// A copy of `project` at `path`, removed at `removed_at` when one is given.
fn copy(conn: &Connection, project: i64, path: &str, removed_at: Option<i64>) -> i64 {
    conn.execute(
        "INSERT INTO location (project_id, kind, path_bytes, path_key, path_display, store_key,
                               presence, repo_kind, removed_at)
         VALUES (?1, 'linux', ?2, ?2, ?3, 's', ?4, 'worktree', ?5)",
        params![
            project,
            path.as_bytes(),
            path,
            if removed_at.is_some() {
                "missing"
            } else {
                "present"
            },
            removed_at
        ],
    )
    .unwrap();
    conn.last_insert_rowid()
}

/// A sealed parcel of `location`'s copy, at the id given or the table's next, with two refs: one
/// in the top-level repository and one under a path that is not UTF-8. Answers its id.
fn sealed_parcel(conn: &Connection, at: Option<i64>, project: i64, location: i64) -> i64 {
    conn.execute(
        "INSERT INTO parcel (id, project_id, location_id, state, folder_bytes, dir_name,
                             staging_bytes, volume_key, store_class, lineage_key, state_digest,
                             manifest_sha256, total_bytes, git_version, tar_version,
                             session_nonce, created_at, sealed_at, checked_at, full_checked_at,
                             full_checked_git, check_result, purged_at)
         VALUES (?1, ?2, ?3, 'sealed', x'2f6b6565702fff', 'kept-7', x'2f6b6565702f2e7374616765',
                 'vol-1', 'local', 'kept-lineage', 'digest-a', 'manifest-a', 4096, '2.43.0',
                 '1.34', x'00ff10', 100, 110, 120, 125, '2.43.0', 'ok', NULL)",
        params![at, project, location],
    )
    .unwrap();
    let id = conn.last_insert_rowid();
    for (repo_path, ref_name, oid) in [
        (
            &b""[..],
            "refs/heads/main",
            "1111111111111111111111111111111111111111",
        ),
        (
            NON_UTF8_REPO,
            "refs/tags/v1",
            "2222222222222222222222222222222222222222",
        ),
    ] {
        conn.execute(
            "INSERT INTO parcel_ref (parcel_id, repo_path, ref_name, oid) VALUES (?1, ?2, ?3, ?4)",
            params![id, repo_path, ref_name, oid],
        )
        .unwrap();
    }
    id
}

/// A finished removal of `location`'s copy, at the id given or the table's next, recovered
/// through `parcel`, with its log. Answers its id.
fn done_record(
    conn: &Connection,
    at: Option<i64>,
    project: i64,
    location: i64,
    parcel: i64,
) -> i64 {
    conn.execute(
        "INSERT INTO removal_record (id, project_id, location_id, kind, state, path_bytes, planned,
                                     holding_bytes, lineage_key, state_digest, recovery,
                                     remotes_json, remote_verified_at, parcel_id, disposal,
                                     readme_name, readme_text, readme_truncated, session_nonce,
                                     started_at, ended_at)
         VALUES (?1, ?2, ?3, 'uninstall', 'done', x'2f722f6b6570742dfe', 'trash',
                 x'2f686f6c642f39', 'kept-lineage', 'digest-a', 'parcel', NULL, NULL, ?4,
                 'trashed', 'README.md', 'what it was', 0, x'00ff10', 100, 140)",
        params![at, project, location, parcel],
    )
    .unwrap();
    let id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO removal_log (removal_id, format, log) VALUES (?1, 1, 'abc first commit')",
        [id],
    )
    .unwrap();
    id
}

/// An index holding one live project with a live copy and a removed one; the removed copy has a
/// sealed parcel and the finished record naming it. Answers the project and its removed copy.
fn exported_side(index: &Index) -> (i64, i64) {
    let conn = index.conn();
    let kept = project(conn, "kept");
    copy(conn, kept, "/r/kept", None);
    let gone = copy(conn, kept, "/r/kept-old", Some(90));
    sealed_parcel(conn, Some(PARCEL), kept, gone);
    done_record(conn, Some(RECORD), kept, gone, PARCEL);
    (kept, gone)
}

/// The parcels and records `exported_side` writes, exported.
fn exported() -> (Vec<SidecarParcel>, Vec<SidecarRemovalRecord>) {
    let (_dir, index) = open();
    let (kept, _) = exported_side(&index);
    let parcels = export_parcels(index.conn(), ProjectId(kept)).unwrap();
    let records = export_removal_records(index.conn(), ProjectId(kept)).unwrap();
    eprintln!(
        "exported: {} parcels with {} refs, {} records, {} logs",
        parcels.len(),
        parcels.iter().map(|p| p.refs.len()).sum::<usize>(),
        records.len(),
        records.iter().filter(|r| r.log.is_some()).count()
    );
    assert_eq!(parcels.len(), 1);
    assert_eq!(parcels[0].refs.len(), 2, "a parcel travels with its refs");
    assert_eq!(records.len(), 1);
    assert!(records[0].log.is_some(), "a record travels with its log");
    (parcels, records)
}

/// Every row of `table`, every column, in key order.
fn rows_of(conn: &Connection, table: &str) -> Vec<Vec<Value>> {
    let mut st = conn
        .prepare(&format!("SELECT * FROM {table} ORDER BY 1, 2"))
        .unwrap();
    let width = st.column_count();
    st.query_map([], |r| (0..width).map(|i| r.get::<_, Value>(i)).collect())
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

/// The four removal tables' rows, in table order.
fn removal_rows(conn: &Connection) -> Vec<Vec<Vec<Value>>> {
    ["parcel", "parcel_ref", "removal_record", "removal_log"]
        .iter()
        .map(|table| rows_of(conn, table))
        .collect()
}

fn sequence(conn: &Connection, table: &str) -> Option<i64> {
    conn.query_row(
        "SELECT seq FROM sqlite_sequence WHERE name = ?1",
        [table],
        |r| r.get(0),
    )
    .optional()
    .unwrap()
}

/// The blob columns of both parent rows and every ref path, as bytes.
fn blobs(conn: &Connection) -> Vec<Vec<u8>> {
    let mut out: Vec<Vec<u8>> = conn
        .query_row(
            "SELECT folder_bytes, staging_bytes, session_nonce FROM parcel WHERE id = ?1",
            [PARCEL],
            |r| Ok(vec![r.get(0)?, r.get(1)?, r.get(2)?]),
        )
        .unwrap();
    out.extend(
        conn.query_row(
            "SELECT path_bytes, holding_bytes, session_nonce FROM removal_record WHERE id = ?1",
            [RECORD],
            |r| Ok(vec![r.get::<_, Vec<u8>>(0)?, r.get(1)?, r.get(2)?]),
        )
        .unwrap(),
    );
    let refs: Vec<Vec<u8>> = conn
        .prepare("SELECT repo_path FROM parcel_ref WHERE parcel_id = ?1 ORDER BY repo_path")
        .unwrap()
        .query_map([PARCEL], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    out.extend(refs);
    out
}

/// Restore the parcel and then the record into `index` for `project`.
fn restore_both(
    index: &mut Index,
    project: i64,
    parcel: &SidecarParcel,
    record: &SidecarRemovalRecord,
) -> (bool, bool) {
    index
        .with_tx(|tx| {
            Ok((
                restore_parcel(tx, ProjectId(project), parcel)?,
                restore_removal_record(tx, ProjectId(project), record)?,
            ))
        })
        .unwrap()
}

#[test]
fn a_parcel_and_a_record_round_trip_under_new_location_ids() {
    let (_a_dir, a) = open();
    let (kept, gone) = exported_side(&a);
    let parcels = export_parcels(a.conn(), ProjectId(kept)).unwrap();
    let records = export_removal_records(a.conn(), ProjectId(kept)).unwrap();
    assert_eq!(parcels.len(), 1);
    assert_eq!(parcels[0].refs.len(), 2, "a parcel travels with its refs");
    assert_eq!(records.len(), 1);

    // The same project and copies in a fresh index, under other ids.
    let (_b_dir, mut b) = open();
    let (b_kept, b_gone) = {
        let conn = b.conn();
        let unrelated = project(conn, "unrelated");
        copy(conn, unrelated, "/r/unrelated", None);
        let b_kept = project(conn, "kept");
        copy(conn, b_kept, "/r/kept", None);
        (b_kept, copy(conn, b_kept, "/r/kept-old", Some(90)))
    };
    eprintln!("removed copy: location {gone} exported, {b_gone} in the fresh index");
    assert_ne!(gone, b_gone);

    let applied = restore_both(&mut b, b_kept, &parcels[0], &records[0]);
    assert_eq!(applied, (true, true));

    let again = (
        export_parcels(b.conn(), ProjectId(b_kept)).unwrap(),
        export_removal_records(b.conn(), ProjectId(b_kept)).unwrap(),
    );
    assert_eq!(again.0, parcels, "the parcel came back changed");
    assert_eq!(again.1, records, "the record came back changed");

    let placed = |table: &str, id: i64| -> (i64, i64) {
        b.conn()
            .query_row(
                &format!("SELECT project_id, location_id FROM {table} WHERE id = ?1"),
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap()
    };
    assert_eq!(placed("parcel", PARCEL), (b_kept, b_gone), "parcel ids");
    assert_eq!(
        placed("removal_record", RECORD),
        (b_kept, b_gone),
        "record ids"
    );

    let (was, now) = (blobs(a.conn()), blobs(b.conn()));
    eprintln!("blobs compared byte for byte: {}", was.len());
    assert_eq!(was.len(), 8);
    assert_eq!(now, was);

    let (parcel_seq, record_seq) = (
        sequence(b.conn(), "parcel").unwrap(),
        sequence(b.conn(), "removal_record").unwrap(),
    );
    eprintln!("sequences after the restore: parcel {parcel_seq}, removal_record {record_seq}");
    assert!(parcel_seq >= PARCEL);
    assert!(record_seq >= RECORD);

    let fresh = sealed_parcel(b.conn(), None, b_kept, b_gone);
    eprintln!("a parcel made after the restore: {fresh}");
    assert!(fresh > PARCEL, "parcel {fresh} reuses a restored id");
}

#[test]
fn a_record_whose_location_is_not_there_stays_pending() {
    let (parcels, records) = exported();
    let (_dir, mut b) = open();
    let b_kept = {
        let conn = b.conn();
        let b_kept = project(conn, "kept");
        copy(conn, b_kept, "/r/kept", None);
        // The key exists, but on another project's copy: never this project's.
        let other = project(conn, "other");
        copy(conn, other, "/r/kept-old", Some(90));
        b_kept
    };
    let applied = restore_both(&mut b, b_kept, &parcels[0], &records[0]);
    let written: usize = removal_rows(b.conn()).iter().map(Vec::len).sum();
    eprintln!("applied {applied:?}, rows written {written}");
    assert_eq!(applied, (false, false));
    assert_eq!(written, 0);
}

#[test]
fn a_record_before_its_parcel_stays_pending_and_applies_after() {
    let (parcels, records) = exported();
    let (_dir, mut b) = open();
    let b_kept = {
        let conn = b.conn();
        let b_kept = project(conn, "kept");
        copy(conn, b_kept, "/r/kept-old", Some(90));
        b_kept
    };
    let early = b
        .with_tx(|tx| restore_removal_record(tx, ProjectId(b_kept), &records[0]))
        .unwrap();
    let written: usize = removal_rows(b.conn()).iter().map(Vec::len).sum();
    eprintln!("the record before its parcel: {early}, rows written {written}");
    assert!(!early);
    assert_eq!(written, 0);

    let applied = restore_both(&mut b, b_kept, &parcels[0], &records[0]);
    eprintln!("then the parcel and the record: {applied:?}");
    assert_eq!(applied, (true, true));
}

#[test]
fn an_existing_id_is_never_overwritten() {
    let (parcels, records) = exported();
    let (_dir, mut b) = open();
    let b_kept = {
        let conn = b.conn();
        let b_kept = project(conn, "kept");
        copy(conn, b_kept, "/r/kept-old", Some(90));
        b_kept
    };
    assert_eq!(
        restore_both(&mut b, b_kept, &parcels[0], &records[0]),
        (true, true)
    );
    b.conn()
        .execute_batch(
            "UPDATE parcel SET check_result = 'missing', checked_at = 999;
             UPDATE parcel_ref SET oid = 'present';
             UPDATE removal_record SET ended_at = 999;
             UPDATE removal_log SET log = 'present'",
        )
        .unwrap();
    let present = removal_rows(b.conn());

    let again = restore_both(&mut b, b_kept, &parcels[0], &records[0]);
    eprintln!(
        "restored over present rows: {again:?}, rows held {}",
        present.iter().map(Vec::len).sum::<usize>()
    );
    assert_eq!(again, (false, false));
    assert_eq!(removal_rows(b.conn()), present);
}

#[test]
fn an_open_record_beside_another_open_one_on_its_copy_stays_pending() {
    let (parcels, records) = exported();
    let (_dir, mut b) = open();
    let (b_kept, gone) = {
        let conn = b.conn();
        let b_kept = project(conn, "kept");
        (b_kept, copy(conn, b_kept, "/r/kept-old", Some(90)))
    };
    let parcel_in = b
        .with_tx(|tx| restore_parcel(tx, ProjectId(b_kept), &parcels[0]))
        .unwrap();
    // The copy already holds an open record, under another id.
    let held_open = done_record(b.conn(), Some(RECORD + 1), b_kept, gone, PARCEL);
    b.conn()
        .execute(
            "UPDATE removal_record SET state = 'journaled', disposal = NULL, ended_at = NULL
              WHERE id = ?1",
            [held_open],
        )
        .unwrap();
    // A finished record goes in beside it: the index holds one open record per copy, not one.
    let finished = b
        .with_tx(|tx| restore_removal_record(tx, ProjectId(b_kept), &records[0]))
        .unwrap();
    eprintln!("finished record {} restored: {finished}", records[0].id);
    assert!(finished);
    let present = removal_rows(b.conn());
    let mut incoming = records[0].clone();
    incoming.id = RECORD + 2;
    incoming.state = "interrupted".to_owned();
    incoming.disposal = None;
    incoming.ended_at = None;

    let restored = b.with_tx(|tx| restore_removal_record(tx, ProjectId(b_kept), &incoming));
    eprintln!(
        "parcel restored: {parcel_in}; open record {held_open} held; open record {} restored: \
         {restored:?}",
        incoming.id
    );
    assert!(
        matches!(restored, Ok(false)),
        "the restore must leave the record pending, not fail"
    );
    assert_eq!(removal_rows(b.conn()), present);

    // Once the copy's open record closes, the waiting one applies.
    b.conn()
        .execute(
            "UPDATE removal_record SET state = 'done' WHERE id = ?1",
            [held_open],
        )
        .unwrap();
    let applied = b
        .with_tx(|tx| restore_removal_record(tx, ProjectId(b_kept), &incoming))
        .unwrap();
    eprintln!("after the open record closed: {applied}");
    assert!(applied);
}

#[test]
fn reserve_ids_keeps_new_rows_clear_of_pending_ones_and_never_lowers() {
    let (_dir, mut index) = open();
    let (kept, gone) = {
        let conn = index.conn();
        let kept = project(conn, "kept");
        (kept, copy(conn, kept, "/r/kept-old", Some(90)))
    };
    index
        .with_tx(|tx| reserve_ids(tx, Some(40), Some(50)))
        .unwrap();
    let parcel = sealed_parcel(index.conn(), None, kept, gone);
    let record = done_record(index.conn(), None, kept, gone, parcel);
    eprintln!("after reserving 40 and 50: parcel {parcel}, record {record}");
    assert!(parcel > 40);
    assert!(record > 50);

    index
        .with_tx(|tx| {
            reserve_ids(tx, Some(10), Some(10))?;
            reserve_ids(tx, None, None)
        })
        .unwrap();
    let held = (
        sequence(index.conn(), "parcel").unwrap(),
        sequence(index.conn(), "removal_record").unwrap(),
    );
    eprintln!("after reserving lower ids and none: {held:?}");
    assert_eq!(held, (parcel, record), "a reservation lowered a sequence");
}

#[test]
fn a_released_location_key_round_trips() {
    let (_a_dir, a) = open();
    let (kept, released) = {
        let conn = a.conn();
        let kept = project(conn, "kept");
        copy(conn, kept, "/r/kept", None);
        let released = copy(conn, kept, "/r/kept-old", Some(90));
        // What a path release writes: the old key, a NUL, and the row's id.
        let mut key = b"/r/kept-old".to_vec();
        key.push(0);
        key.extend_from_slice(released.to_string().as_bytes());
        conn.execute(
            "UPDATE location SET path_key = ?2 WHERE id = ?1",
            params![released, key],
        )
        .unwrap();
        sealed_parcel(conn, Some(PARCEL), kept, released);
        done_record(conn, Some(RECORD), kept, released, PARCEL);
        (kept, key)
    };
    let parcels = export_parcels(a.conn(), ProjectId(kept)).unwrap();
    let records = export_removal_records(a.conn(), ProjectId(kept)).unwrap();
    eprintln!(
        "the released key travels as {}",
        parcels[0].location.path_key
    );
    let travels = released.iter().fold(String::new(), |mut text, b| {
        use std::fmt::Write as _;
        write!(text, "{b:02x}").unwrap();
        text
    });
    assert_eq!(parcels[0].location.path_key, travels);
    assert_eq!(records[0].location.path_key, travels);

    let (_b_dir, mut b) = open();
    let b_kept = {
        let conn = b.conn();
        let b_kept = project(conn, "kept");
        copy(conn, b_kept, "/r/kept", None);
        let row = copy(conn, b_kept, "/r/kept-old", Some(90));
        conn.execute(
            "UPDATE location SET path_key = ?2 WHERE id = ?1",
            params![row, released],
        )
        .unwrap();
        b_kept
    };
    assert_eq!(
        restore_both(&mut b, b_kept, &parcels[0], &records[0]),
        (true, true)
    );
    let again = export_parcels(b.conn(), ProjectId(b_kept)).unwrap();
    assert_eq!(again, parcels);
    assert_eq!(
        export_removal_records(b.conn(), ProjectId(b_kept)).unwrap(),
        records
    );
}

/// A project the user removed comes back from the rebuild's restore transaction, since no scan
/// will ever hand it off; its parcel and record come back with it, under their own ids, and
/// nothing is left waiting.
#[test]
fn a_rebuild_restores_a_removed_projects_parcels_and_records_with_no_scan() {
    let (dir, index) = open();
    {
        let conn = index.conn();
        let removed = project(conn, "removed");
        conn.execute(
            "UPDATE project SET removed_at = 95 WHERE id = ?1",
            [removed],
        )
        .unwrap();
        let gone = copy(conn, removed, "/r/removed-old", Some(90));
        sealed_parcel(conn, Some(PARCEL), removed, gone);
        done_record(conn, Some(RECORD), removed, gone, PARCEL);
    }
    let was = removal_rows(index.conn());
    index.export_sidecar(NOW).unwrap();
    drop(index);
    std::fs::write(Index::db_path(dir.path()), b"this is not a database").unwrap();
    let report = match rebuild_in_place(dir.path(), NOW + 1) {
        Ok(RebuildOutcome::Rebuilt(report)) => report,
        other => panic!("expected a rebuild, got {other:?}"),
    };
    eprintln!(
        "restored: {:?}, pending: {}",
        report.restored, report.pending
    );

    let rebuilt = Index::open_at(dir.path(), NOW + 2).unwrap();
    let conn = rebuilt.conn();
    let scans: i64 = conn
        .query_row("SELECT count(*) FROM scan_run", [], |r| r.get(0))
        .unwrap();
    let waiting: i64 = conn
        .query_row("SELECT count(*) FROM sidecar_pending", [], |r| r.get(0))
        .unwrap();
    let now = removal_rows(conn);
    eprintln!(
        "rows back: {}, scans run: {scans}, waiting: {waiting}",
        now.iter().map(Vec::len).sum::<usize>()
    );
    assert_eq!(now, was, "the removal rows came back changed");
    assert_eq!(report.restored.get("parcels"), Some(&1));
    assert_eq!(report.restored.get("removal_records"), Some(&1));
    assert_eq!(scans, 0, "a scan ran");
    assert_eq!((report.pending, waiting), (0, 0), "a record still waits");
}

/// A section's restore, as the registry holds it.
type Restore = fn(
    &rusqlite::Transaction<'_>,
    &SectionRow,
    &RestoreCtx<'_>,
) -> Result<RestoreOutcome, IndexError>;

fn unhex(text: &str) -> Vec<u8> {
    text.as_bytes()
        .chunks(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect()
}

/// `row` through `restore` as the matcher hands it over: matched to `project`, each of its keys
/// resolved to that project's copy there, if it has one.
fn restore_row(
    index: &mut Index,
    project: Option<i64>,
    row: &SectionRow,
    restore: Restore,
) -> RestoreOutcome {
    index
        .with_tx(|tx| {
            let mut locations = Vec::new();
            for key in &row.location_keys {
                let found: Option<i64> = tx
                    .query_row(
                        "SELECT id FROM location
                          WHERE project_id = ?1 AND kind = ?2 AND distro = ?3 AND path_key = ?4",
                        params![project, key.kind, key.distro, unhex(&key.path_key)],
                        |r| r.get(0),
                    )
                    .optional()?;
                if let Some(id) = found {
                    locations.push((key.clone(), LocationId(id)));
                }
            }
            let ctx = RestoreCtx {
                now: NOW,
                project: project.map(ProjectId),
                locations: &locations,
                source_generation: 1,
            };
            restore(tx, row, &ctx)
        })
        .unwrap()
}

fn written(index: &Index) -> usize {
    removal_rows(index.conn()).iter().map(Vec::len).sum()
}

#[test]
fn adapters_round_trip_through_section_rows() {
    let (_a_dir, a) = open();
    let (kept, _) = exported_side(&a);
    let parcels = export_parcels_section(a.conn()).unwrap();
    let records = export_removal_records_section(a.conn()).unwrap();
    eprintln!(
        "section rows: {} parcels, {} records",
        parcels.len(),
        records.len()
    );
    assert_eq!((parcels.len(), records.len()), (1, 1));
    let subject = subject_for_project(a.conn(), ProjectId(kept))
        .unwrap()
        .unwrap()
        .to_key();
    let typed = (
        export_parcels(a.conn(), ProjectId(kept)).unwrap(),
        export_removal_records(a.conn(), ProjectId(kept)).unwrap(),
    );
    for (row, key) in [
        (&parcels[0], &typed.0[0].location),
        (&records[0], &typed.1[0].location),
    ] {
        assert_eq!(row.subject.as_deref(), Some(subject.as_str()));
        assert_eq!(
            row.location_keys,
            std::slice::from_ref(key),
            "a row names its own copy"
        );
    }

    let (_b_dir, mut b) = open();
    let b_kept = {
        let conn = b.conn();
        let unrelated = project(conn, "unrelated");
        copy(conn, unrelated, "/r/unrelated", None);
        let b_kept = project(conn, "kept");
        copy(conn, b_kept, "/r/kept", None);
        copy(conn, b_kept, "/r/kept-old", Some(90));
        b_kept
    };
    let outcomes = [
        restore_row(&mut b, Some(b_kept), &parcels[0], restore_parcels_section),
        restore_row(
            &mut b,
            Some(b_kept),
            &records[0],
            restore_removal_records_section,
        ),
    ];
    eprintln!("restored through the adapters: {outcomes:?}");
    assert_eq!(outcomes, [RestoreOutcome::Applied(1); 2]);
    assert_eq!(
        export_parcels(b.conn(), ProjectId(b_kept)).unwrap(),
        typed.0
    );
    assert_eq!(
        export_removal_records(b.conn(), ProjectId(b_kept)).unwrap(),
        typed.1
    );
}

#[test]
fn adapters_map_a_row_left_pending_to_pending() {
    let (_a_dir, a) = open();
    exported_side(&a);
    let parcel = export_parcels_section(a.conn()).unwrap().remove(0);
    let record = export_removal_records_section(a.conn()).unwrap().remove(0);

    // The project has no copy at the rows' key.
    let (_b_dir, mut b) = open();
    let b_kept = {
        let conn = b.conn();
        let b_kept = project(conn, "kept");
        copy(conn, b_kept, "/r/kept", None);
        b_kept
    };
    let no_copy = [
        restore_row(&mut b, Some(b_kept), &parcel, restore_parcels_section),
        restore_row(
            &mut b,
            Some(b_kept),
            &record,
            restore_removal_records_section,
        ),
    ];
    eprintln!(
        "no copy at the key: {no_copy:?}, rows written {}",
        written(&b)
    );
    assert_eq!(no_copy, [RestoreOutcome::Pending; 2]);
    assert_eq!(written(&b), 0);

    // No project matched yet.
    let (_c_dir, mut c) = open();
    let c_kept = {
        let conn = c.conn();
        let c_kept = project(conn, "kept");
        copy(conn, c_kept, "/r/kept-old", Some(90));
        c_kept
    };
    let no_project = [
        restore_row(&mut c, None, &parcel, restore_parcels_section),
        restore_row(&mut c, None, &record, restore_removal_records_section),
    ];
    eprintln!("no project: {no_project:?}, rows written {}", written(&c));
    assert_eq!(no_project, [RestoreOutcome::Pending; 2]);
    assert_eq!(written(&c), 0);

    // A record before its parcel waits, and applies once the parcel is in.
    let early = restore_row(
        &mut c,
        Some(c_kept),
        &record,
        restore_removal_records_section,
    );
    eprintln!(
        "the record before its parcel: {early:?}, rows written {}",
        written(&c)
    );
    assert_eq!(early, RestoreOutcome::Pending);
    assert_eq!(written(&c), 0);
    let then = [
        restore_row(&mut c, Some(c_kept), &parcel, restore_parcels_section),
        restore_row(
            &mut c,
            Some(c_kept),
            &record,
            restore_removal_records_section,
        ),
    ];
    eprintln!("then the parcel and the record: {then:?}");
    assert_eq!(then, [RestoreOutcome::Applied(1); 2]);

    // Their ids are now held: a second restore writes nothing.
    let present = removal_rows(c.conn());
    let again = [
        restore_row(&mut c, Some(c_kept), &parcel, restore_parcels_section),
        restore_row(
            &mut c,
            Some(c_kept),
            &record,
            restore_removal_records_section,
        ),
    ];
    eprintln!("over held ids: {again:?}");
    assert_eq!(again, [RestoreOutcome::Pending; 2]);
    assert_eq!(removal_rows(c.conn()), present);
}

#[test]
fn adapters_a_pending_row_survives_the_matcher() {
    // Two removed copies with a parcel each; the index the document is staged into has only the
    // first of them.
    let (_a_dir, a) = open();
    let (kept, _) = exported_side(&a);
    let elsewhere = copy(a.conn(), kept, "/r/kept-elsewhere", Some(91));
    sealed_parcel(a.conn(), Some(PARCEL + 1), kept, elsewhere);
    let mut doc = export(a.conn(), 1, NOW).unwrap();
    // The project's own record would re-create every removed copy, so the document carries the
    // parcels alone.
    doc.payload.projects.clear();
    doc.payload.sections.retain(|name, _| name == "parcels");
    let rows = doc.payload.sections.get("parcels").map_or(0, Vec::len);
    eprintln!("parcels rows in the document: {rows}");
    assert_eq!(rows, 2);

    let (_b_dir, mut b) = open();
    let b_kept = {
        let conn = b.conn();
        let b_kept = project(conn, "kept");
        copy(conn, b_kept, "/r/kept", None);
        copy(conn, b_kept, "/r/kept-old", Some(90));
        b_kept
    };
    let report = b
        .with_tx(|tx| {
            stage_pending(tx, &doc, NOW)?;
            match_pending(tx, ProjectId(b_kept), NOW)
        })
        .unwrap();
    let held: Vec<i64> = b
        .conn()
        .prepare("SELECT id FROM parcel ORDER BY id")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let waiting: Vec<i64> = b
        .conn()
        .prepare("SELECT record FROM sidecar_pending ORDER BY id")
        .unwrap()
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .map(
            |record| match serde_json::from_str(&record.unwrap()).unwrap() {
                PendingRecord::Section { row, .. } => {
                    serde_json::from_value::<SidecarParcel>(row.data)
                        .unwrap()
                        .id
                }
                PendingRecord::Project { .. } => panic!("a project record was staged"),
            },
        )
        .collect();
    eprintln!(
        "applied {:?}; parcels held {held:?}; parcels waiting {waiting:?}",
        report.applied
    );
    assert_eq!(report.applied.get("parcels"), Some(&1));
    assert_eq!(held, [PARCEL]);
    assert_eq!(waiting, [PARCEL + 1], "the unresolved row was dropped");
}

/// Export, close, overwrite the index with bytes SQLite reads as no database, and rebuild.
fn export_corrupt_and_rebuild(dir: &std::path::Path, index: Index, at: i64) {
    index.export_sidecar(at).unwrap();
    drop(index);
    std::fs::write(Index::db_path(dir), b"this is not a database").unwrap();
    match rebuild_in_place(dir, at + 1) {
        Ok(RebuildOutcome::Rebuilt(report)) => eprintln!(
            "rebuilt: restored {:?}, {} pending",
            report.restored, report.pending
        ),
        other => panic!("expected a rebuild, got {other:?}"),
    }
}

/// A live project's parcel and record wait for the scan that brings its copy back. Until then
/// their ids are reserved — through a second rebuild too, when they travel only as pending rows —
/// so a parcel or record made first can never take one.
#[test]
fn a_rebuild_reserves_the_ids_of_parcels_and_records_still_waiting() {
    let (dir, index) = open();
    exported_side(&index);
    export_corrupt_and_rebuild(dir.path(), index, NOW);
    let first = Index::open_at(dir.path(), NOW + 2).unwrap();
    let after_one = (
        sequence(first.conn(), "parcel"),
        sequence(first.conn(), "removal_record"),
    );
    eprintln!("sequences after one rebuild: {after_one:?}");

    export_corrupt_and_rebuild(dir.path(), first, NOW + 10);
    let second = Index::open_at(dir.path(), NOW + 12).unwrap();
    let waiting: i64 = second
        .conn()
        .query_row("SELECT count(*) FROM sidecar_pending", [], |r| r.get(0))
        .unwrap();
    let after_two = (
        sequence(second.conn(), "parcel"),
        sequence(second.conn(), "removal_record"),
    );
    let (parcel, record) = {
        let conn = second.conn();
        let made = project(conn, "made-first");
        let at = copy(conn, made, "/r/made-first-old", Some(90));
        let parcel = sealed_parcel(conn, None, made, at);
        (parcel, done_record(conn, None, made, at, parcel))
    };
    eprintln!(
        "after a second rebuild: sequences {after_two:?}, {waiting} waiting; made first: parcel \
         {parcel}, record {record}"
    );
    assert_eq!(after_one, (Some(PARCEL), Some(RECORD)));
    assert_eq!(after_two, (Some(PARCEL), Some(RECORD)));
    assert!(parcel > PARCEL, "parcel {parcel} takes a waiting id");
    assert!(record > RECORD, "record {record} takes a waiting id");
}
