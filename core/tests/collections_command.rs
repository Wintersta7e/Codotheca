#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! §8.8's collections: a saved **query**, recomputed, never a stored membership list — and a
//! removal that removes a name rather than work (§17, criterion 52).

use codotheca_core::index::Index;
use codotheca_core::protocol::{CollectionId, CollectionKind, CollectionRefusal};
use codotheca_core::query::QUERY_GRAMMAR_VERSION;
use codotheca_core::view::collections::{self, UpsertArgs};

fn query_args(name: &str, text: &str) -> UpsertArgs {
    UpsertArgs {
        id: None,
        name: name.to_owned(),
        kind: CollectionKind::Query,
        query_text: Some(text.to_owned()),
        query_grammar_version: QUERY_GRAMMAR_VERSION,
        sort_index: 0,
    }
}

fn seed_project(conn: &rusqlite::Connection, id: i64, name: &str) {
    conn.execute(
        "INSERT INTO project (id, name, seed_basename, created_at, updated_at, last_touched_at)
         VALUES (?1, ?2, ?2, 1, 1, 1)",
        rusqlite::params![id, name],
    )
    .expect("seed project");
}

fn opened() -> (tempfile::TempDir, Index) {
    let dir = tempfile::tempdir().expect("tempdir");
    let index = Index::open(dir.path()).expect("open");
    (dir, index)
}

/// Every column of every row, in rowid order — criterion 52's before/after comparison.
fn fingerprint(conn: &rusqlite::Connection, table: &str) -> String {
    let cols: Vec<String> = {
        let mut stmt = conn
            .prepare("SELECT name FROM pragma_table_info(?1) ORDER BY cid")
            .expect("table_info");
        let mapped = stmt
            .query_map([table], |r| r.get::<_, String>(0))
            .expect("columns");
        mapped
            .map(|c| format!("quote(\"{}\")", c.expect("column")))
            .collect()
    };
    let sql = format!(
        "SELECT COALESCE(group_concat({}, '|'), '<empty>') FROM (SELECT * FROM \"{table}\")",
        cols.join(" || ',' || ")
    );
    conn.query_row(&sql, [], |r| r.get(0)).expect("fingerprint")
}

#[test]
fn a_saved_query_stores_a_predicate_and_never_a_membership_list() {
    let (_dir, index) = opened();
    seed_project(index.conn(), 1, "alpha");

    let saved = collections::upsert(index.conn(), &query_args("Rust work", "lang:rust is:dirty"))
        .expect("upsert");
    let row = saved.collection.expect("a collection came back");
    assert_eq!(saved.refused_because, None);
    assert_eq!(row.query_text.as_deref(), Some("lang:rust is:dirty"));

    // §8.8: a query collection materialises nothing.
    let members: i64 = index
        .conn()
        .query_row("SELECT COUNT(*) FROM collection_member", [], |r| r.get(0))
        .expect("count");
    assert_eq!(members, 0, "a saved query owns no membership rows");
    // …and the count is not computed core-side either. Unknown is null, never 0.
    let listed = collections::list(index.conn()).expect("list");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].member_count, None);
}

#[test]
fn a_manual_collection_reports_the_membership_the_core_does_own() {
    let (_dir, index) = opened();
    seed_project(index.conn(), 1, "alpha");
    seed_project(index.conn(), 2, "bravo");
    index
        .conn()
        .execute(
            "INSERT INTO collection (id, name, kind, sort_index) VALUES (5, 'By hand', 'manual', 0)",
            [],
        )
        .expect("seed collection");
    index
        .conn()
        .execute_batch(
            "INSERT INTO collection_member (collection_id, project_id) VALUES (5, 1), (5, 2);",
        )
        .expect("seed members");

    let listed = collections::list(index.conn()).expect("list");
    assert_eq!(listed[0].kind, CollectionKind::Manual);
    assert_eq!(listed[0].member_count, Some(2));
}

#[test]
fn the_five_refusals_of_section_8_8_and_nothing_else() {
    let (_dir, index) = opened();
    let conn = index.conn();
    collections::upsert(conn, &query_args("Rust work", "lang:rust")).expect("first");

    let refused = |name: &str, text: &str| {
        collections::upsert(conn, &query_args(name, text))
            .expect("upsert")
            .refused_because
    };
    assert_eq!(
        refused("   ", "lang:rust"),
        Some(CollectionRefusal::EmptyName)
    );
    // §8.8: the name is the only thing distinguishing two chips, and §1.9's UNIQUE is NOCASE.
    assert_eq!(
        refused("RUST WORK", "lang:go"),
        Some(CollectionRefusal::NameTaken)
    );
    assert_eq!(
        refused(&"x".repeat(49), "lang:go"),
        Some(CollectionRefusal::TooLong)
    );
    assert_eq!(
        refused("two\nlines", "lang:go"),
        Some(CollectionRefusal::TooLong)
    );
    // §8.8: refused, not cycle-checked — a saved query naming another makes deletion break a
    // second collection, and deletion's whole promise is that it breaks nothing.
    assert_eq!(
        refused("Nested", "collection:\"Rust work\""),
        Some(CollectionRefusal::ContainsCollectionTerm)
    );
    // …and the check is the parser's, not a substring's: a quoted value is a value.
    assert_eq!(refused("Quoted", "in:\"a collection: of things\""), None);

    for i in 0..collections::COLLECTION_LIMIT_ROWS {
        let _ = collections::upsert(conn, &query_args(&format!("c{i}"), "lang:go")).expect("fill");
    }
    assert_eq!(
        refused("One too many", "lang:go"),
        Some(CollectionRefusal::LimitReached)
    );
    // A rename of an existing row is not a new row and does not hit the cap.
    let existing = collections::list(conn).expect("list");
    let renamed = collections::upsert(
        conn,
        &UpsertArgs {
            id: Some(existing[0].id),
            name: "Renamed".into(),
            ..query_args("Renamed", "lang:rust")
        },
    )
    .expect("rename");
    assert_eq!(renamed.refused_because, None);
}

#[test]
fn a_stored_grammar_version_comes_back_verbatim_and_is_never_rewritten() {
    let (_dir, index) = opened();
    let conn = index.conn();
    let saved = collections::upsert(conn, &query_args("Rust work", "lang:rust")).expect("upsert");
    let id = saved.collection.expect("row").id;

    // Stand in for a query written under an older grammar than this build speaks.
    conn.execute(
        "UPDATE collection SET query_grammar_version = 0 WHERE id = ?1",
        [id.0],
    )
    .expect("age it");
    // §8.8: bumping it on a successful parse would destroy the only evidence of when the query
    // was written, and it is the input to §8.8's ok/degraded/broken table.
    let edited = collections::upsert(
        conn,
        &UpsertArgs {
            id: Some(id),
            name: "Rust work".into(),
            kind: CollectionKind::Query,
            query_text: Some("lang:rust".into()),
            query_grammar_version: 0,
            sort_index: 3,
        },
    )
    .expect("edit");
    assert_eq!(edited.collection.expect("row").query_grammar_version, 0);
    assert_eq!(
        collections::list(conn).expect("list")[0].query_grammar_version,
        0
    );
}

#[test]
fn a_grammar_version_above_this_build_is_a_protocol_refusal_not_one_of_the_five() {
    let (_dir, index) = opened();
    let ctx = codotheca_core::view::ViewCtx {
        index: &index,
        now: 1_700_000_000,
    };
    let args = serde_json::json!({
        "id": null,
        "name": "From the future",
        "kind": "query",
        "queryText": "lang:rust",
        "queryGrammarVersion": QUERY_GRAMMAR_VERSION + 1,
        "sortIndex": 0
    });
    let err = collections::handle_collection_upsert(&ctx, args).expect_err("must refuse");
    assert_eq!(err.code, codotheca_core::protocol::ErrorCode::Protocol);
    assert_eq!(
        collections::list(index.conn()).expect("list").len(),
        0,
        "and nothing was written"
    );
}

#[test]
fn remove_deletes_the_collection_and_its_members_and_touches_nothing_else() {
    let (_dir, index) = opened();
    let conn = index.conn();
    seed_project(conn, 1, "alpha");
    conn.execute(
        "INSERT INTO collection (id, name, kind, sort_index) VALUES (5, 'By hand', 'manual', 0)",
        [],
    )
    .expect("seed collection");
    conn.execute(
        "INSERT INTO collection_member (collection_id, project_id) VALUES (5, 1)",
        [],
    )
    .expect("seed member");

    // Criterion 52 / §17: phase 1 has no destructive operation. Removing a collection removes a
    // name, not work.
    let before: Vec<String> = ["project", "location", "session", "xp_events"]
        .iter()
        .map(|t| fingerprint(conn, t))
        .collect();

    collections::remove(conn, CollectionId(5)).expect("remove");

    let after: Vec<String> = ["project", "location", "session", "xp_events"]
        .iter()
        .map(|t| fingerprint(conn, t))
        .collect();
    assert_eq!(
        before, after,
        "no project, location, session or xp_events row moved"
    );

    let rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM collection", [], |r| r.get(0))
        .expect("count");
    let members: i64 = conn
        .query_row("SELECT COUNT(*) FROM collection_member", [], |r| r.get(0))
        .expect("count");
    assert_eq!((rows, members), (0, 0));

    // Idempotent: the chip is gone either way, and there is nothing here that could be
    // half-deleted.
    collections::remove(conn, CollectionId(5)).expect("a second remove is a no-op");
}

#[test]
fn list_is_ordered_by_sort_index_so_saved_chips_follow_the_four_built_ins() {
    let (_dir, index) = opened();
    let conn = index.conn();
    for (i, name) in ["third", "first", "second"].iter().enumerate() {
        let mut args = query_args(name, "lang:rust");
        args.sort_index = [2, 0, 1][i];
        let _ = collections::upsert(conn, &args).expect("upsert");
    }
    let names: Vec<String> = collections::list(conn)
        .expect("list")
        .into_iter()
        .map(|c| c.name)
        .collect();
    assert_eq!(names, vec!["first", "second", "third"]);
}

#[test]
fn a_query_collection_that_somehow_gained_a_member_is_refused_by_the_database() {
    // §8.8's guarantee does not rest on this module alone: plan 04's trigger aborts a member
    // written for a query collection, whoever writes it.
    let (_dir, index) = opened();
    let conn = index.conn();
    seed_project(conn, 1, "alpha");
    let saved = collections::upsert(conn, &query_args("Rust work", "lang:rust")).expect("upsert");
    let id = saved.collection.expect("row").id;
    let attempt = conn.execute(
        "INSERT INTO collection_member (collection_id, project_id) VALUES (?1, 1)",
        [id.0],
    );
    assert!(attempt.is_err(), "a query collection owns no members");
}
