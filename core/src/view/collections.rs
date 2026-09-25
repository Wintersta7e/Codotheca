//! §8.8's collections.
//!
//! **A collection is a saved query, recomputed.** `kind='query'` stores a predicate and
//! materialises nothing: it owns no `collection_member` rows, this module writes none for it, and
//! plan 04's `collection_member_rejects_query_collection` trigger aborts any handed to it anyway.
//! A stored list would go stale silently the first time a scan landed, and §1.12 already counts
//! collections among the content no rescan can re-derive.
//!
//! **`member_count` is NULL for every `kind='query'` row, and that is the invariant.** The core
//! has no executor to evaluate a saved predicate against, so `0` would be *unknown rendered as
//! zero* on the wire and a stored number would be the stale membership list §8.8 forbids. For
//! `kind='manual'` the count is a fact the core does own and is returned.
//!
//! **Removal is not destructive.** It deletes one `collection` row and that row's
//! `collection_member` rows and writes no `project`, `location`, `session` or `xp_events` row and
//! no byte on disk (§8.8, §17).

use crate::index::IndexError;
use crate::proto::dispatch::{parse_args, CommandFailure};
use crate::proto::txguard::TxGuard;
use crate::protocol::{
    Collection, CollectionId, CollectionKind, CollectionRefusal, CollectionUpsert,
    CollectionsRemoveArgs, CollectionsUpsertArgs,
};
use crate::query::ast::{QueryTerm, TextField};
use crate::query::{parse_query, QUERY_GRAMMAR_VERSION};
use crate::view::ViewCtx;

/// §8.8's longest collection name, in `char`s of the trimmed name.
///
/// §8.8's two numbers. The renderer states them for its control text; the core is the **writer**,
/// so it enforces them again and has the last word. The renderer measures the name in UTF-16 code
/// units and this measures `char`s — they can disagree only on astral characters, and a name the
/// field allowed and the core refuses comes back as `refusedBecause: too_long`, which the UI
/// already renders.
pub const COLLECTION_NAME_MAX_CHARS: usize = 48;
/// §8.8's most collections: an insert at this many `collection` rows is refused
/// `limit_reached`; a rename never is.
pub const COLLECTION_LIMIT_ROWS: usize = 16;

/// The `collections.upsert` argument struct, under the name plan 15 gave it.
///
/// **R31**: the argument struct is the schema's. Plan 15 named a hand-written `UpsertArgs` with
/// the same six fields; a second declaration beside the generated one is how a wire type and its
/// twin drift, so this is the generated one under the plan's name.
pub type UpsertArgs = CollectionsUpsertArgs;

fn row(r: &rusqlite::Row<'_>) -> Result<Collection, rusqlite::Error> {
    let kind: String = r.get(2)?;
    Ok(Collection {
        id: CollectionId(r.get(0)?),
        name: r.get(1)?,
        kind: if kind == "manual" {
            CollectionKind::Manual
        } else {
            CollectionKind::Query
        },
        query_text: r.get(3)?,
        // A manual row's column is NULL by §1.9's CHECK and nothing reads this field for it —
        // the renderer branches on `kind` first. Named as a wart for plan 02, whose schema makes
        // the field non-nullable, rather than papered over here.
        query_grammar_version: r.get::<_, Option<u32>>(4)?.unwrap_or(0),
        sort_index: r.get(5)?,
        member_count: r.get(6)?,
    })
}

/// §8.8: saved chips are appended after the four built-ins in `sort_index` order.
///
/// # Errors
/// `Sqlite` for anything the index refuses.
pub fn list(conn: &rusqlite::Connection) -> Result<Vec<Collection>, IndexError> {
    let mut stmt = conn.prepare(
        "SELECT c.id, c.name, c.kind, c.query_text, c.query_grammar_version, c.sort_index,
                CASE WHEN c.kind = 'manual'
                     THEN (SELECT COUNT(*) FROM collection_member m WHERE m.collection_id = c.id)
                     ELSE NULL END
           FROM collection c
          ORDER BY c.sort_index, c.id",
    )?;
    let rows = stmt.query_map([], row)?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

/// §8.8: `collection:` inside a **stored** query is refused rather than cycle-checked — a saved
/// query naming another makes its meaning depend on a name deletion can take away, and deletion's
/// whole promise is that it breaks nothing. Asked of plan 13's parser (**R13**), never of the raw
/// string: `in:"a collection: of things"` is a quoted value, not a term.
fn mentions_a_collection(query_text: &str) -> bool {
    parse_query(query_text).terms.iter().any(|t| {
        matches!(
            t,
            QueryTerm::Text {
                field: TextField::Collection,
                ..
            }
        )
    })
}

fn name_taken(
    conn: &rusqlite::Connection,
    id: Option<CollectionId>,
    name: &str,
) -> Result<bool, IndexError> {
    let existing = id.map_or(-1, |c| c.0);
    let hits: i64 = conn.query_row(
        "SELECT COUNT(*) FROM collection WHERE name = ?1 COLLATE NOCASE AND id <> ?2",
        rusqlite::params![name, existing],
        |r| r.get(0),
    )?;
    Ok(hits > 0)
}

/// §8.8's five refusals, in the order its table states them. `None` means admissible.
///
/// # Errors
/// `Sqlite` for anything the index refuses.
pub fn refuse(
    conn: &rusqlite::Connection,
    id: Option<CollectionId>,
    name: &str,
    query_text: Option<&str>,
) -> Result<Option<CollectionRefusal>, IndexError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Ok(Some(CollectionRefusal::EmptyName));
    }
    if name_taken(conn, id, trimmed)? {
        return Ok(Some(CollectionRefusal::NameTaken));
    }
    if trimmed.chars().count() > COLLECTION_NAME_MAX_CHARS || name.contains(['\r', '\n']) {
        return Ok(Some(CollectionRefusal::TooLong));
    }
    if query_text.is_some_and(mentions_a_collection) {
        return Ok(Some(CollectionRefusal::ContainsCollectionTerm));
    }
    // Only an insert can hit the cap: a rename of an existing row is not a new row.
    if id.is_none() {
        let rows: i64 = conn.query_row("SELECT COUNT(*) FROM collection", [], |r| r.get(0))?;
        if usize::try_from(rows).unwrap_or(usize::MAX) >= COLLECTION_LIMIT_ROWS {
            return Ok(Some(CollectionRefusal::LimitReached));
        }
    }
    Ok(None)
}

const fn to_kind(kind: CollectionKind) -> &'static str {
    match kind {
        CollectionKind::Manual => "manual",
        CollectionKind::Query => "query",
    }
}

/// # Errors
/// `Sqlite` for anything the index refuses. A refusal is **not** an error: §8.8 needs to know
/// *which* refusal fired, because each has its own control text, and a bare error would lose it.
pub fn upsert(
    conn: &rusqlite::Connection,
    args: &UpsertArgs,
) -> Result<CollectionUpsert, IndexError> {
    let name = args.name.trim().to_owned();
    let is_query = args.kind == CollectionKind::Query;
    let query_text = if is_query {
        args.query_text.clone()
    } else {
        None
    };
    if let Some(refusal) = refuse(conn, args.id, &args.name, query_text.as_deref())? {
        return Ok(CollectionUpsert {
            collection: None,
            refused_because: Some(refusal),
        });
    }

    // §1.9's CHECK: a manual row holds neither a query nor a grammar version.
    let version: Option<u32> = is_query.then_some(args.query_grammar_version);
    let _guard = TxGuard::enter();
    let tx = conn.unchecked_transaction()?;
    let id = if let Some(id) = args.id {
        tx.execute(
            "UPDATE collection
                SET name = ?2, kind = ?3, query_text = ?4,
                    query_grammar_version = ?5, sort_index = ?6
              WHERE id = ?1",
            rusqlite::params![
                id.0,
                name,
                to_kind(args.kind),
                query_text,
                version,
                args.sort_index
            ],
        )?;
        id
    } else {
        tx.execute(
            "INSERT INTO collection (name, kind, query_text, query_grammar_version, sort_index)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![
                name,
                to_kind(args.kind),
                query_text,
                version,
                args.sort_index
            ],
        )?;
        CollectionId(tx.last_insert_rowid())
    };
    tx.commit()?;

    let saved = list(conn)?.into_iter().find(|c| c.id == id);
    Ok(CollectionUpsert {
        collection: saved,
        refused_because: None,
    })
}

/// §8.8 and §17: this removes a name, not work.
///
/// The member delete is explicit rather than left to `ON DELETE CASCADE` so the guarantee does
/// not depend on a pragma being on. Removing an id that is not there is a no-op: the chip is gone
/// either way, and there is nothing here that could be half-deleted.
///
/// # Errors
/// `Sqlite` for anything the index refuses.
pub fn remove(conn: &rusqlite::Connection, id: CollectionId) -> Result<(), IndexError> {
    let _guard = TxGuard::enter();
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "DELETE FROM collection_member WHERE collection_id = ?1",
        [id.0],
    )?;
    tx.execute("DELETE FROM collection WHERE id = ?1", [id.0])?;
    tx.commit()?;
    Ok(())
}

/// # Errors
/// `INTERNAL` for an index fault.
pub fn handle_collection_list(ctx: &ViewCtx<'_>) -> Result<Vec<Collection>, CommandFailure> {
    list(ctx.index.conn()).map_err(|e| CommandFailure::internal(e.to_string()))
}

/// # Errors
/// `PROTOCOL` for an argument shape the schema does not admit, or a `queryGrammarVersion` above
/// this build's; `INTERNAL` for an index fault.
pub fn handle_collection_upsert(
    ctx: &ViewCtx<'_>,
    args: serde_json::Value,
) -> Result<CollectionUpsert, CommandFailure> {
    let args: UpsertArgs = parse_args(args)?;
    // §8.8: the stored version records what the text was **written against** and is never
    // silently rewritten — bumping it on a successful parse destroys the only evidence of when
    // the query was written, and it is the input to §8.8's ok/degraded/broken table. A version
    // above this build's is a query the core cannot parse and so cannot honestly store: a
    // protocol fault, not one of the five user-facing refusals.
    if args.kind == CollectionKind::Query && args.query_grammar_version > QUERY_GRAMMAR_VERSION {
        return Err(CommandFailure::protocol(
            "queryGrammarVersion is above this build's",
        ));
    }
    upsert(ctx.index.conn(), &args).map_err(|e| CommandFailure::internal(e.to_string()))
}

/// # Errors
/// `PROTOCOL` for an argument shape the schema does not admit; `INTERNAL` for an index fault.
pub fn handle_collection_remove(
    ctx: &ViewCtx<'_>,
    args: serde_json::Value,
) -> Result<serde_json::Value, CommandFailure> {
    let args: CollectionsRemoveArgs = parse_args(args)?;
    remove(ctx.index.conn(), args.id).map_err(|e| CommandFailure::internal(e.to_string()))?;
    Ok(serde_json::json!({}))
}
