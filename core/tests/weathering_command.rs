#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! §33.8's handler, over a real migrated database.
//!
//! Three projects and three different facts: one with an `art_scene` row, one without, and one
//! whose `scene_json` will not deserialise. The third is the one that matters — **an empty layer
//! set and an unreadable document are different facts**, and answering the first for the second
//! is the absent-versus-unreadable conflation A13 forbids one level up.

use codotheca_core::art::generate::{generate, SceneInputs};
use codotheca_core::art::scene::{canonical_json, scene_hash};
use codotheca_core::index::migrate::{apply_all, MIGRATIONS};
use codotheca_core::index::{open_connection, Index};
use codotheca_core::protocol::{ProjectId, SceneHash};
use codotheca_core::weathering::store::SqliteSceneSource;
use codotheca_core::weathering::{
    dispatch_weathering_command, weathering_for, WeatheringCtx, WeatheringError,
};

fn fresh() -> (tempfile::TempDir, rusqlite::Connection) {
    let dir = tempfile::tempdir().unwrap();
    let mut conn = open_connection(&Index::db_path(dir.path())).unwrap();
    apply_all(&mut conn, MIGRATIONS).unwrap();
    (dir, conn)
}

fn insert_project(conn: &rusqlite::Connection, name: &str) -> ProjectId {
    conn.execute(
        "INSERT INTO project (name, seed_basename, lineage_key, created_at, updated_at)
         VALUES (?1, ?1, 'abc123', 1, 1)",
        [name],
    )
    .unwrap();
    ProjectId(conn.last_insert_rowid())
}

fn put_scene_json(conn: &rusqlite::Connection, id: ProjectId, hash: &str, json: &str) {
    conn.execute(
        "INSERT INTO art_scene (project_id, scene_hash, scene_json, schema_version, state)
         VALUES (?1, ?2, ?3, 1, 'ready')",
        rusqlite::params![id.0, hash, json],
    )
    .unwrap();
}

/// A project carrying a real generated scene, with the hash that addresses it.
fn with_scene(conn: &rusqlite::Connection, name: &str) -> (ProjectId, String) {
    let id = insert_project(conn, name);
    let scene = generate(&SceneInputs {
        seed_basename: name.to_owned(),
        archetype: Some("site".to_owned()),
        primary_language: Some("Rust".to_owned()),
        size_tracked_bytes: Some(4_000_000),
        ..SceneInputs::default()
    });
    let hash = scene_hash(&scene).unwrap();
    let json = String::from_utf8(canonical_json(&scene).unwrap()).unwrap();
    put_scene_json(conn, id, &hash, &json);
    (id, hash)
}

#[test]
fn a_project_with_a_scene_answers_its_hash_the_space_and_five_layers() {
    let (_d, conn) = fresh();
    let (id, hash) = with_scene(&conn, "has-a-scene");

    let reply = weathering_for(&SqliteSceneSource::new(&conn), &conn, id).unwrap();
    assert_eq!(reply.project_id, id);
    assert_eq!(reply.scene_hash, Some(SceneHash(hash)));
    assert_eq!((reply.space_w, reply.space_h), (600, 900));
    assert_eq!(reply.layers.len(), 5);
}

#[test]
fn a_project_with_no_art_scene_row_answers_a_null_hash_and_no_layers() {
    let (_d, conn) = fresh();
    let id = insert_project(&conn, "never-rendered");

    let reply = weathering_for(&SqliteSceneSource::new(&conn), &conn, id).unwrap();
    assert_eq!(reply.scene_hash, None);
    assert!(
        reply.layers.is_empty(),
        "five empty entries and no hash would claim a surface that does not exist"
    );
}

#[test]
fn an_unreadable_scene_document_is_an_error_and_never_an_empty_reply() {
    let (_d, conn) = fresh();
    let id = insert_project(&conn, "corrupt");
    put_scene_json(&conn, id, "deadbeef", "{\"v\":1,\"not\":\"a scene\"}");

    let error = weathering_for(&SqliteSceneSource::new(&conn), &conn, id)
        .expect_err("a document that will not deserialise is not an empty layer set");
    assert!(matches!(error, WeatheringError::UnreadableScene(who, _) if who == id));
}

#[test]
fn an_unknown_project_is_refused_rather_than_answered_empty() {
    let (_d, conn) = fresh();
    let error = weathering_for(&SqliteSceneSource::new(&conn), &conn, ProjectId(9_999))
        .expect_err("no project is not a project with no scene");
    assert_eq!(error, WeatheringError::NoProject(ProjectId(9_999)));
}

#[test]
fn every_anchor_lies_inside_the_declared_space() {
    let (_d, conn) = fresh();
    let mut scanned = 0_usize;
    for name in ["cli-shaped", "site-shaped", "docs-shaped", "unclassified"] {
        let (id, _) = with_scene(&conn, name);
        let reply = weathering_for(&SqliteSceneSource::new(&conn), &conn, id).unwrap();
        let (w, h) = (i64::from(reply.space_w), i64::from(reply.space_h));
        for layer in &reply.layers {
            for r in &layer.rects {
                assert!(i64::from(r.x) >= 0 && i64::from(r.x) + i64::from(r.w) <= w);
                assert!(i64::from(r.y) >= 0 && i64::from(r.y) + i64::from(r.h) <= h);
            }
            for p in &layer.points {
                assert!(i64::from(p.x) >= 0 && i64::from(p.x) <= w);
                assert!(i64::from(p.y) >= 0 && i64::from(p.y) <= h);
            }
            for path in &layer.paths {
                for p in &path.points {
                    assert!(i64::from(p.x) >= 0 && i64::from(p.x) <= w);
                    assert!(i64::from(p.y) >= 0 && i64::from(p.y) <= h);
                }
            }
        }
        scanned += 1;
    }
    assert!(scanned > 0, "scanned no project at all");
    eprintln!("§33.8: {scanned} projects scanned for in-space anchors");
}

#[test]
fn the_dispatcher_owns_its_command_and_declines_every_other() {
    let dir = tempfile::tempdir().unwrap();
    let index = Index::open(dir.path()).unwrap();
    let (id, _) = with_scene(index.conn(), "dispatched");
    let ctx = WeatheringCtx { index: &index };

    let answer = dispatch_weathering_command(
        &ctx,
        "health.weathering",
        serde_json::json!({ "projectId": id.0 }),
    )
    .expect("this module owns health.weathering")
    .expect("and answers it");
    assert_eq!(answer["projectId"], serde_json::json!(id.0));
    assert_eq!(answer["layers"].as_array().unwrap().len(), 5);
    assert!(
        answer.get("computedAt").is_none(),
        "the anchor set is a derivation, not an observation"
    );

    assert!(
        dispatch_weathering_command(&ctx, "projects.get", serde_json::json!({})).is_none(),
        "answering for a neighbour takes the command away from the plan that owns it"
    );
}
