//! The one place a `ProjectRow` is built. §5.1's multi-location rules land here, and so does
//! every NULL that must survive the trip to the wire.
//!
//! **Three statements, not N+1**: one over `location`, one over `collection_member`, one over
//! `project` with the two `EXISTS`-shaped facts folded in. The base predicate —
//! `merged_into IS NULL`, ordered `last_touched_at DESC` — is the fixed shape
//! `idx_project_shelf_order` exists for; the *query* is not pushed into SQL, and
//! `crate::query::execute` says why.

use std::collections::BTreeMap;

use crate::art::compose::local_year;
use crate::index::path::{display_paths_for_ui, DisplayPathTable};
use crate::projects::{ProjectsCtx, ProjectsError};
use crate::protocol::{
    ArtState, CollectionId, ConditionSignal, ErrorCode, InterruptedOp, LocationId, LocationKind,
    LocationRef, Presence, ProjectId, ProjectRow, SceneHash,
};

/// Reads a stored TEXT enum through the **generated** serde renames.
///
/// There is deliberately no second table here. R31's failure mode is a hand-written vocabulary
/// beside the schema's, and R26's is a stored slug that has drifted from the emitted one; going
/// through serde means the only vocabulary in this file is the one `protocol/schema/protocol.json`
/// generated. `None` is a word this build does not know — a row from a newer schema — which the
/// caller turns into `BadColumn` rather than guessing at.
#[must_use]
pub fn enum_from_column<T: serde::de::DeserializeOwned>(raw: &str) -> Option<T> {
    serde_json::from_value(serde_json::Value::String(raw.to_owned())).ok()
}

fn column<T: serde::de::DeserializeOwned>(
    raw: &str,
    name: &'static str,
) -> Result<T, ProjectsError> {
    enum_from_column(raw).ok_or_else(|| ProjectsError::BadColumn {
        column: name,
        value: raw.to_owned(),
    })
}

fn optional_column<T: serde::de::DeserializeOwned>(
    raw: Option<String>,
    name: &'static str,
) -> Result<Option<T>, ProjectsError> {
    match raw {
        None => Ok(None),
        Some(text) => column(&text, name).map(Some),
    }
}

#[derive(Debug, Clone)]
pub struct LocationFacts {
    pub id: LocationId,
    pub project_id: ProjectId,
    pub kind: LocationKind,
    pub distro: String,
    pub path_display: String,
    pub presence: Presence,
    pub branch: Option<String>,
    pub is_dirty: Option<bool>,
    pub untracked_count: Option<i64>,
    pub ahead: Option<i64>,
    pub behind: Option<i64>,
    pub stash_count: Option<i64>,
    pub interrupted_op: Option<InterruptedOp>,
    /// §8.5.2's per-copy HEAD. The shelf row does not draw it; `LocationDetail` does, and one
    /// location mapping serving both is the point of this struct.
    pub head_oid: Option<String>,
    /// §11.1's TRUST THIS REPOSITORY, scoped to the exact path `safe.directory` takes.
    pub trusted_at: Option<i64>,
    pub fetch_head_at: Option<i64>,
    pub refstate_observed_at: Option<i64>,
    pub worktree_observed_at: Option<i64>,
    /// §5.1's "most recently touched". J3 writes it; `pick_primary` reads it.
    pub worktree_newest_mtime: Option<i64>,
    pub last_seen_at: Option<i64>,
}

/// What §8.3's grammar filters on and the wire row does not carry yet — the core-side view of
/// the renderer's `ProjectRowExtras`. Every field is `Option` for **not available**, never a
/// default: `has:ci` with no column behind it is an ignored term, not a `false`.
#[derive(Debug, Clone, Default)]
pub struct RowFacts {
    pub authored_by_user: Option<bool>,
    pub location_kind: Option<LocationKind>,
    pub distro: Option<String>,
    pub has_remote: bool,
    pub has_submodules: bool,
    /// `Some` only once J6 has run for the project; `peek_cache.computed_at` is what says so.
    pub has_readme: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct LoadedRow {
    pub row: ProjectRow,
    pub facts: RowFacts,
}

/// §5.1: *most recently touched, present, native side preferred*.
///
/// **One rule, and `core::derive` already had it.** `aggregate` in `core/src/derive/mod.rs`
/// orders present copies by `worktree_newest_mtime`, then native, then the lowest id; this
/// orders by the same keys with presence as the outermost one rather than a filter, because a
/// project whose every copy is offline must still name a copy on the wire (§11.1) where
/// `aggregate` correctly answers `None`. `core/tests/projects_rows.rs` asserts the two agree.
///
/// R41 recorded `last_seen_at` here on the premise that `location` has no touched column;
/// migration `0007_jobs_derived.sql` added `worktree_newest_mtime`, so the premise is stale and
/// a presence clock is no longer the best answer available.
#[must_use]
pub fn pick_primary(locations: &[LocationFacts]) -> Option<&LocationFacts> {
    locations.iter().max_by_key(|l| {
        (
            i32::from(l.presence == Presence::Present),
            l.worktree_newest_mtime.unwrap_or(i64::MIN),
            i32::from(l.kind.is_native()),
            -l.id.0,
        )
    })
}

/// §5.1's OR, three-valued. `None` means nobody has looked — **never `false`** (§6).
#[must_use]
pub fn any_present_dirty(locations: &[LocationFacts]) -> Option<bool> {
    let mut observed = false;
    for loc in locations.iter().filter(|l| l.presence == Presence::Present) {
        match loc.is_dirty {
            Some(true) => return Some(true),
            Some(false) => observed = true,
            None => {}
        }
    }
    observed.then_some(false)
}

fn max_present<T: Ord + Copy>(
    locations: &[LocationFacts],
    f: impl Fn(&LocationFacts) -> Option<T>,
) -> Option<T> {
    locations
        .iter()
        .filter(|l| l.presence == Presence::Present)
        .filter_map(f)
        .max()
}

/// The generation §8.2's response carries. `0` before the first scan — a run counter, not a
/// measurement, so a zero here claims nothing.
pub fn scan_generation(conn: &rusqlite::Connection) -> Result<i64, ProjectsError> {
    Ok(conn.query_row(
        "SELECT COALESCE(MAX(generation), 0) FROM scan_run",
        [],
        |r| r.get(0),
    )?)
}

/// §1.10: `path_display` is **not** here. It is write-once and the one function permitted to
/// read it back is `index::path::display_paths_for_ui`; `fill_display_paths` below calls it.
/// `volume_key` and `store_key` are deliberately absent: a column that is never selected cannot
/// leak into a payload, and §11.1's offline row must name no drive (criterion 63).
const LOCATION_COLUMNS: &str = "SELECT id, project_id, kind, distro, presence, branch,
                is_dirty, untracked_count, ahead, behind, stash_count, interrupted_op,
                fetch_head_at, refstate_observed_at, worktree_observed_at, worktree_newest_mtime,
                last_seen_at, head_oid, trusted_at
           FROM location";

fn map_location(r: &rusqlite::Row<'_>) -> Result<LocationFacts, ProjectsError> {
    let kind_raw: String = r.get(2)?;
    let presence_raw: String = r.get(4)?;
    Ok(LocationFacts {
        id: LocationId(r.get(0)?),
        project_id: ProjectId(r.get(1)?),
        kind: column(&kind_raw, "location.kind")?,
        distro: r.get(3)?,
        // Filled by `fill_display_paths`, which is the only route §1.10 allows.
        path_display: String::new(),
        presence: column(&presence_raw, "location.presence")?,
        branch: r.get(5)?,
        is_dirty: r.get::<_, Option<i64>>(6)?.map(|v| v != 0),
        untracked_count: r.get(7)?,
        ahead: r.get(8)?,
        behind: r.get(9)?,
        stash_count: r.get(10)?,
        interrupted_op: optional_column(r.get(11)?, "location.interrupted_op")?,
        fetch_head_at: r.get(12)?,
        refstate_observed_at: r.get(13)?,
        worktree_observed_at: r.get(14)?,
        worktree_newest_mtime: r.get(15)?,
        last_seen_at: r.get(16)?,
        head_oid: r.get(17)?,
        trusted_at: r.get(18)?,
    })
}

/// §1.10's one permitted reader, called once for the whole set rather than per row.
///
/// The column is write-once and `core/tests/index_paths.rs` scans the source to keep every
/// other statement off it — anything that opens, launches or compares a path takes
/// `path_bytes`, and a `LocationRef` is the only shape a path leaves the core in (§2.5).
fn fill_display_paths(
    conn: &rusqlite::Connection,
    locations: &mut [LocationFacts],
) -> Result<(), ProjectsError> {
    let ids: Vec<i64> = locations.iter().map(|l| l.id.0).collect();
    let displays: BTreeMap<i64, String> =
        display_paths_for_ui(conn, DisplayPathTable::Location, &ids)?
            .into_iter()
            .collect();
    for loc in locations.iter_mut() {
        if let Some(text) = displays.get(&loc.id.0) {
            loc.path_display.clone_from(text);
        }
    }
    Ok(())
}

/// Every copy of one project, in the same mapping `load_project_rows` uses for all of them.
/// One mapping, two callers — `projects.peek` needs exactly this for a single project.
pub fn locations_of(
    conn: &rusqlite::Connection,
    project: ProjectId,
) -> Result<Vec<LocationFacts>, ProjectsError> {
    let mut out = Vec::new();
    {
        let mut stmt = conn.prepare(&format!("{LOCATION_COLUMNS} WHERE project_id = ?1"))?;
        let mut rows = stmt.query(rusqlite::params![project.0])?;
        while let Some(r) = rows.next()? {
            out.push(map_location(r)?);
        }
    }
    fill_display_paths(conn, &mut out)?;
    Ok(out)
}

fn count_u32(value: Option<i64>) -> Option<u32> {
    value.and_then(|v| u32::try_from(v).ok())
}

/// Statement one: every `location` row, grouped by the project that owns it.
fn locations_by_project(
    conn: &rusqlite::Connection,
) -> Result<BTreeMap<i64, Vec<LocationFacts>>, ProjectsError> {
    let mut all: Vec<LocationFacts> = Vec::new();
    {
        let mut stmt = conn.prepare(LOCATION_COLUMNS)?;
        let mut rows = stmt.query([])?;
        while let Some(r) = rows.next()? {
            all.push(map_location(r)?);
        }
    }
    fill_display_paths(conn, &mut all)?;

    let mut by_project: BTreeMap<i64, Vec<LocationFacts>> = BTreeMap::new();
    for facts in all {
        by_project
            .entry(facts.project_id.0)
            .or_default()
            .push(facts);
    }
    Ok(by_project)
}

/// Statement two: collection membership, so the row carries `collection:` without an N+1.
fn collections_by_project(
    conn: &rusqlite::Connection,
) -> Result<BTreeMap<i64, Vec<CollectionId>>, ProjectsError> {
    let mut collections: BTreeMap<i64, Vec<CollectionId>> = BTreeMap::new();
    let mut stmt =
        conn.prepare("SELECT project_id, collection_id FROM collection_member ORDER BY 1, 2")?;
    let mut rows = stmt.query([])?;
    while let Some(r) = rows.next()? {
        collections
            .entry(r.get(0)?)
            .or_default()
            .push(CollectionId(r.get(1)?));
    }
    Ok(collections)
}

const PROJECT_COLUMNS: &str =
    "SELECT p.id, p.name, p.owner, p.description, p.description_source, p.primary_language,
            p.archetype, p.art_scene_hash, p.art_state, p.condition_signal, p.completion_lit,
            p.completion_applicable, p.is_pinned, p.is_archived, p.is_hidden, p.is_reference,
            p.is_fork, p.is_bare, p.is_shallow, p.submodule_path, p.ambiguous_lineage,
            p.last_touched_at, p.last_interaction_at, p.last_commit_at, p.last_commit_subject,
            p.first_commit_at, p.first_commit_tz_offset_min, p.created_at, p.acknowledged_at,
            p.size_tracked_bytes, p.tracked_files, p.error_kind, p.error_at,
            p.authored_by_user, p.remote_key,
            EXISTS(SELECT 1 FROM submodule_edge se WHERE se.parent_project_id = p.id),
            pc.computed_at, pc.readme_excerpt,
            p.seed_basename, p.reroll_offset
       FROM project p
       LEFT JOIN peek_cache pc ON pc.project_id = p.id";

/// The shelf's fixed shape, which `idx_project_shelf_order` exists for.
const PROJECT_SHELF_FILTER: &str = " WHERE p.merged_into IS NULL
      ORDER BY p.last_touched_at DESC, p.id ASC";

/// One `project` row plus its already-loaded copies and collections, folded into the wire row.
fn map_loaded_row(
    r: &rusqlite::Row<'_>,
    locations: &[LocationFacts],
    collection_ids: Vec<CollectionId>,
) -> Result<LoadedRow, ProjectsError> {
    let id: i64 = r.get(0)?;
    let primary = pick_primary(locations);
    let art_state_raw: String = r.get(8)?;
    let created_at: i64 = r.get(27)?;
    // `last_touched_at` is NULLable (§1.2's DDL) and `ProjectRow.lastTouchedAt` is not.
    // The fallback is `created_at` — when the app first saw it — and NEVER epoch 0, which
    // would file a brand-new repository under `era:tail` as the oldest section there is.
    let last_touched_at: i64 = r.get::<_, Option<i64>>(21)?.unwrap_or(created_at);
    let first_commit_at: Option<i64> = r.get(25)?;
    let first_commit_tz: Option<i64> = r.get(26)?;
    let readme_computed_at: Option<i64> = r.get(36)?;
    let readme_excerpt: Option<String> = r.get(37)?;

    let row = ProjectRow {
        id: ProjectId(id),
        name: r.get(1)?,
        owner: r.get(2)?,
        description: r.get(3)?,
        description_source: optional_column(r.get(4)?, "project.description_source")?,
        birth_year: first_commit_at.and_then(|at| {
            let offset = i32::try_from(first_commit_tz.unwrap_or(0)).unwrap_or(0);
            u32::try_from(local_year(at, offset)).ok()
        }),
        primary_language: r.get(5)?,
        archetype: r.get(6)?,
        art_scene_hash: r.get::<_, Option<String>>(7)?.map(SceneHash),
        art_state: column::<ArtState>(&art_state_raw, "project.art_state")?,
        condition_signal: optional_column::<ConditionSignal>(
            r.get(9)?,
            "project.condition_signal",
        )?,
        // §7.7a: nothing in phase 1 writes these. They cross as null, and every consumer
        // downstream must render "not computed" rather than "0 of 10".
        completion_lit: count_u32(r.get(10)?),
        completion_applicable: count_u32(r.get(11)?),
        is_pinned: r.get::<_, i64>(12)? != 0,
        is_archived: r.get::<_, i64>(13)? != 0,
        is_hidden: r.get::<_, i64>(14)? != 0,
        is_reference: r.get::<_, i64>(15)? != 0,
        is_fork: r.get::<_, i64>(16)? != 0,
        is_bare: r.get::<_, i64>(17)? != 0,
        is_shallow: r.get::<_, i64>(18)? != 0,
        is_submodule: r.get::<_, Option<String>>(19)?.is_some(),
        ambiguous_lineage: r.get::<_, i64>(20)? != 0,
        last_touched_at,
        last_interaction_at: r.get(22)?,
        last_commit_at: r.get(23)?,
        last_commit_subject: r.get(24)?,
        first_commit_at,
        created_at,
        acknowledged_at: r.get(28)?,
        size_tracked_bytes: r.get(29)?,
        tracked_files: r.get(30)?,
        collection_ids,
        primary_location: primary.map(|l| LocationRef {
            id: l.id,
            path_display: l.path_display.clone(),
        }),
        // §23.2: presence is a property of a *location*, so a project with none has none.
        // `Unscanned` here was a false claim with a wired control behind it — §8.5.2 draws
        // `NOT SCANNED` and offers `ENABLE ROOT` on the covering root, and a not-cloned project
        // is covered by no root, so that control named a root that cannot exist.
        presence: primary.map(|l| l.presence),
        // §5.1: the tile's branch and divergence come from the primary copy, not the fold.
        branch: primary.and_then(|l| l.branch.clone()),
        is_dirty: any_present_dirty(locations),
        untracked_count: count_u32(primary.and_then(|l| l.untracked_count)),
        ahead: count_u32(primary.and_then(|l| l.ahead)),
        behind: count_u32(primary.and_then(|l| l.behind)),
        stash_count: count_u32(primary.and_then(|l| l.stash_count)),
        interrupted_op: primary.and_then(|l| l.interrupted_op),
        fetch_head_at: primary.and_then(|l| l.fetch_head_at),
        refstate_observed_at: max_present(locations, |l| l.refstate_observed_at),
        worktree_observed_at: max_present(locations, |l| l.worktree_observed_at),
        error_kind: optional_column::<ErrorCode>(r.get(31)?, "project.error_kind")?,
        error_at: r.get(32)?,
        // §7.3a derives the jewel from these two, so they cross as stored and are never
        // defaulted here: a basename the renderer did not seed from produces a different
        // colour than the art the core rasterized.
        seed_basename: r.get(38)?,
        reroll_offset: u32::try_from(r.get::<_, i64>(39)?).unwrap_or(0),
        // Stamped by `projects::list`, which owns the band rules. Empty here so a caller
        // that forgets to section cannot pass an id off as one this loader computed.
        era_section_id: String::new(),
    };

    let facts = RowFacts {
        authored_by_user: r.get::<_, Option<i64>>(33)?.map(|v| v != 0),
        location_kind: primary.map(|l| l.kind),
        distro: primary.map(|l| l.distro.clone()),
        has_remote: r.get::<_, Option<String>>(34)?.is_some(),
        has_submodules: r.get::<_, i64>(35)? != 0,
        has_readme: readme_computed_at.map(|_| readme_excerpt.is_some()),
    };
    Ok(LoadedRow { row, facts })
}

/// Statement three, and the fold. Three statements over the whole library, never N+1: the
/// per-project work is a `BTreeMap` lookup, not a query.
pub fn load_project_rows(ctx: &ProjectsCtx<'_>) -> Result<Vec<LoadedRow>, ProjectsError> {
    let conn = ctx.index.conn();
    let mut by_project = locations_by_project(conn)?;
    let mut collections = collections_by_project(conn)?;

    let mut stmt = conn.prepare(&format!("{PROJECT_COLUMNS}{PROJECT_SHELF_FILTER}"))?;
    let mut out: Vec<LoadedRow> = Vec::new();
    let mut rows = stmt.query([])?;
    while let Some(r) = rows.next()? {
        let id: i64 = r.get(0)?;
        let locations = by_project.remove(&id).unwrap_or_default();
        let collection_ids = collections.remove(&id).unwrap_or_default();
        out.push(map_loaded_row(r, &locations, collection_ids)?);
    }
    Ok(out)
}

/// One project through the **same** projection the shelf uses.
///
/// The opened page and the tile must not be able to disagree about one repository, which is
/// R12's rule applied to a row rather than a formatter: `projects.get` builds its hero from this
/// and never from a second `SELECT`. `merged_into` is not filtered here because the caller has
/// already followed §1.6's redirect and holds the survivor's id.
///
/// # Errors
/// `UnknownProject` when the id names no row; `Sqlite` for anything the index refuses.
pub fn load_project_row(
    conn: &rusqlite::Connection,
    project: ProjectId,
) -> Result<LoadedRow, ProjectsError> {
    let mut locations = locations_of(conn, project)?;
    locations.sort_by_key(|l| l.id.0);

    let mut collection_ids: Vec<CollectionId> = Vec::new();
    {
        let mut stmt = conn.prepare(
            "SELECT collection_id FROM collection_member WHERE project_id = ?1 ORDER BY 1",
        )?;
        let mut rows = stmt.query(rusqlite::params![project.0])?;
        while let Some(r) = rows.next()? {
            collection_ids.push(CollectionId(r.get(0)?));
        }
    }

    let mut stmt = conn.prepare(&format!("{PROJECT_COLUMNS} WHERE p.id = ?1"))?;
    let mut rows = stmt.query(rusqlite::params![project.0])?;
    let Some(r) = rows.next()? else {
        return Err(ProjectsError::UnknownProject(project.0));
    };
    map_loaded_row(r, &locations, collection_ids)
}
