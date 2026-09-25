//! §7.4's rename rule. Identity is frozen so recognition holds: `seed_basename` is the directory
//! basename at first index, written once, and the *only* thing that re-seeds it is a rename the
//! user actually performed. A second location with a different basename does not; a remote name
//! learned mid-scan does not — that was v1's defect, and it re-rolled art while the user watched.

use crate::art::store::state_slug;
use crate::art::ArtError;
use crate::index::Index;
use crate::paths::path_from_bytes;
use crate::proto::txguard::TxGuard;
use crate::protocol::ArtState;
use crate::scan::presence::Presence;

/// One of a project's locations, reduced to what the rule reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenameLocation {
    pub location_id: i64,
    pub basename: String,
    pub presence: Presence,
}

/// What a re-seed changed, so a caller can log it and J5 can be believed when it redraws.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reseeded {
    pub project_id: i64,
    pub seed_basename: String,
    /// §7.4: "the walk was a walk over the old basename". Recorded, then discarded.
    pub cleared_offset: u32,
}

/// Ruling 3: the basename comes from the bytes, because `path_display` is lossy and §1.10
/// forbids comparing it. The lossy conversion happens here and only here, at the point the value
/// becomes `seed_basename`, which is a TEXT column with no other representation.
fn basename_of(path_bytes: &[u8]) -> String {
    path_from_bytes(path_bytes)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// §7.4: a rename is one location gone `missing` under the stored basename and exactly one
/// location `present` under a different one. Anything else — two present copies, an offline
/// volume, a location not yet scanned, the same name in a new place — is not a rename.
#[must_use]
pub fn rename_target(seed_basename: &str, locations: &[RenameLocation]) -> Option<String> {
    let mut present = locations
        .iter()
        .filter(|location| location.presence == Presence::Present);
    let candidate = present.next()?;
    if present.next().is_some() {
        return None;
    }
    if candidate.basename == seed_basename || candidate.basename.is_empty() {
        return None;
    }
    let old_is_missing = locations.iter().any(|location| {
        location.presence == Presence::Missing && location.basename == seed_basename
    });
    old_is_missing.then(|| candidate.basename.clone())
}

pub fn location_basenames(
    conn: &rusqlite::Connection,
    project_id: i64,
) -> Result<Vec<RenameLocation>, ArtError> {
    let mut stmt =
        conn.prepare("SELECT id, path_bytes, presence FROM location WHERE project_id = ?1")?;
    let rows = stmt.query_map([project_id], |row| {
        let id: i64 = row.get(0)?;
        let bytes: Vec<u8> = row.get(1)?;
        let presence: String = row.get(2)?;
        Ok((id, bytes, presence))
    })?;

    let mut out = Vec::new();
    for row in rows {
        let (location_id, bytes, presence) = row?;
        // An unparseable presence is not a rename signal; treat it as "not yet decided".
        let presence = Presence::parse(&presence).unwrap_or(Presence::Unscanned);
        out.push(RenameLocation {
            location_id,
            basename: basename_of(&bytes),
            presence,
        });
    }
    Ok(out)
}

/// §7.4: "Reset, then re-render once." `stale` is what makes J5 redraw lazily, shelf-visible
/// first, rather than 400 renders on the launch after a rename storm. Nothing else moves — not
/// `name`, not `lineage_key`, not `remote_key`, not `id`.
pub fn apply_rename(
    tx: &rusqlite::Transaction<'_>,
    project_id: i64,
    basename: &str,
    now: i64,
) -> Result<Reseeded, ArtError> {
    let stored: i64 = tx.query_row(
        "SELECT reroll_offset FROM project WHERE id = ?1",
        [project_id],
        |row| row.get(0),
    )?;
    let stale = state_slug(ArtState::Stale);
    tx.execute(
        "UPDATE project
            SET seed_basename = ?2, reroll_offset = 0, art_state = ?4, updated_at = ?3
          WHERE id = ?1",
        rusqlite::params![project_id, basename, now, stale],
    )?;
    tx.execute(
        "UPDATE art_scene SET state = ?2 WHERE project_id = ?1",
        rusqlite::params![project_id, stale],
    )?;
    Ok(Reseeded {
        project_id,
        seed_basename: basename.to_owned(),
        cleared_offset: u32::try_from(stored).unwrap_or(0),
    })
}

/// The whole rule against the database, for one project. **This is the entry point the scan is
/// missing a call to** — see this task's gap note. It is idempotent: once `seed_basename` matches
/// the present location, `rename_target` answers `None` and nothing is written.
pub fn reseed_after_scan(
    index: &Index,
    project_id: i64,
    now: i64,
) -> Result<Option<Reseeded>, ArtError> {
    let conn = index.conn();
    let seed: String = conn.query_row(
        "SELECT seed_basename FROM project WHERE id = ?1",
        [project_id],
        |row| row.get(0),
    )?;
    let locations = location_basenames(conn, project_id)?;
    let Some(target) = rename_target(&seed, &locations) else {
        return Ok(None);
    };

    let guard = TxGuard::enter();
    let tx = conn.unchecked_transaction()?;
    let out = apply_rename(&tx, project_id, &target, now)?;
    tx.commit()?;
    drop(guard);
    Ok(Some(out))
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
    use crate::protocol::ArtState;
    use crate::scan::presence::Presence;

    fn loc(id: i64, basename: &str, presence: Presence) -> RenameLocation {
        RenameLocation {
            location_id: id,
            basename: basename.to_owned(),
            presence,
        }
    }

    #[test]
    fn the_old_path_missing_and_the_new_one_present_is_the_rename() {
        let locations = [
            loc(1, "alpha-tool", Presence::Missing),
            loc(2, "beta-tool", Presence::Present),
        ];
        assert_eq!(
            rename_target("alpha-tool", &locations),
            Some("beta-tool".to_owned())
        );
    }

    #[test]
    fn a_second_location_with_a_different_basename_never_re_seeds() {
        // §7.4, in as many words. Both are present, so nothing moved.
        let locations = [
            loc(1, "alpha-tool", Presence::Present),
            loc(2, "beta-tool", Presence::Present),
        ];
        assert_eq!(rename_target("alpha-tool", &locations), None);
    }

    #[test]
    fn an_offline_volume_is_not_a_rename() {
        // A removable drive that is simply unplugged must not re-roll the art on the copy that
        // happens to be mounted.
        let locations = [
            loc(1, "alpha-tool", Presence::Offline),
            loc(2, "beta-tool", Presence::Present),
        ];
        assert_eq!(rename_target("alpha-tool", &locations), None);
    }

    #[test]
    fn an_unscanned_location_decides_nothing_yet() {
        let locations = [
            loc(1, "alpha-tool", Presence::Unscanned),
            loc(2, "beta-tool", Presence::Present),
        ];
        assert_eq!(rename_target("alpha-tool", &locations), None);
    }

    #[test]
    fn the_same_basename_in_a_new_place_is_a_move_and_not_a_rename() {
        let locations = [
            loc(1, "alpha-tool", Presence::Missing),
            loc(2, "alpha-tool", Presence::Present),
        ];
        assert_eq!(rename_target("alpha-tool", &locations), None);
    }

    #[test]
    fn a_rename_resets_the_offset_and_stales_the_art_and_moves_nothing_else() {
        let dir = tempfile::tempdir().expect("tempdir");
        let index = Index::open(dir.path()).expect("open");
        index
            .conn()
            .execute(
                "INSERT INTO project (id, name, seed_basename, reroll_offset, art_state,
                                      lineage_key, created_at, updated_at)
                 VALUES (7, 'Alpha Tool', 'alpha-tool', 3, 'ready', 'lin-7', 0, 0)",
                [],
            )
            .expect("insert");

        let tx = index.conn().unchecked_transaction().expect("tx");
        let out = apply_rename(&tx, 7, "beta-tool", 99).expect("rename");
        tx.commit().expect("commit");

        assert_eq!(
            out,
            Reseeded {
                project_id: 7,
                seed_basename: "beta-tool".to_owned(),
                cleared_offset: 3,
            }
        );

        let row: (String, i64, String, String, String) = index
            .conn()
            .query_row(
                "SELECT seed_basename, reroll_offset, art_state, name, lineage_key
                 FROM project WHERE id = 7",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .expect("row");
        // §7.4: "Reset, then re-render once." Identity does not move.
        assert_eq!(row.0, "beta-tool");
        assert_eq!(row.1, 0);
        assert_eq!(row.2, "stale");
        assert_eq!(row.3, "Alpha Tool");
        assert_eq!(row.4, "lin-7");
    }

    #[test]
    fn a_rename_stales_both_art_state_mirrors() {
        let dir = tempfile::tempdir().expect("tempdir");
        let index = Index::open(dir.path()).expect("open");
        index
            .conn()
            .execute(
                "INSERT INTO project (id, name, seed_basename, created_at, updated_at)
                 VALUES (7, 'Alpha Tool', 'alpha-tool', 0, 0)",
                [],
            )
            .expect("insert");

        let scene = generate(&SceneInputs {
            seed_basename: "alpha-tool".to_owned(),
            ..SceneInputs::default()
        });
        let hash = crate::art::scene::scene_hash(&scene).expect("hash");
        crate::art::store::put_scene(index.conn(), 7, &hash, &scene, ArtState::Ready, Some(1))
            .expect("put scene");

        let tx = index.conn().unchecked_transaction().expect("tx");
        apply_rename(&tx, 7, "beta-tool", 99).expect("rename");
        tx.commit().expect("commit");

        let states: (String, String) = index
            .conn()
            .query_row(
                "SELECT project.art_state, art_scene.state
                   FROM project
                   JOIN art_scene ON art_scene.project_id = project.id
                  WHERE project.id = 7",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("states");
        assert_eq!(states.0, "stale");
        assert_eq!(states.1, "stale");
    }

    /// Every NOT NULL column of `location`, so the row the rule reads is the row the schema
    /// actually permits. `path_display` is deliberately wrong here: §1.10 forbids comparing it,
    /// and a reader that used it would answer with this string instead of the bytes.
    fn insert_location(index: &Index, id: i64, dir: &str, basename: &str, presence: Presence) {
        let bytes = format!("/roots/{dir}/{basename}").into_bytes();
        index
            .conn()
            .execute(
                "INSERT INTO location
                     (id, project_id, kind, distro, path_bytes, path_key, path_display,
                      store_key, presence, repo_kind)
                 VALUES (?1, 7, 'linux', '', ?2, ?2, 'not-the-basename', 'store', ?3, 'worktree')",
                rusqlite::params![id, bytes, presence.as_str()],
            )
            .expect("insert location");
    }

    #[test]
    fn the_rule_runs_end_to_end_against_real_location_rows_and_then_stops() {
        let dir = tempfile::tempdir().expect("tempdir");
        let index = Index::open(dir.path()).expect("open");
        index
            .conn()
            .execute(
                "INSERT INTO project (id, name, seed_basename, reroll_offset, created_at,
                                      updated_at)
                 VALUES (7, 'Alpha Tool', 'alpha-tool', 2, 0, 0)",
                [],
            )
            .expect("insert project");
        insert_location(&index, 1, "old-place", "alpha-tool", Presence::Missing);
        insert_location(&index, 2, "new-place", "beta-tool", Presence::Present);

        let out = reseed_after_scan(&index, 7, 99).expect("reseed");
        assert_eq!(
            out,
            Some(Reseeded {
                project_id: 7,
                seed_basename: "beta-tool".to_owned(),
                cleared_offset: 2,
            })
        );

        let row: (String, i64, String) = index
            .conn()
            .query_row(
                "SELECT seed_basename, reroll_offset, art_state FROM project WHERE id = 7",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .expect("row");
        assert_eq!(row, ("beta-tool".to_owned(), 0, "stale".to_owned()));

        // Idempotent: the present location now matches the stored basename, so a second scan
        // finds no rename and writes nothing. Without this the rule would re-seed every scan.
        assert_eq!(reseed_after_scan(&index, 7, 100).expect("second"), None);
        let updated_at: i64 = index
            .conn()
            .query_row("SELECT updated_at FROM project WHERE id = 7", [], |r| {
                r.get(0)
            })
            .expect("updated_at");
        assert_eq!(updated_at, 99, "the second pass did not touch the row");
    }

    #[test]
    fn a_basename_is_read_from_the_bytes_and_never_from_the_display_string() {
        // §1.10: path_display is lossy and is never used to compare. A path whose bytes are not
        // UTF-8 still yields a basename, and it is the bytes that decided it.
        let raw = b"/roots/\xff\xfeodd-name".to_vec();
        let name = basename_of(&raw);
        assert!(name.contains("odd-name"));
    }
}
