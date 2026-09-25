#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! §8's shelf view state. `savedAt` is what makes a fresh install distinguishable from a client
//! that deliberately saved an empty shelf — *never render unknown as zero*, on the wire.

use codotheca_core::index::Index;
use codotheca_core::protocol::{ProjectId, SortKey, ViewMode, ViewPatch};
use codotheca_core::view::state;

const fn empty_patch() -> ViewPatch {
    ViewPatch {
        query: None,
        sort: None,
        view_mode: None,
        density: None,
        collapsed_sections: None,
        scroll_offset: None,
        selected_project_id: None,
        dismissed_notices: None,
        window_geometry: None,
    }
}

fn seed_project(conn: &rusqlite::Connection, id: i64) {
    conn.execute(
        "INSERT INTO project (id, name, seed_basename, created_at, updated_at, last_touched_at)
         VALUES (?1, 'alpha', 'alpha', 1, 1, 1)",
        [id],
    )
    .expect("seed project");
}

fn opened() -> (tempfile::TempDir, Index) {
    let dir = tempfile::tempdir().expect("tempdir");
    let index = Index::open(dir.path()).expect("open");
    (dir, index)
}

#[test]
fn a_fresh_install_is_distinguishable_from_a_client_that_saved_an_empty_view() {
    let (_dir, index) = opened();

    let fresh = state::load(index.conn()).expect("read");
    assert_eq!(fresh.saved_at, None, "nothing has ever written a view");
    assert_eq!(fresh.query, "");
    assert_eq!(fresh.sort, SortKey::LastTouched);
    assert_eq!(fresh.view_mode, ViewMode::Grid);
    assert_eq!(fresh.density, state::DEFAULT_DENSITY);
    assert!(fresh.collapsed_sections.is_empty());
    assert_eq!(fresh.window_geometry, None);

    // The same visible fields, deliberately saved. Everything on screen matches the line above;
    // the one thing that differs is the fact of having been saved, and it is on the wire.
    state::store(
        index.conn(),
        &ViewPatch {
            query: Some(String::new()),
            ..empty_patch()
        },
        1_700_000_042,
    )
    .expect("write");
    let saved = state::load(index.conn()).expect("read");
    assert_eq!(saved.query, "");
    assert_eq!(saved.saved_at, Some(1_700_000_042));
}

#[test]
fn a_null_field_leaves_its_key_alone() {
    let (_dir, index) = opened();
    state::store(
        index.conn(),
        &ViewPatch {
            view_mode: Some(ViewMode::List),
            density: Some(232),
            ..empty_patch()
        },
        10,
    )
    .expect("first");
    state::store(
        index.conn(),
        &ViewPatch {
            scroll_offset: Some(940.0),
            ..empty_patch()
        },
        20,
    )
    .expect("second");
    let after = state::load(index.conn()).expect("read");
    assert_eq!(
        after.view_mode,
        ViewMode::List,
        "an untouched field survives the next patch"
    );
    assert_eq!(after.density, 232);
    assert!((after.scroll_offset - 940.0).abs() < f64::EPSILON);
    assert_eq!(after.saved_at, Some(20));
}

#[test]
fn dismissed_notices_keep_one_row_per_notice_and_a_patch_replaces_the_set() {
    let (_dir, index) = opened();
    state::store(
        index.conn(),
        &ViewPatch {
            dismissed_notices: Some(vec!["identity".into(), "git_too_old".into()]),
            ..empty_patch()
        },
        30,
    )
    .expect("write");

    // §8.0's convention, from §1.9: `notice.dismissed.<id>`, unscoped, one row each — so a
    // section that reads one key directly still finds it.
    let stored: i64 = index
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM view_state WHERE k LIKE 'notice.dismissed.%'",
            [],
            |r| r.get(0),
        )
        .expect("count");
    assert_eq!(stored, 2);
    assert_eq!(
        state::load(index.conn()).expect("read").dismissed_notices,
        vec!["git_too_old".to_owned(), "identity".to_owned()]
    );

    state::store(
        index.conn(),
        &ViewPatch {
            dismissed_notices: Some(vec!["identity".into()]),
            ..empty_patch()
        },
        40,
    )
    .expect("rewrite");
    assert_eq!(
        state::load(index.conn()).expect("read").dismissed_notices,
        vec!["identity".to_owned()]
    );
}

#[test]
fn a_selection_whose_project_is_gone_is_not_a_selection() {
    let (_dir, index) = opened();
    seed_project(index.conn(), 7);
    state::store(
        index.conn(),
        &ViewPatch {
            selected_project_id: Some(ProjectId(7)),
            ..empty_patch()
        },
        50,
    )
    .expect("write");
    assert_eq!(
        state::load(index.conn()).expect("read").selected_project_id,
        Some(ProjectId(7))
    );

    index
        .conn()
        .execute("DELETE FROM project WHERE id = 7", [])
        .expect("drop");
    // Never claim currency you do not have: a stored id that no longer names a row is not a
    // selection, and restoring it would put the shelf's cursor on nothing.
    assert_eq!(
        state::load(index.conn()).expect("read").selected_project_id,
        None
    );
}

#[test]
fn an_unreadable_stored_value_falls_back_to_the_default_rather_than_failing_the_read() {
    let (_dir, index) = opened();
    index
        .conn()
        .execute(
            "INSERT OR REPLACE INTO view_state (k, v) VALUES ('shelf.sort', 'completion')",
            [],
        )
        .expect("write junk");
    // §8's SORT control drops `Completion` outright — nothing computes it in phase 1 — so a value
    // a rolled-back build wrote must not take the shelf down with it.
    assert_eq!(
        state::load(index.conn()).expect("read").sort,
        SortKey::LastTouched
    );
}

#[test]
fn the_core_stamps_saved_at_and_a_client_cannot_forge_it() {
    // It is provenance. `ViewPatch` has no `savedAt` field at all, so a client sending one is a
    // `PROTOCOL` refusal against `deny_unknown_fields` rather than a value that lands.
    let (_dir, index) = opened();
    let ctx = codotheca_core::view::ViewCtx {
        index: &index,
        now: 1_700_000_777,
    };
    let err = state::handle_view_set(
        &ctx,
        serde_json::json!({ "patch": {
            "query": null, "sort": null, "viewMode": null, "density": null,
            "collapsedSections": null, "scrollOffset": null, "selectedProjectId": null,
            "dismissedNotices": null, "windowGeometry": null, "savedAt": 1
        }}),
    )
    .expect_err("a forged provenance does not deserialise");
    assert_eq!(err.code, codotheca_core::protocol::ErrorCode::Protocol);
    assert_eq!(state::load(index.conn()).expect("read").saved_at, None);

    state::handle_view_set(
        &ctx,
        serde_json::json!({ "patch": {
            "query": null, "sort": null, "viewMode": null, "density": null,
            "collapsedSections": null, "scrollOffset": null, "selectedProjectId": null,
            "dismissedNotices": null, "windowGeometry": null
        }}),
    )
    .expect("set");
    assert_eq!(
        state::load(index.conn()).expect("read").saved_at,
        Some(1_700_000_777),
        "the core's clock, not the caller's"
    );
}
