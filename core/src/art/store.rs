//! The art tree on disk. Content-addressed, two-level fan-out, atomic writes.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::art::encode::encode_webp;
use crate::art::raster::{render, target_for};
use crate::art::scene::{canonical_json, Scene};
use crate::art::{art_root, is_scene_hash, rendition_path, ArtError, HERO_CACHE_MAX};
use crate::proto::txguard::TxGuard;
use crate::protocol::{ArtState, Rendition};

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

fn io(context: &str, err: &std::io::Error) -> ArtError {
    ArtError::Io(format!("{context}: {err}"))
}

fn write_temp(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;
    let mut file = std::fs::File::create(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

/// §7.2: written atomically — temp then rename. The temp file is a sibling, so the rename never
/// crosses a filesystem, and it carries the pid and a counter so two writers cannot collide.
pub fn write_atomically(path: &Path, bytes: &[u8]) -> Result<(), ArtError> {
    let parent = path
        .parent()
        .ok_or_else(|| ArtError::Io(format!("no parent for {}", path.display())))?;
    std::fs::create_dir_all(parent).map_err(|e| io("create art directory", &e))?;
    let ticket = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let name = path
        .file_name()
        .ok_or_else(|| ArtError::Io(format!("no file name in {}", path.display())))?
        .to_string_lossy()
        .into_owned();
    let temp = parent.join(format!("{name}.tmp-{}-{ticket}", std::process::id()));
    if let Err(err) = write_temp(&temp, bytes) {
        let _ = std::fs::remove_file(&temp);
        return Err(io("write art temporary", &err));
    }
    if let Err(err) = std::fs::rename(&temp, path) {
        let _ = std::fs::remove_file(&temp);
        return Err(io("rename art temporary", &err));
    }
    Ok(())
}

/// Rasterize one rendition of a scene and put it at its content address.
///
/// §23.5: a two-way dispatch over the **same** `Scene` and the same target geometry. The
/// blueprint is a second render *pass*, not a second scene — so `card` and `card-blueprint`
/// share a `scene_hash` and differ only in the file the address names.
pub fn write_rendition(
    data_dir: &Path,
    hash: &str,
    rendition: Rendition,
    scene: &Scene,
) -> Result<PathBuf, ArtError> {
    let path = rendition_path(data_dir, hash, rendition)
        .ok_or_else(|| ArtError::BadHash(hash.to_owned()))?;
    let target = target_for(rendition);
    let pixmap = if crate::art::blueprint::is_blueprint(rendition) {
        crate::art::blueprint::render_blueprint(scene, target)?
    } else {
        render(scene, target)?
    };
    let bytes = encode_webp(&pixmap)?;
    write_atomically(&path, &bytes)?;
    Ok(path)
}

#[must_use]
pub fn rendition_exists(data_dir: &Path, hash: &str, rendition: Rendition) -> bool {
    rendition_path(data_dir, hash, rendition).is_some_and(|p| p.is_file())
}

/// `Ok(false)` when there was nothing to remove. Removing a regenerable cache file is not a §17
/// destructive operation: §7.5 prices it at the milliseconds to redraw from `scene_json`.
pub fn remove_rendition(
    data_dir: &Path,
    hash: &str,
    rendition: Rendition,
) -> Result<bool, ArtError> {
    if !is_scene_hash(hash) {
        return Err(ArtError::BadHash(hash.to_owned()));
    }
    let path = rendition_path(data_dir, hash, rendition)
        .ok_or_else(|| ArtError::BadHash(hash.to_owned()))?;
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(true),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(err) => Err(io("remove rendition", &err)),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtRow {
    pub project_id: i64,
    pub scene_hash: String,
    pub scene_json: String,
    pub schema_version: u32,
    pub rendered_at: Option<i64>,
    /// §7.6: `art_state` and `fail_count` track the **`card`** rendition only. A `hero` nobody
    /// has demanded is not `pending` and its absence is not a failure.
    pub state: ArtState,
    pub fail_count: u32,
}

#[must_use]
pub fn state_slug(state: ArtState) -> &'static str {
    match state {
        ArtState::Pending => "pending",
        ArtState::Ready => "ready",
        ArtState::Failed => "failed",
        ArtState::Stale => "stale",
    }
}

#[must_use]
pub fn state_from_slug(s: &str) -> Option<ArtState> {
    match s {
        "pending" => Some(ArtState::Pending),
        "ready" => Some(ArtState::Ready),
        "failed" => Some(ArtState::Failed),
        "stale" => Some(ArtState::Stale),
        _ => None,
    }
}

pub fn load_row(conn: &rusqlite::Connection, project_id: i64) -> Result<Option<ArtRow>, ArtError> {
    let mut stmt = conn.prepare(
        "SELECT scene_hash, scene_json, schema_version, rendered_at, state, fail_count
           FROM art_scene WHERE project_id = ?1",
    )?;
    let mut rows = stmt.query([project_id])?;
    let Some(row) = rows.next()? else {
        return Ok(None);
    };
    let state_text: String = row.get(4)?;
    let state = state_from_slug(&state_text)
        .ok_or_else(|| ArtError::Encode(format!("unknown art_state {state_text}")))?;
    let schema: i64 = row.get(2)?;
    let fails: i64 = row.get(5)?;
    Ok(Some(ArtRow {
        project_id,
        scene_hash: row.get(0)?,
        scene_json: row.get(1)?,
        schema_version: u32::try_from(schema.max(0)).unwrap_or(0),
        rendered_at: row.get(3)?,
        state,
        fail_count: u32::try_from(fails.max(0)).unwrap_or(0),
    }))
}

/// Which project a content address belongs to. Used by `art.url` to find the document a missing
/// file has to be redrawn from.
pub fn find_project_by_hash(
    conn: &rusqlite::Connection,
    hash: &str,
) -> Result<Option<i64>, ArtError> {
    if !is_scene_hash(hash) {
        return Err(ArtError::BadHash(hash.to_owned()));
    }
    let mut stmt =
        conn.prepare("SELECT project_id FROM art_scene WHERE scene_hash = ?1 LIMIT 1")?;
    let mut rows = stmt.query([hash])?;
    Ok(match rows.next()? {
        Some(row) => Some(row.get(0)?),
        None => None,
    })
}

/// One transaction, two cells. `art_scene` is §1.9's authoritative row; `project.art_scene_hash`
/// and `project.art_state` are §1.2's projection mirror, which §8.3 reads. A writer that moves
/// only one of them is the drift this function exists to prevent.
pub fn put_scene(
    conn: &rusqlite::Connection,
    project_id: i64,
    hash: &str,
    scene: &Scene,
    state: ArtState,
    rendered_at: Option<i64>,
) -> Result<(), ArtError> {
    if !is_scene_hash(hash) {
        return Err(ArtError::BadHash(hash.to_owned()));
    }
    let json =
        String::from_utf8(canonical_json(scene)?).map_err(|e| ArtError::Encode(e.to_string()))?;
    let guard = TxGuard::enter();
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "INSERT INTO art_scene
             (project_id, scene_hash, scene_json, schema_version, rendered_at, state, fail_count)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0)
         ON CONFLICT(project_id) DO UPDATE SET
             scene_hash = excluded.scene_hash,
             scene_json = excluded.scene_json,
             schema_version = excluded.schema_version,
             rendered_at = excluded.rendered_at,
             state = excluded.state,
             fail_count = 0",
        rusqlite::params![
            project_id,
            hash,
            json,
            i64::from(crate::art::ART_SCHEMA_VERSION),
            rendered_at,
            state_slug(state),
        ],
    )?;
    tx.execute(
        "UPDATE project SET art_scene_hash = ?2, art_state = ?3 WHERE id = ?1",
        rusqlite::params![project_id, hash, state_slug(state)],
    )?;
    tx.commit()?;
    drop(guard);
    Ok(())
}

pub fn set_state(
    conn: &rusqlite::Connection,
    project_id: i64,
    state: ArtState,
) -> Result<(), ArtError> {
    let guard = TxGuard::enter();
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "UPDATE art_scene SET state = ?2 WHERE project_id = ?1",
        rusqlite::params![project_id, state_slug(state)],
    )?;
    tx.execute(
        "UPDATE project SET art_state = ?2 WHERE id = ?1",
        rusqlite::params![project_id, state_slug(state)],
    )?;
    tx.commit()?;
    drop(guard);
    Ok(())
}

/// §7.5's `fail_count`, incremented per failed attempt and returned. A success clears it, which
/// `put_scene` does as part of its upsert.
pub fn record_failure(conn: &rusqlite::Connection, project_id: i64) -> Result<u32, ArtError> {
    let guard = TxGuard::enter();
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "UPDATE art_scene SET fail_count = fail_count + 1, state = 'failed' WHERE project_id = ?1",
        [project_id],
    )?;
    tx.execute(
        "UPDATE project SET art_state = 'failed' WHERE id = ?1",
        [project_id],
    )?;
    let count: i64 = tx.query_row(
        "SELECT fail_count FROM art_scene WHERE project_id = ?1",
        [project_id],
        |r| r.get(0),
    )?;
    tx.commit()?;
    drop(guard);
    Ok(u32::try_from(count.max(0)).unwrap_or(0))
}

/// §7.5: a schema bump marks every row `stale`. It does **not** re-render them — that happens
/// lazily, shelf-visible first, when a tile comes into view (Task 15's `on_visible` hook).
pub fn mark_stale_on_schema_bump(
    conn: &rusqlite::Connection,
    schema_version: u32,
) -> Result<usize, ArtError> {
    let version = i64::from(schema_version);
    let guard = TxGuard::enter();
    let tx = conn.unchecked_transaction()?;
    let changed = tx.execute(
        "UPDATE art_scene SET state = 'stale' WHERE schema_version <> ?1 AND state <> 'stale'",
        [version],
    )?;
    tx.execute(
        "UPDATE project SET art_state = 'stale'
          WHERE id IN (SELECT project_id FROM art_scene
                        WHERE schema_version <> ?1 AND state = 'stale')",
        [version],
    )?;
    tx.commit()?;
    drop(guard);
    Ok(changed)
}

/// Ruling 10: the hero LRU is a journal file — one 64-hex hash per line, oldest first.
pub const HERO_LRU_FILE: &str = "hero.lru";

#[must_use]
pub fn hero_lru_path(data_dir: &Path) -> PathBuf {
    art_root(data_dir).join(HERO_LRU_FILE)
}

/// Absent, unreadable or corrupt all read as empty: it is a cache index, and losing it costs a
/// redraw, never a wrong answer. Lines that are not scene hashes are dropped on the way in, so
/// nothing else can ever be used to build a path.
#[must_use]
pub fn read_hero_lru(data_dir: &Path) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(hero_lru_path(data_dir)) else {
        return Vec::new();
    };
    text.lines()
        .map(str::trim)
        .filter(|line| is_scene_hash(line))
        .map(str::to_owned)
        .collect()
}

fn write_hero_lru(data_dir: &Path, entries: &[String]) -> Result<(), ArtError> {
    let mut body = String::with_capacity(entries.len() * 65);
    for entry in entries {
        body.push_str(entry);
        body.push('\n');
    }
    write_atomically(&hero_lru_path(data_dir), body.as_bytes())
}

/// Record an open. Returns the hashes evicted by this open — §7.5's least-recently-opened rule —
/// whose files have already been removed.
pub fn touch_hero(data_dir: &Path, hash: &str) -> Result<Vec<String>, ArtError> {
    if !is_scene_hash(hash) {
        return Err(ArtError::BadHash(hash.to_owned()));
    }
    let mut entries = read_hero_lru(data_dir);
    entries.retain(|entry| entry != hash);
    entries.push(hash.to_owned());
    let mut evicted = Vec::new();
    while entries.len() > HERO_CACHE_MAX {
        if entries.is_empty() {
            break;
        }
        let oldest = entries.remove(0);
        remove_rendition(data_dir, &oldest, Rendition::Hero)?;
        evicted.push(oldest);
    }
    write_hero_lru(data_dir, &entries)?;
    Ok(evicted)
}

/// Drop an entry without touching its file — used when the file has already gone.
pub fn forget_hero(data_dir: &Path, hash: &str) -> Result<(), ArtError> {
    let mut entries = read_hero_lru(data_dir);
    let before = entries.len();
    entries.retain(|entry| entry != hash);
    if entries.len() == before {
        return Ok(());
    }
    write_hero_lru(data_dir, &entries)
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SweepReport {
    pub cards_removed: usize,
    pub heroes_removed: usize,
    /// §23.5's second render pass, both targets. A blueprint is neither a card nor a hero, and
    /// folding it into either counter would make that counter say a number it did not measure.
    pub blueprints_removed: usize,
}

/// §7.5: superseded files are swept when no `art_scene` row references them. Only files this
/// module could have written are considered — a `<64 hex>.<card|hero>.webp` under a two-hex
/// directory. Anything else in the tree is left alone.
pub fn sweep_unreferenced(
    conn: &rusqlite::Connection,
    data_dir: &Path,
) -> Result<SweepReport, ArtError> {
    let mut live = std::collections::BTreeSet::new();
    {
        let mut stmt = conn.prepare("SELECT scene_hash FROM art_scene")?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let hash: String = row.get(0)?;
            if is_scene_hash(&hash) {
                live.insert(hash);
            }
        }
    }
    let root = art_root(data_dir);
    let Ok(fan_dirs) = std::fs::read_dir(&root) else {
        return Ok(SweepReport::default());
    };
    let mut report = SweepReport::default();
    let mut orphaned = Vec::new();
    for fan in fan_dirs.filter_map(Result::ok) {
        if !fan.path().is_dir() {
            continue;
        }
        let Ok(files) = std::fs::read_dir(fan.path()) else {
            continue;
        };
        for entry in files.filter_map(Result::ok) {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some(stem) = name.strip_suffix(".webp") else {
                continue;
            };
            let Some((hash, rendition)) = stem.rsplit_once('.') else {
                continue;
            };
            let Some(rendition) = crate::art::rendition_from_slug(rendition) else {
                continue;
            };
            if !is_scene_hash(hash) || live.contains(hash) {
                continue;
            }
            if std::fs::remove_file(entry.path()).is_err() {
                continue;
            }
            match rendition {
                Rendition::Card => report.cards_removed += 1,
                Rendition::Hero => {
                    report.heroes_removed += 1;
                    orphaned.push(hash.to_owned());
                }
                // Only the `hero` rendition is journalled by `touch_hero`, so a swept blueprint
                // has no LRU entry to forget and must not push one.
                Rendition::CardBlueprint | Rendition::HeroBlueprint => {
                    report.blueprints_removed += 1;
                }
            }
        }
    }
    for hash in orphaned {
        forget_hero(data_dir, &hash)?;
    }
    Ok(report)
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
mod tests {
    use super::*;
    use crate::art::generate::{generate, SceneInputs};
    use crate::protocol::Rendition;

    const H: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn scene() -> crate::art::scene::Scene {
        generate(&SceneInputs {
            seed_basename: "alpha-tool".to_owned(),
            ..SceneInputs::default()
        })
    }

    #[test]
    fn a_rendition_lands_under_the_two_level_fan_out() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = write_rendition(dir.path(), H, Rendition::Card, &scene()).expect("write");
        assert!(path.exists());
        assert_eq!(
            path,
            crate::art::rendition_path(dir.path(), H, Rendition::Card).expect("path")
        );
        assert!(path.to_string_lossy().contains("art"));
        assert!(rendition_exists(dir.path(), H, Rendition::Card));
        assert!(!rendition_exists(dir.path(), H, Rendition::Hero));
    }

    #[test]
    fn the_two_renditions_are_different_files_of_different_sizes() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_rendition(dir.path(), H, Rendition::Card, &scene()).expect("card");
        write_rendition(dir.path(), H, Rendition::Hero, &scene()).expect("hero");
        let card = crate::art::rendition_path(dir.path(), H, Rendition::Card).expect("p");
        let hero = crate::art::rendition_path(dir.path(), H, Rendition::Hero).expect("p");
        assert_ne!(card, hero);
        let card_bytes = std::fs::read(&card).expect("read");
        let hero_bytes = std::fs::read(&hero).expect("read");
        assert_ne!(card_bytes, hero_bytes);
    }

    #[test]
    fn a_rewrite_replaces_the_file_and_leaves_no_temporary_behind() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = write_rendition(dir.path(), H, Rendition::Card, &scene()).expect("first");
        let first = std::fs::read(&path).expect("read");
        write_rendition(dir.path(), H, Rendition::Card, &scene()).expect("second");
        assert_eq!(std::fs::read(&path).expect("read"), first);
        let parent = path.parent().expect("parent");
        let strays: Vec<_> = std::fs::read_dir(parent)
            .expect("dir")
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|name| name.contains(".tmp"))
            .collect();
        assert!(strays.is_empty(), "{strays:?}");
    }

    #[test]
    fn the_bytes_are_only_ever_visible_complete() {
        // §7.2: written atomically, temp + rename. The temp file must not be at the final path.
        let dir = tempfile::tempdir().expect("tempdir");
        let final_path = crate::art::rendition_path(dir.path(), H, Rendition::Card).expect("p");
        std::fs::create_dir_all(final_path.parent().expect("parent")).expect("mkdir");
        write_atomically(&final_path, b"complete").expect("write");
        assert_eq!(std::fs::read(&final_path).expect("read"), b"complete");
        write_atomically(&final_path, b"replaced").expect("write");
        assert_eq!(std::fs::read(&final_path).expect("read"), b"replaced");
    }

    #[test]
    fn a_hash_that_is_not_one_never_reaches_the_filesystem() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(write_rendition(dir.path(), "../escape", Rendition::Card, &scene()).is_err());
        assert!(!rendition_exists(dir.path(), "../escape", Rendition::Card));
        assert!(remove_rendition(dir.path(), "../escape", Rendition::Card).is_err());
    }

    #[test]
    fn removing_a_rendition_that_is_not_there_is_not_an_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(!remove_rendition(dir.path(), H, Rendition::Hero).expect("remove"));
        write_rendition(dir.path(), H, Rendition::Hero, &scene()).expect("write");
        assert!(remove_rendition(dir.path(), H, Rendition::Hero).expect("remove"));
        assert!(!rendition_exists(dir.path(), H, Rendition::Hero));
    }
    fn seeded() -> (tempfile::TempDir, crate::index::Index) {
        let dir = tempfile::tempdir().expect("tempdir");
        let index = crate::index::Index::open(dir.path()).expect("open");
        index
            .conn()
            .execute(
                "INSERT INTO project (id, name, seed_basename, created_at, updated_at)
                 VALUES (7, 'alpha tool', 'alpha-tool', 0, 0)",
                [],
            )
            .expect("insert");
        (dir, index)
    }

    #[test]
    fn a_scene_write_moves_the_row_and_the_mirror_together() {
        let (_dir, index) = seeded();
        let scene = scene();
        let hash = crate::art::scene::scene_hash(&scene).expect("hash");
        put_scene(
            index.conn(),
            7,
            &hash,
            &scene,
            ArtState::Ready,
            Some(1_700_000_000),
        )
        .expect("put");
        let row = load_row(index.conn(), 7).expect("load").expect("row");
        assert_eq!(row.scene_hash, hash);
        assert_eq!(row.state, ArtState::Ready);
        assert_eq!(row.rendered_at, Some(1_700_000_000));
        assert_eq!(row.schema_version, crate::art::ART_SCHEMA_VERSION);
        let (mirror_hash, mirror_state): (Option<String>, String) = index
            .conn()
            .query_row(
                "SELECT art_scene_hash, art_state FROM project WHERE id = 7",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .expect("mirror");
        assert_eq!(mirror_hash.as_deref(), Some(hash.as_str()));
        assert_eq!(mirror_state, "ready");
    }

    #[test]
    fn the_stored_document_is_the_document_that_was_hashed() {
        let (_dir, index) = seeded();
        let scene = scene();
        let hash = crate::art::scene::scene_hash(&scene).expect("hash");
        put_scene(index.conn(), 7, &hash, &scene, ArtState::Ready, Some(1)).expect("put");
        let row = load_row(index.conn(), 7).expect("load").expect("row");
        let parsed: crate::art::scene::Scene =
            serde_json::from_str(&row.scene_json).expect("parse");
        assert_eq!(crate::art::scene::scene_hash(&parsed).expect("hash"), hash);
    }

    #[test]
    fn a_second_write_replaces_rather_than_duplicating() {
        let (_dir, index) = seeded();
        let scene = scene();
        let hash = crate::art::scene::scene_hash(&scene).expect("hash");
        put_scene(index.conn(), 7, &hash, &scene, ArtState::Pending, None).expect("first");
        put_scene(index.conn(), 7, &hash, &scene, ArtState::Ready, Some(9)).expect("second");
        let count: i64 = index
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM art_scene WHERE project_id = 7",
                [],
                |r| r.get(0),
            )
            .expect("count");
        assert_eq!(count, 1);
        assert_eq!(
            load_row(index.conn(), 7).expect("load").expect("row").state,
            ArtState::Ready
        );
    }

    #[test]
    fn a_hash_finds_the_project_that_owns_it() {
        let (_dir, index) = seeded();
        let scene = scene();
        let hash = crate::art::scene::scene_hash(&scene).expect("hash");
        put_scene(index.conn(), 7, &hash, &scene, ArtState::Ready, Some(1)).expect("put");
        assert_eq!(
            find_project_by_hash(index.conn(), &hash).expect("find"),
            Some(7)
        );
        assert_eq!(find_project_by_hash(index.conn(), H).expect("find"), None);
        // A hash that is not one never reaches a query.
        assert!(find_project_by_hash(index.conn(), "../x").is_err());
    }

    #[test]
    fn the_scene_counter_and_the_scheduler_counter_are_different_counters() {
        // Ruling 11: art_scene.fail_count is §7.5's per-scene counter. It is not the retry
        // counter the scheduler keeps in project_job_state.
        let (_dir, index) = seeded();
        let scene = scene();
        let hash = crate::art::scene::scene_hash(&scene).expect("hash");
        put_scene(index.conn(), 7, &hash, &scene, ArtState::Pending, None).expect("put");
        assert_eq!(record_failure(index.conn(), 7).expect("fail"), 1);
        assert_eq!(record_failure(index.conn(), 7).expect("fail"), 2);
        let row = load_row(index.conn(), 7).expect("load").expect("row");
        assert_eq!(row.fail_count, 2);
        assert_eq!(row.state, ArtState::Failed);
        // A success zeroes it.
        put_scene(index.conn(), 7, &hash, &scene, ArtState::Ready, Some(3)).expect("put");
        assert_eq!(
            load_row(index.conn(), 7)
                .expect("load")
                .expect("row")
                .fail_count,
            0
        );
    }

    #[test]
    fn a_schema_bump_stales_every_row_that_is_behind_and_leaves_the_current_ones_alone() {
        // §7.5: "A schema bump marks every row stale and re-renders lazily, shelf-visible
        // first, not 400 at once on the launch after an update."
        let (_dir, index) = seeded();
        index
            .conn()
            .execute(
                "INSERT INTO project (id, name, seed_basename, created_at, updated_at)
                 VALUES (8, 'beta lib', 'beta-lib', 0, 0)",
                [],
            )
            .expect("insert");
        let scene = scene();
        let hash = crate::art::scene::scene_hash(&scene).expect("hash");
        put_scene(index.conn(), 7, &hash, &scene, ArtState::Ready, Some(1)).expect("put");
        put_scene(index.conn(), 8, &hash, &scene, ArtState::Ready, Some(1)).expect("put");
        index
            .conn()
            .execute(
                "UPDATE art_scene SET schema_version = 0 WHERE project_id = 8",
                [],
            )
            .expect("age it");
        let staled =
            mark_stale_on_schema_bump(index.conn(), crate::art::ART_SCHEMA_VERSION).expect("stale");
        assert_eq!(staled, 1);
        assert_eq!(
            load_row(index.conn(), 7).expect("l").expect("r").state,
            ArtState::Ready
        );
        assert_eq!(
            load_row(index.conn(), 8).expect("l").expect("r").state,
            ArtState::Stale
        );
        let mirror: String = index
            .conn()
            .query_row("SELECT art_state FROM project WHERE id = 8", [], |r| {
                r.get(0)
            })
            .expect("mirror");
        assert_eq!(mirror, "stale");
        // Running it twice stales nothing further.
        assert_eq!(
            mark_stale_on_schema_bump(index.conn(), crate::art::ART_SCHEMA_VERSION).expect("s"),
            0
        );
    }

    #[test]
    fn every_state_slug_round_trips_because_it_is_a_stored_value() {
        for state in [
            ArtState::Pending,
            ArtState::Ready,
            ArtState::Failed,
            ArtState::Stale,
        ] {
            assert_eq!(state_from_slug(state_slug(state)), Some(state));
        }
        assert_eq!(state_slug(ArtState::Pending), "pending");
        assert_eq!(state_from_slug("rendered"), None);
    }
    fn hash_n(n: u32) -> String {
        format!("{n:064x}")
    }

    #[test]
    fn the_journal_is_most_recently_opened_last() {
        let dir = tempfile::tempdir().expect("tempdir");
        for n in 0..3 {
            touch_hero(dir.path(), &hash_n(n)).expect("touch");
        }
        assert_eq!(
            read_hero_lru(dir.path()),
            vec![hash_n(0), hash_n(1), hash_n(2)]
        );
        // Re-opening moves it to the end rather than adding a second entry.
        touch_hero(dir.path(), &hash_n(0)).expect("touch");
        assert_eq!(
            read_hero_lru(dir.path()),
            vec![hash_n(1), hash_n(2), hash_n(0)]
        );
    }

    #[test]
    fn the_journal_evicts_the_least_recently_opened_beyond_the_cap() {
        // §7.5: "Heroes are evicted least-recently-opened beyond 200 renditions."
        //
        // DEVIATION: the plan renders a real hero for each of the 200 entries. That is 201
        // rasters and lossless encodes for a test about a text journal, and the eviction path
        // only cares whether a file is there. A one-byte placeholder at the same content
        // address exercises the identical `remove_rendition` call.
        let dir = tempfile::tempdir().expect("tempdir");
        let place = |hash: &str| {
            let path = crate::art::rendition_path(dir.path(), hash, Rendition::Hero).expect("p");
            write_atomically(&path, b"hero").expect("write");
        };
        for n in 0..u32::try_from(crate::art::HERO_CACHE_MAX).expect("cap") {
            let hash = hash_n(n);
            place(&hash);
            assert!(touch_hero(dir.path(), &hash).expect("touch").is_empty());
        }
        assert_eq!(read_hero_lru(dir.path()).len(), crate::art::HERO_CACHE_MAX);
        let overflow = hash_n(9999);
        place(&overflow);
        let evicted = touch_hero(dir.path(), &overflow).expect("touch");
        assert_eq!(evicted, vec![hash_n(0)]);
        assert_eq!(read_hero_lru(dir.path()).len(), crate::art::HERO_CACHE_MAX);
        assert!(!rendition_exists(dir.path(), &hash_n(0), Rendition::Hero));
        assert!(rendition_exists(dir.path(), &hash_n(1), Rendition::Hero));
        assert!(rendition_exists(dir.path(), &overflow, Rendition::Hero));
    }

    #[test]
    fn a_corrupt_or_absent_journal_reads_as_empty_rather_than_failing() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(read_hero_lru(dir.path()).is_empty());
        std::fs::create_dir_all(crate::art::art_root(dir.path())).expect("mkdir");
        std::fs::write(hero_lru_path(dir.path()), "not a hash\n\n../escape\n").expect("write");
        assert!(
            read_hero_lru(dir.path()).is_empty(),
            "only scene hashes survive the read"
        );
    }

    #[test]
    fn the_sweep_removes_files_no_scene_row_references() {
        // §7.5: "Superseded files are swept when no art_scene row references them."
        let (dir, index) = seeded();
        let scene = scene();
        let live = crate::art::scene::scene_hash(&scene).expect("hash");
        put_scene(index.conn(), 7, &live, &scene, ArtState::Ready, Some(1)).expect("put");
        write_rendition(dir.path(), &live, Rendition::Card, &scene).expect("card");
        write_rendition(dir.path(), &live, Rendition::Hero, &scene).expect("hero");
        touch_hero(dir.path(), &live).expect("touch");
        let dead = hash_n(1234);
        write_rendition(dir.path(), &dead, Rendition::Card, &scene).expect("card");
        write_rendition(dir.path(), &dead, Rendition::Hero, &scene).expect("hero");
        touch_hero(dir.path(), &dead).expect("touch");

        let report = sweep_unreferenced(index.conn(), dir.path()).expect("sweep");
        assert_eq!(report.cards_removed, 1);
        assert_eq!(report.heroes_removed, 1);
        assert!(rendition_exists(dir.path(), &live, Rendition::Card));
        assert!(rendition_exists(dir.path(), &live, Rendition::Hero));
        assert!(!rendition_exists(dir.path(), &dead, Rendition::Card));
        assert!(!rendition_exists(dir.path(), &dead, Rendition::Hero));
        // The journal loses the dead entry with the file.
        assert_eq!(read_hero_lru(dir.path()), vec![live]);
    }

    #[test]
    fn the_sweep_leaves_anything_it_does_not_recognise_alone() {
        // A stray file in the art tree is not ours to delete — §17's posture applies to
        // everything except the regenerable renditions this module wrote.
        let (dir, index) = seeded();
        let root = crate::art::art_root(dir.path()).join("zz");
        std::fs::create_dir_all(&root).expect("mkdir");
        let stray = root.join("notes.txt");
        std::fs::write(&stray, b"hands off").expect("write");
        let report = sweep_unreferenced(index.conn(), dir.path()).expect("sweep");
        assert_eq!(report.cards_removed, 0);
        assert!(stray.exists());
    }

    #[cfg(feature = "testkit")]
    #[test]
    fn startup_stales_the_behind_rows_and_sweeps_in_one_pass() {
        let (dir, index) = seeded();
        let scene = scene();
        let live = crate::art::scene::scene_hash(&scene).expect("hash");
        put_scene(index.conn(), 7, &live, &scene, ArtState::Ready, Some(1)).expect("put");
        index
            .conn()
            .execute(
                "UPDATE art_scene SET schema_version = 0 WHERE project_id = 7",
                [],
            )
            .expect("age");
        write_rendition(dir.path(), &hash_n(4242), Rendition::Card, &scene).expect("write");
        let events = crate::art::testsupport::CollectingSink::default();
        let ctx = crate::art::ArtCtx {
            index: &index,
            events: &events,
            now: 1_700_000_000,
        };
        let report = crate::art::startup(&ctx).expect("startup");
        assert_eq!(report.staled, 1);
        assert_eq!(report.swept.cards_removed, 1);
    }
}
