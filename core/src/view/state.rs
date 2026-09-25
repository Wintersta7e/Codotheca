//! §8's shelf view state, in §1.9's `view_state(k, v)` table.
//!
//! §1.12 counts view state among the content **no rescan can re-derive**, so it is written on
//! demand and restored verbatim. `saved_at` is the field that makes a fresh install
//! distinguishable from a client that saved an empty view: *never render unknown as zero*, on the
//! wire and not only in the DOM.

use crate::index::IndexError;
use crate::proto::dispatch::{parse_args, CommandFailure};
use crate::proto::txguard::TxGuard;
use crate::protocol::{ProjectId, SortKey, ViewMode, ViewPatch, ViewSetArgs, ViewState};
use crate::view::ViewCtx;

pub const KEY_QUERY: &str = "shelf.query";
pub const KEY_SORT: &str = "shelf.sort";
pub const KEY_VIEW_MODE: &str = "shelf.view_mode";
pub const KEY_DENSITY: &str = "shelf.density";
pub const KEY_COLLAPSED_SECTIONS: &str = "shelf.collapsed_sections";
pub const KEY_SCROLL_OFFSET: &str = "shelf.scroll_offset";
pub const KEY_SELECTED_PROJECT: &str = "shelf.selected_project_id";
pub const KEY_WINDOW_GEOMETRY: &str = "window.geometry";
pub const KEY_SAVED_AT: &str = "saved_at";

/// §8.0's convention, restated by §1.9: **one row per dismissed notice**, `notice.dismissed.<id>`,
/// unscoped. A single JSON array would be a second spelling of keys other sections read directly.
pub const NOTICE_DISMISSED_PREFIX: &str = "notice.dismissed.";

/// §8's `DENSITY` control: the tile width in px, default `186` (`--tile-min`).
pub const DEFAULT_DENSITY: u32 = 186;

/// Not a `const`: `ViewState` carries two `Vec`s.
#[must_use]
pub fn view_state_defaults() -> ViewState {
    ViewState {
        query: String::new(),
        sort: SortKey::LastTouched,
        view_mode: ViewMode::Grid,
        density: DEFAULT_DENSITY,
        collapsed_sections: Vec::new(),
        scroll_offset: 0.0,
        selected_project_id: None,
        dismissed_notices: Vec::new(),
        window_geometry: None,
        // `None` is *never saved*. A client that saved an empty view gets a timestamp, and the
        // two are different facts.
        saved_at: None,
    }
}

fn get(conn: &rusqlite::Connection, key: &str) -> Result<Option<String>, IndexError> {
    let mut stmt = conn.prepare("SELECT v FROM view_state WHERE k = ?1")?;
    let mut rows = stmt.query([key])?;
    Ok(match rows.next()? {
        Some(row) => Some(row.get(0)?),
        None => None,
    })
}

fn put(conn: &rusqlite::Connection, key: &str, value: &str) -> Result<(), IndexError> {
    conn.execute(
        "INSERT INTO view_state (k, v) VALUES (?1, ?2)
         ON CONFLICT(k) DO UPDATE SET v = excluded.v",
        rusqlite::params![key, value],
    )?;
    Ok(())
}

/// A stored value a rolled-back build wrote must not take the shelf down with it.
fn from_json<T: serde::de::DeserializeOwned>(raw: Option<String>, fallback: T) -> T {
    raw.and_then(|v| serde_json::from_str::<T>(&v).ok())
        .unwrap_or(fallback)
}

fn from_enum<T: serde::de::DeserializeOwned>(raw: Option<String>, fallback: T) -> T {
    raw.and_then(|v| serde_json::from_value::<T>(serde_json::Value::String(v)).ok())
        .unwrap_or(fallback)
}

fn to_json<T: serde::Serialize>(value: &T) -> Result<String, IndexError> {
    serde_json::to_string(value)
        .map_err(|e| IndexError::Sidecar(format!("view_state value did not serialise: {e}")))
}

fn to_enum<T: serde::Serialize>(value: &T) -> Result<String, IndexError> {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(s)) => Ok(s),
        _ => Err(IndexError::Sidecar(
            "enum did not serialise as a string".into(),
        )),
    }
}

fn dismissed(conn: &rusqlite::Connection) -> Result<Vec<String>, IndexError> {
    let mut stmt =
        conn.prepare("SELECT k FROM view_state WHERE k LIKE 'notice.dismissed.%' ORDER BY k")?;
    let mut rows = stmt.query([])?;
    let mut out = Vec::new();
    while let Some(row) = rows.next()? {
        let key: String = row.get(0)?;
        if let Some(id) = key.strip_prefix(NOTICE_DISMISSED_PREFIX) {
            out.push(id.to_owned());
        }
    }
    Ok(out)
}

/// Never claim currency you do not have: a stored selection whose `project` row is gone — merged
/// away, or lost to a rebuild — is not a selection. Following §1.6's `project_redirect` is plan
/// 08's business, not this module's.
fn selection(
    conn: &rusqlite::Connection,
    stored: Option<ProjectId>,
) -> Result<Option<ProjectId>, IndexError> {
    let Some(id) = stored else { return Ok(None) };
    let alive: i64 = conn.query_row("SELECT COUNT(*) FROM project WHERE id = ?1", [id.0], |r| {
        r.get(0)
    })?;
    Ok((alive == 1).then_some(id))
}

/// # Errors
/// `Sqlite` for anything the index refuses. A stored value this build cannot read is **not** an
/// error: it falls back to the default, so a rolled-back build's leftovers cannot take the shelf
/// down with them.
pub fn load(conn: &rusqlite::Connection) -> Result<ViewState, IndexError> {
    let d = view_state_defaults();
    let stored_selection = from_json(get(conn, KEY_SELECTED_PROJECT)?, d.selected_project_id);
    Ok(ViewState {
        query: get(conn, KEY_QUERY)?.unwrap_or(d.query),
        sort: from_enum(get(conn, KEY_SORT)?, d.sort),
        view_mode: from_enum(get(conn, KEY_VIEW_MODE)?, d.view_mode),
        density: from_json(get(conn, KEY_DENSITY)?, d.density),
        collapsed_sections: from_json(get(conn, KEY_COLLAPSED_SECTIONS)?, d.collapsed_sections),
        scroll_offset: from_json(get(conn, KEY_SCROLL_OFFSET)?, d.scroll_offset),
        selected_project_id: selection(conn, stored_selection)?,
        dismissed_notices: dismissed(conn)?,
        window_geometry: from_json(get(conn, KEY_WINDOW_GEOMETRY)?, d.window_geometry),
        saved_at: from_json(get(conn, KEY_SAVED_AT)?, d.saved_at),
    })
}

/// §2: every `ViewPatch` field is nullable and `null` means **leave unchanged**. There are no
/// optional properties on this wire, so a field's absence is not expressible and is not a state.
///
/// One consequence worth knowing: a selection cannot be *cleared* through a patch, because
/// `null` already means "leave alone". `load` drops a selection whose project is gone, which is
/// the case that actually arises.
///
/// # Errors
/// `Sqlite` for anything the index refuses; `Sidecar` for a value that will not serialise.
pub fn store(conn: &rusqlite::Connection, patch: &ViewPatch, now: i64) -> Result<(), IndexError> {
    let _guard = TxGuard::enter();
    let tx = conn.unchecked_transaction()?;
    if let Some(v) = patch.query.as_deref() {
        put(&tx, KEY_QUERY, v)?;
    }
    if let Some(v) = patch.sort {
        put(&tx, KEY_SORT, &to_enum(&v)?)?;
    }
    if let Some(v) = patch.view_mode {
        put(&tx, KEY_VIEW_MODE, &to_enum(&v)?)?;
    }
    if let Some(v) = patch.density {
        put(&tx, KEY_DENSITY, &v.to_string())?;
    }
    if let Some(v) = patch.collapsed_sections.as_ref() {
        put(&tx, KEY_COLLAPSED_SECTIONS, &to_json(v)?)?;
    }
    if let Some(v) = patch.scroll_offset {
        put(&tx, KEY_SCROLL_OFFSET, &to_json(&v)?)?;
    }
    if let Some(v) = patch.selected_project_id {
        put(&tx, KEY_SELECTED_PROJECT, &v.0.to_string())?;
    }
    if let Some(v) = patch.dismissed_notices.as_ref() {
        tx.execute(
            "DELETE FROM view_state WHERE k LIKE 'notice.dismissed.%'",
            [],
        )?;
        for id in v {
            put(&tx, &format!("{NOTICE_DISMISSED_PREFIX}{id}"), "1")?;
        }
    }
    if let Some(v) = patch.window_geometry.as_ref() {
        put(&tx, KEY_WINDOW_GEOMETRY, &to_json(v)?)?;
    }
    // Stamped by the core, never by the client: it is provenance, and a caller cannot forge it.
    put(&tx, KEY_SAVED_AT, &now.to_string())?;
    tx.commit()?;
    Ok(())
}

/// # Errors
/// `INTERNAL` for an index fault.
pub fn handle_view_get(ctx: &ViewCtx<'_>) -> Result<ViewState, CommandFailure> {
    load(ctx.index.conn()).map_err(|e| CommandFailure::internal(e.to_string()))
}

/// # Errors
/// `PROTOCOL` for an argument shape the schema does not admit; `INTERNAL` for an index fault.
pub fn handle_view_set(
    ctx: &ViewCtx<'_>,
    args: serde_json::Value,
) -> Result<serde_json::Value, CommandFailure> {
    // R15: plan 03's helper, not a second copy. R31: `ViewSetArgs` is the schema's, not a
    // hand-written `SetArgs` beside it.
    let args: ViewSetArgs = parse_args(args)?;
    store(ctx.index.conn(), &args.patch, ctx.now)
        .map_err(|e| CommandFailure::internal(e.to_string()))?;
    Ok(serde_json::json!({}))
}
