//! The merge procedure (§1.5). One transaction, and every table has a rule.

use rusqlite::{params, OptionalExtension as _, Transaction};

use super::IdentityError;

/// The rule line §1.5 puts between two concatenated notes.
pub const NOTE_SEPARATOR: &str = "\n\n---\n\n";

/// The absorbed row exactly as it stood before the merge. Stored as `merge_record.absorbed_json`
/// (§1.9) so a phase-4 split has something to restore.
///
/// Four bools, deliberately: they are §1.2's four organisation flags, each with its own
/// reconciliation rule in §1.5. Collapsing them into a bitfield would hide which rule applies.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AbsorbedSnapshot {
    pub name: String,
    pub description: Option<String>,
    pub description_source: Option<String>,
    pub seed_basename: String,
    pub reroll_offset: i64,
    pub is_pinned: bool,
    pub is_archived: bool,
    pub is_hidden: bool,
    pub is_reference: bool,
    /// Byte offset in the survivor's merged note at which the absorbed text begins. `None` when
    /// the absorbed row had no note.
    pub notes_offset: Option<i64>,
}

/// §1.5: the project with the earliest `created_at`, ties on the lowest `id`. Deterministic and
/// walk-order independent, which is the whole point — the same two projects must merge the same
/// way whichever the scanner reached first.
pub fn choose_survivor(tx: &Transaction<'_>, a: i64, b: i64) -> Result<(i64, i64), IdentityError> {
    if a == b {
        return Err(IdentityError::SameProject(a));
    }
    let created = |id: i64| -> Result<i64, IdentityError> {
        tx.query_row(
            "SELECT created_at FROM project WHERE id = ?1",
            params![id],
            |r| r.get(0),
        )
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => IdentityError::UnknownProject(id),
            other => IdentityError::Sqlite(other),
        })
    };
    let (ca, cb) = (created(a)?, created(b)?);
    if (ca, a) <= (cb, b) {
        Ok((a, b))
    } else {
        Ok((b, a))
    }
}

/// §1.5's scalar rows: notes, the four flags, and the survivor-wins fields. Returns the absorbed
/// row as it stood before anything was written.
///
/// `name`, `description`, `description_source`, `seed_basename` and `reroll_offset` are **not**
/// in the `UPDATE`: §1.5 says the survivor's values stand, always, and the absorbed ones are
/// kept in `merge_record`. Not writing them is how that rule is enforced.
pub fn reconcile_scalars(
    tx: &Transaction<'_>,
    survivor: i64,
    absorbed: i64,
    now: i64,
) -> Result<AbsorbedSnapshot, IdentityError> {
    let s = read_row(tx, survivor)?;
    let a = read_row(tx, absorbed)?;

    // Notes are concatenated, survivor first, and never discarded. A rule line with nothing
    // above it is not a rule line, so an empty survivor note produces no separator.
    let s_note = s.notes.unwrap_or_default();
    let a_note = a.notes.unwrap_or_default();
    let (merged_notes, notes_offset): (Option<String>, Option<i64>) =
        match (s_note.is_empty(), a_note.is_empty()) {
            (true, true) => (None, None),
            (false, true) => (Some(s_note), None),
            (true, false) => (Some(a_note), Some(0)),
            (false, false) => {
                let offset = s_note.len().saturating_add(NOTE_SEPARATOR.len());
                let joined = format!("{s_note}{NOTE_SEPARATOR}{a_note}");
                (Some(joined), i64::try_from(offset).ok())
            }
        };

    tx.execute(
        "UPDATE project
            SET notes = ?2,
                is_pinned = ?3, is_archived = ?4, is_hidden = ?5, is_reference = ?6,
                updated_at = ?7
          WHERE id = ?1",
        params![
            survivor,
            merged_notes,
            i64::from(s.is_pinned || a.is_pinned),
            i64::from(s.is_archived || a.is_archived),
            i64::from(s.is_hidden && a.is_hidden),
            i64::from(s.is_reference && a.is_reference),
            now,
        ],
    )?;

    Ok(AbsorbedSnapshot {
        name: a.name,
        description: a.description,
        description_source: a.description_source,
        seed_basename: a.seed_basename,
        reroll_offset: a.reroll_offset,
        is_pinned: a.is_pinned,
        is_archived: a.is_archived,
        is_hidden: a.is_hidden,
        is_reference: a.is_reference,
        notes_offset,
    })
}

/// How many rows of each not-derivable table moved. Four of these go into
/// `merge_record.absorbed_json` (§1.9); the rest are what `projects.unmergeHint` reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ReparentCounts {
    pub location: usize,
    pub session: usize,
    pub collection_member: usize,
    pub launch_target: usize,
    /// Of `launch_target`, how many lost a `(kind, name)` collision and were kept disabled.
    pub launch_target_disabled: usize,
    pub health_delta: usize,
    pub submodule_edge: usize,
}

/// §1.5's reparenting rows. Nothing here is recomputed, because none of it is derivable from
/// history; and nothing here is deleted, because deleting a user-authored row is a destructive
/// operation and §17 admits none in phase 1.
///
/// `session_segment` names its `session`, not its project, so it follows without being touched —
/// which is why it needs no statement of its own beyond this one.
pub fn reparent_rows(
    tx: &Transaction<'_>,
    survivor: i64,
    absorbed: i64,
) -> Result<ReparentCounts, IdentityError> {
    let location = tx.execute(
        "UPDATE location SET project_id = ?1 WHERE project_id = ?2",
        params![survivor, absorbed],
    )?;
    let session = tx.execute(
        "UPDATE session SET project_id = ?1 WHERE project_id = ?2",
        params![survivor, absorbed],
    )?;
    let health_delta = tx.execute(
        "UPDATE health_delta SET project_id = ?1 WHERE project_id = ?2",
        params![survivor, absorbed],
    )?;

    // Union, deduplicated. `WHERE NOT EXISTS` rather than `INSERT OR IGNORE`, so a membership
    // the survivor already holds is left alone instead of being replaced — and a count is what
    // §8.8's chip renders, so a second row for one collection would double it.
    let collection_member = tx.execute(
        "INSERT INTO collection_member (collection_id, project_id)
         SELECT m.collection_id, ?1 FROM collection_member m
          WHERE m.project_id = ?2
            AND NOT EXISTS (SELECT 1 FROM collection_member x
                             WHERE x.collection_id = m.collection_id AND x.project_id = ?1)",
        params![survivor, absorbed],
    )?;
    tx.execute(
        "DELETE FROM collection_member WHERE project_id = ?1",
        params![absorbed],
    )?;

    // Union; on a (kind, name) collision the survivor's row wins and the other is kept
    // disabled. Kept, not deleted: it is the only record that the copy had its own target.
    let launch_target_disabled = tx.execute(
        "UPDATE launch_target SET disabled = 1
          WHERE project_id = ?2
            AND EXISTS (SELECT 1 FROM launch_target s
                         WHERE s.project_id = ?1 AND s.kind = launch_target.kind
                           AND s.name = launch_target.name)",
        params![survivor, absorbed],
    )?;
    let launch_target = tx.execute(
        "UPDATE launch_target SET project_id = ?1 WHERE project_id = ?2",
        params![survivor, absorbed],
    )?;

    // A submodule edge names two projects and either may be the absorbed one (§4.4).
    let submodule_edge = tx.execute(
        "UPDATE submodule_edge SET parent_project_id = ?1 WHERE parent_project_id = ?2",
        params![survivor, absorbed],
    )?;
    tx.execute(
        "UPDATE submodule_edge SET child_project_id = ?1 WHERE child_project_id = ?2",
        params![survivor, absorbed],
    )?;
    tx.execute(
        "DELETE FROM submodule_edge WHERE parent_project_id = child_project_id",
        [],
    )?;

    // An earlier merge whose survivor was this row must keep naming a live project, or §1.5's
    // "the absorbed id keeps resolving" guarantee ends one hop short.
    tx.execute(
        "UPDATE merge_record SET survivor_project_id = ?1 WHERE survivor_project_id = ?2",
        params![survivor, absorbed],
    )?;

    // The parent link is not one of §1.5's survivor-wins fields, and losing it would orphan a
    // submodule from its superproject. The survivor's stands; an absorbed link fills a gap.
    tx.execute(
        "UPDATE project
            SET parent_project_id = (SELECT a.parent_project_id FROM project a WHERE a.id = ?2),
                submodule_path     = (SELECT a.submodule_path     FROM project a WHERE a.id = ?2)
          WHERE id = ?1 AND parent_project_id IS NULL
            AND (SELECT a.parent_project_id FROM project a WHERE a.id = ?2) IS NOT NULL",
        params![survivor, absorbed],
    )?;

    Ok(ReparentCounts {
        location,
        session,
        collection_member,
        launch_target,
        launch_target_disabled,
        health_delta,
        submodule_edge,
    })
}

/// §1.7's git-derived event kinds — a pure function of history, so never migrated.
///
/// This is the vocabulary, not the predicate. The sweep below keys on `xp_events.track`, which
/// `0003`'s `CHECK ((track = 'session') = (kind IN ('session', 'focus')))` already ties to these
/// five: restating them in the `DELETE` would be one value in two places, and the copy that
/// went stale would leave a sixth kind's rows behind as stale duplicates after a merge.
pub const GIT_DERIVED_XP_KINDS: [&str; 5] = [
    "commit_day",
    "release",
    "language_first",
    "revival",
    "first_push",
];

/// What the derived half of a merge did. `swept_scene_hash` is the absorbed row's art scene: the
/// row is gone, and the art layer sweeps its files (§1.5, §7.5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DerivedSweep {
    pub git_events_deleted: usize,
    pub xp_events_reparented: usize,
    /// `app_meta.level_floor` as it now stands.
    pub level_floor: i64,
    pub swept_scene_hash: Option<String>,
}

/// Delete everything that recomputes, reparent everything that does not.
///
/// **`level_floor` is stamped before the delete, or it is unrecoverable** (§1.4's ruling, which
/// §1.7 extends to every deletion of git-derived events). Phase 1 renders no level and has no
/// level function, so the floor is stored in the only unit phase 1 can compute honestly: the
/// library-wide count of git-derived events immediately before the delete, `max(existing,
/// counted)`, monotonic. Any level function monotonic in that count inherits the guarantee.
pub fn recompute_derived(
    tx: &Transaction<'_>,
    survivor: i64,
    absorbed: i64,
) -> Result<DerivedSweep, IdentityError> {
    let before: i64 = tx.query_row(
        "SELECT COUNT(*) FROM xp_events WHERE track = 'git'",
        [],
        |r| r.get(0),
    )?;
    let level_floor = stamp_level_floor(tx, before)?;

    let git_events_deleted = tx.execute(
        "DELETE FROM xp_events WHERE project_id IN (?1, ?2) AND track = 'git'",
        params![survivor, absorbed],
    )?;
    let xp_events_reparented = tx.execute(
        "UPDATE xp_events SET project_id = ?1 WHERE project_id = ?2",
        params![survivor, absorbed],
    )?;

    // Recomputed from the survivor by the history and content jobs. Deleting both sides is
    // required, not tidy: a stale row for the survivor would survive the merge and be read as
    // current.
    //
    // **[p3] §29.10: `blob_scan` and `blob_finding` are deliberately NOT here, and that is a
    // guard rather than an omission.** They carry no project id, their key is a **content
    // address**, and deleting a library-wide cache because two project rows merged discards work
    // for an event that cannot have invalidated it — a blob's content is not a property of any
    // project. `project_content_scan` is here because it is per project and entirely derived.
    // **[p3] §28.7:  and  are DERIVED.** Both are recomputed from the
    // survivor by the producers §28 owns, so deleting both sides is required rather than tidy —
    // a stale item for the survivor would survive the merge and be read as current, and a stale
    //  sweep would let the next observation close items it never compared against.
    //
    // **A merge is not a closure.** Deleting both sides leaves no → pair
    // across the merge, so the recompute opens the survivor's items fresh, no closure event
    // fires and no XP is paid. That falls out of this class assignment rather than needing a
    // guard of its own.
    for table in [
        "fts_commits",
        "peek_cache",
        "project_committer",
        "project_content_scan",
        "debt_item",
        "debt_sweep",
    ] {
        tx.execute(
            &format!("DELETE FROM {table} WHERE project_id IN (?1, ?2)"),
            params![survivor, absorbed],
        )?;
    }
    // The absorbed row is no longer a scheduling subject; the survivor's state is the caller's
    // to requeue.
    tx.execute(
        "DELETE FROM project_job_state WHERE project_id = ?1",
        params![absorbed],
    )?;

    let swept_scene_hash: Option<String> = tx
        .query_row(
            "SELECT scene_hash FROM art_scene WHERE project_id = ?1",
            params![absorbed],
            |r| r.get(0),
        )
        .optional()?;
    tx.execute(
        "DELETE FROM art_scene WHERE project_id = ?1",
        params![absorbed],
    )?;

    Ok(DerivedSweep {
        git_events_deleted,
        xp_events_reparented,
        level_floor,
        swept_scene_hash,
    })
}

fn stamp_level_floor(tx: &Transaction<'_>, counted: i64) -> Result<i64, IdentityError> {
    let existing: Option<i64> = tx
        .query_row(
            "SELECT CAST(v AS INTEGER) FROM app_meta WHERE k = 'level_floor'",
            [],
            |r| r.get(0),
        )
        .optional()?;
    match existing {
        Some(current) if current >= counted => Ok(current),
        Some(_) => {
            tx.execute(
                "UPDATE app_meta SET v = ?1 WHERE k = 'level_floor'",
                params![counted.to_string()],
            )?;
            Ok(counted)
        }
        None => {
            tx.execute(
                "INSERT INTO app_meta (k, v) VALUES ('level_floor', ?1)",
                params![counted.to_string()],
            )?;
            Ok(counted)
        }
    }
}

/// The result of §1.5's one transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeOutcome {
    pub survivor: i64,
    pub absorbed: i64,
    /// The survivor's `association_kind` after combination.
    pub association: super::AssociationKind,
    pub reparented: ReparentCounts,
    pub derived: DerivedSweep,
    pub merge_record_id: i64,
    /// Always true. Git-derived rows were deleted, and recomputing them means walking history —
    /// which §1.10 forbids inside a transaction, so the caller requeues the history job.
    pub needs_history_recompute: bool,
}

/// §1.5, whole. The caller owns the transaction, so this is one transaction by construction;
/// it must not be given a connection that autocommits per statement.
///
/// `a` and `b` may be stale ids: both resolve through `project_redirect` first (§1.6). Neither
/// argument picks the survivor — [`choose_survivor`] does, so the answer does not depend on
/// which way round the caller named them.
pub fn merge_projects(
    tx: &Transaction<'_>,
    a: i64,
    b: i64,
    kind: super::AssociationKind,
    evidence: &serde_json::Value,
    now: i64,
) -> Result<MergeOutcome, IdentityError> {
    let a = super::redirect::resolve_project_id(tx, a)?;
    let b = super::redirect::resolve_project_id(tx, b)?;
    let (survivor, absorbed) = choose_survivor(tx, a, b)?;

    let snapshot = reconcile_scalars(tx, survivor, absorbed, now)?;
    let reparented = reparent_rows(tx, survivor, absorbed)?;
    let derived = recompute_derived(tx, survivor, absorbed)?;

    let stored: Option<String> = tx
        .query_row(
            "SELECT association_kind FROM project WHERE id = ?1",
            params![survivor],
            |r| r.get(0),
        )
        .optional()?
        .flatten();
    let association = match stored.as_deref().and_then(super::AssociationKind::parse) {
        Some(previous) => previous.combine(kind),
        None => kind,
    };
    tx.execute(
        "UPDATE project SET association_kind = ?2, updated_at = ?3 WHERE id = ?1",
        params![survivor, association.as_str(), now],
    )?;

    // Tombstoned, never deleted.
    tx.execute(
        "UPDATE project SET merged_into = ?2, updated_at = ?3 WHERE id = ?1",
        params![absorbed, survivor, now],
    )?;
    super::redirect::write_redirect(tx, absorbed, survivor, now)?;

    let absorbed_json = serde_json::json!({
        "name": snapshot.name,
        "description": snapshot.description,
        "description_source": snapshot.description_source,
        "seed_basename": snapshot.seed_basename,
        "reroll_offset": snapshot.reroll_offset,
        "flags": {
            "is_pinned": i64::from(snapshot.is_pinned),
            "is_archived": i64::from(snapshot.is_archived),
            "is_hidden": i64::from(snapshot.is_hidden),
            "is_reference": i64::from(snapshot.is_reference),
        },
        "notes_offset": snapshot.notes_offset,
        "reparented": {
            "location": reparented.location,
            "session": reparented.session,
            "collection_member": reparented.collection_member,
            "xp_events": derived.xp_events_reparented,
            "launch_target": reparented.launch_target,
            "launch_target_disabled": reparented.launch_target_disabled,
            "submodule_edge": reparented.submodule_edge,
        },
        "swept_scene_hash": derived.swept_scene_hash,
    });
    tx.execute(
        "INSERT INTO merge_record (survivor_project_id, absorbed_project_id, merged_at,
                                   association_kind, evidence_json, absorbed_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            survivor,
            absorbed,
            now,
            association.as_str(),
            evidence.to_string(),
            absorbed_json.to_string(),
        ],
    )?;
    let merge_record_id = tx.last_insert_rowid();

    Ok(MergeOutcome {
        survivor,
        absorbed,
        association,
        reparented,
        derived,
        merge_record_id,
        needs_history_recompute: true,
    })
}

/// What a split of one absorbed row would have to decide. Every field is a report; none is a
/// control (§1.5, §8.5.2).
///
/// **Not the wire type of the same name**, and not a duplicate of it either: `crate::protocol`'s
/// carries `flagsOrred`/`flagsAnded` as the three-variant `Flag`, drops `is_reference` — which
/// is computed, not user-set — and has no `merge_record_id` or `evidence_json`. The projection
/// between them is a total struct literal, so a schema change fails to compile here rather than
/// drifting (R31's caution about name collisions: compare shapes, not names).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnmergeHint {
    pub merge_record_id: i64,
    pub absorbed_project_id: i64,
    pub merged_at: i64,
    pub association_kind: super::AssociationKind,
    /// Verbatim from `merge_record`, so the renderer names the evidence that made the
    /// association rather than re-deriving it against a candidate set that has since moved.
    pub evidence_json: String,
    pub notes_were_concatenated: bool,
    pub locations: i64,
    pub sessions: i64,
    pub collection_members: i64,
    /// Session-derived rows only. Git-derived rows recompute (§1.7) and a split does not have
    /// to divide them.
    pub xp_events: i64,
    /// Flags whose absorbed value was overwritten by the OR — the deliberate act won.
    pub flags_lost_by_or: Vec<String>,
    /// Flags whose absorbed value was overwritten by the AND.
    pub flags_lost_by_and: Vec<String>,
}

/// One `merge_record` row as SQLite hands it over. Aliased because a six-field tuple trips
/// `clippy::type_complexity`, which this crate denies.
type MergeRecordRow = (i64, i64, i64, String, String, String);

/// Read-only. Performs no split, writes nothing, takes no confirmation.
pub fn unmerge_hint(
    tx: &Transaction<'_>,
    project_id: i64,
) -> Result<Vec<UnmergeHint>, IdentityError> {
    let survivor = super::redirect::resolve_project_id(tx, project_id)?;
    // `[is_pinned, is_archived, is_hidden, is_reference]`, in that order — the two OR'd flags
    // first, then the two AND'd ones, which is what the index arithmetic below reads.
    let current = tx.query_row(
        "SELECT is_pinned, is_archived, is_hidden, is_reference FROM project WHERE id = ?1",
        params![survivor],
        |r| {
            Ok([
                r.get::<_, i64>(0)? == 1,
                r.get::<_, i64>(1)? == 1,
                r.get::<_, i64>(2)? == 1,
                r.get::<_, i64>(3)? == 1,
            ])
        },
    )?;

    let mut st = tx.prepare(
        "SELECT id, absorbed_project_id, merged_at, association_kind, evidence_json,
                absorbed_json
           FROM merge_record WHERE survivor_project_id = ?1 ORDER BY merged_at, id",
    )?;
    let rows = st
        .query_map(params![survivor], |r| -> rusqlite::Result<MergeRecordRow> {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;

    let mut out = Vec::new();
    for (id, absorbed, merged_at, kind, evidence_json, absorbed_json) in rows {
        let blob: serde_json::Value =
            serde_json::from_str(&absorbed_json).unwrap_or(serde_json::Value::Null);
        // `.get`, not `Index`: `indexing_slicing` is denied crate-wide, and a `merge_record`
        // written by an older build may legitimately lack a key.
        let field = |object: &str, key: &str| {
            blob.get(object)
                .and_then(|v| v.get(key))
                .and_then(serde_json::Value::as_i64)
        };
        let count = |key: &str| field("reparented", key).unwrap_or(0);
        let was = |key: &str| field("flags", key) == Some(1);

        // "OR'd away" is an absorbed 0 that the survivor now reads as 1; "AND'd away" is an
        // absorbed 1 the survivor now reads as 0.
        let mut flags_lost_by_or = Vec::new();
        for (i, name) in ["is_pinned", "is_archived"].iter().enumerate() {
            if !was(name) && current.get(i).copied().unwrap_or(false) {
                flags_lost_by_or.push((*name).to_owned());
            }
        }
        let mut flags_lost_by_and = Vec::new();
        for (i, name) in ["is_hidden", "is_reference"].iter().enumerate() {
            if was(name) && !current.get(i + 2).copied().unwrap_or(false) {
                flags_lost_by_and.push((*name).to_owned());
            }
        }

        out.push(UnmergeHint {
            merge_record_id: id,
            absorbed_project_id: absorbed,
            merged_at,
            association_kind: super::AssociationKind::parse(&kind)
                .unwrap_or(super::AssociationKind::Manual),
            evidence_json,
            notes_were_concatenated: blob
                .get("notes_offset")
                .and_then(serde_json::Value::as_i64)
                .is_some_and(|o| o > 0),
            locations: count("location"),
            sessions: count("session"),
            collection_members: count("collection_member"),
            xp_events: count("xp_events"),
            flags_lost_by_or,
            flags_lost_by_and,
        });
    }
    Ok(out)
}

#[allow(clippy::struct_excessive_bools)] // §1.2's four organisation flags; see AbsorbedSnapshot.
struct Row {
    name: String,
    description: Option<String>,
    description_source: Option<String>,
    seed_basename: String,
    reroll_offset: i64,
    is_pinned: bool,
    is_archived: bool,
    is_hidden: bool,
    is_reference: bool,
    notes: Option<String>,
}

fn read_row(tx: &Transaction<'_>, id: i64) -> Result<Row, IdentityError> {
    tx.query_row(
        "SELECT name, description, description_source, seed_basename, reroll_offset,
                is_pinned, is_archived, is_hidden, is_reference, notes
           FROM project WHERE id = ?1",
        params![id],
        |r| {
            Ok(Row {
                name: r.get(0)?,
                description: r.get(1)?,
                description_source: r.get(2)?,
                seed_basename: r.get(3)?,
                reroll_offset: r.get(4)?,
                is_pinned: r.get::<_, i64>(5)? == 1,
                is_archived: r.get::<_, i64>(6)? == 1,
                is_hidden: r.get::<_, i64>(7)? == 1,
                is_reference: r.get::<_, i64>(8)? == 1,
                notes: r.get(9)?,
            })
        },
    )
    .map_err(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => IdentityError::UnknownProject(id),
        other => IdentityError::Sqlite(other),
    })
}

#[cfg(test)]
mod tests {
    // `indexing_slicing`: `serde_json::Value`'s `Index` is how `absorbed_json` is read back,
    // and a missing key yields `Value::Null` rather than panicking.
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]
    use super::super::testutil::{insert_project, open_test_index, NewProject};
    use super::{choose_survivor, reconcile_scalars, NOTE_SEPARATOR};

    fn p(conn: &rusqlite::Connection, name: &'static str, created_at: i64) -> i64 {
        insert_project(
            conn,
            NewProject {
                name,
                lineage_key: Some("L"),
                remote_key: None,
                created_at,
            },
        )
    }

    fn field<T: rusqlite::types::FromSql>(tx: &rusqlite::Transaction<'_>, id: i64, col: &str) -> T {
        tx.query_row(
            &format!("SELECT {col} FROM project WHERE id=?1"),
            rusqlite::params![id],
            |r| r.get(0),
        )
        .unwrap()
    }

    #[test]
    fn the_survivor_is_the_earliest_created_and_ties_break_on_the_lowest_id() {
        let mut conn = open_test_index();
        let old = p(&conn, "old", 100);
        let new = p(&conn, "new", 200);
        let tx = conn.transaction().unwrap();
        // Walk-order independent: the arguments may arrive either way round.
        assert_eq!(choose_survivor(&tx, old, new).unwrap(), (old, new));
        assert_eq!(choose_survivor(&tx, new, old).unwrap(), (old, new));
        tx.commit().unwrap();

        let a = p(&conn, "a", 50);
        let b = p(&conn, "b", 50);
        let tx = conn.transaction().unwrap();
        assert_eq!(choose_survivor(&tx, b, a).unwrap(), (a.min(b), a.max(b)));
        tx.commit().unwrap();
    }

    #[test]
    fn notes_are_concatenated_survivor_first_and_never_discarded() {
        let mut conn = open_test_index();
        let s = p(&conn, "s", 100);
        let a = p(&conn, "a", 200);
        conn.execute(
            "UPDATE project SET notes='survivor text' WHERE id=?1",
            rusqlite::params![s],
        )
        .unwrap();
        conn.execute(
            "UPDATE project SET notes='absorbed text' WHERE id=?1",
            rusqlite::params![a],
        )
        .unwrap();

        let tx = conn.transaction().unwrap();
        let snap = reconcile_scalars(&tx, s, a, 300).unwrap();
        let merged: String = field(&tx, s, "notes");
        assert_eq!(
            merged,
            format!("survivor text{NOTE_SEPARATOR}absorbed text")
        );
        let offset = usize::try_from(snap.notes_offset.unwrap()).unwrap();
        assert_eq!(merged.get(offset..), Some("absorbed text"));
        tx.commit().unwrap();
    }

    #[test]
    fn an_absorbed_note_with_no_survivor_note_needs_no_rule_line() {
        let mut conn = open_test_index();
        let s = p(&conn, "s", 100);
        let a = p(&conn, "a", 200);
        conn.execute(
            "UPDATE project SET notes='only mine' WHERE id=?1",
            rusqlite::params![a],
        )
        .unwrap();
        let tx = conn.transaction().unwrap();
        let snap = reconcile_scalars(&tx, s, a, 300).unwrap();
        assert_eq!(field::<String>(&tx, s, "notes"), "only mine");
        assert_eq!(snap.notes_offset, Some(0));
        tx.commit().unwrap();
    }

    #[test]
    fn no_note_on_either_side_records_no_offset_and_writes_no_rule_line() {
        let mut conn = open_test_index();
        let s = p(&conn, "s", 100);
        let a = p(&conn, "a", 200);
        let tx = conn.transaction().unwrap();
        let snap = reconcile_scalars(&tx, s, a, 300).unwrap();
        assert_eq!(field::<Option<String>>(&tx, s, "notes"), None);
        assert_eq!(snap.notes_offset, None);
        tx.commit().unwrap();
    }

    #[test]
    fn pinned_and_archived_or_while_hidden_and_reference_and() {
        let mut conn = open_test_index();
        let s = p(&conn, "s", 100);
        let a = p(&conn, "a", 200);
        // The deliberate act wins; a project hidden in only one row stays visible; evidence of
        // authorship anywhere makes it yours.
        conn.execute(
            "UPDATE project SET is_pinned=0, is_archived=1, is_hidden=1, is_reference=1
              WHERE id=?1",
            rusqlite::params![s],
        )
        .unwrap();
        conn.execute(
            "UPDATE project SET is_pinned=1, is_archived=0, is_hidden=0, is_reference=1
              WHERE id=?1",
            rusqlite::params![a],
        )
        .unwrap();

        let tx = conn.transaction().unwrap();
        let snap = reconcile_scalars(&tx, s, a, 300).unwrap();
        assert_eq!(field::<i64>(&tx, s, "is_pinned"), 1);
        assert_eq!(field::<i64>(&tx, s, "is_archived"), 1);
        assert_eq!(field::<i64>(&tx, s, "is_hidden"), 0);
        assert_eq!(field::<i64>(&tx, s, "is_reference"), 1);
        // The pre-merge values survive in the snapshot, which is what merge_record stores.
        assert!(snap.is_pinned && !snap.is_archived && !snap.is_hidden && snap.is_reference);
        tx.commit().unwrap();
    }

    #[test]
    fn name_description_and_art_seed_stay_the_survivors() {
        let mut conn = open_test_index();
        let s = p(&conn, "s", 100);
        let a = p(&conn, "a", 200);
        conn.execute(
            "UPDATE project SET description='theirs', description_source='readme',
                                seed_basename='absorbed-dir', reroll_offset=3 WHERE id=?1",
            rusqlite::params![a],
        )
        .unwrap();
        let tx = conn.transaction().unwrap();
        let snap = reconcile_scalars(&tx, s, a, 300).unwrap();
        assert_eq!(field::<String>(&tx, s, "name"), "s");
        assert_eq!(field::<Option<String>>(&tx, s, "description"), None);
        assert_eq!(field::<String>(&tx, s, "seed_basename"), "s");
        assert_eq!(field::<i64>(&tx, s, "reroll_offset"), 0);
        // …and the absorbed ones are recoverable, which is the only reason a split is
        // conceivable at all (§1.5, §1.9).
        assert_eq!(snap.name, "a");
        assert_eq!(snap.description.as_deref(), Some("theirs"));
        assert_eq!(snap.seed_basename, "absorbed-dir");
        assert_eq!(snap.reroll_offset, 3);
        tx.commit().unwrap();
    }

    fn count(tx: &rusqlite::Transaction<'_>, sql: &str, id: i64) -> i64 {
        tx.query_row(sql, rusqlite::params![id], |r| r.get(0))
            .unwrap()
    }

    #[test]
    fn locations_sessions_and_health_rows_move_to_the_survivor() {
        let mut conn = open_test_index();
        let s = p(&conn, "s", 100);
        let a = p(&conn, "a", 200);
        super::super::testutil::insert_location(&conn, s, "/w/s", None);
        let al = super::super::testutil::insert_location(&conn, a, "/w/a", None);
        // `ended_at` and `close_reason` are set together: the DDL's
        // `CHECK ((ended_at IS NULL) = (close_reason IS NULL))` rejects one without the other,
        // and the plan's fixture supplies only the reason.
        conn.execute(
            "INSERT INTO session (project_id, location_id, started_at, ended_at,
                                  credited_seconds, close_reason)
             VALUES (?1, ?2, 10, 70, 60, 'stop')",
            rusqlite::params![a, al],
        )
        .unwrap();
        let sess = conn.last_insert_rowid();
        // `closed_by` is a different vocabulary from `close_reason` — 'stop' is not one of its
        // four values, and the plan's fixture uses it.
        conn.execute(
            "INSERT INTO session_segment (session_id, started_at, ended_at, credited_seconds,
                                          closed_by) VALUES (?1, 10, 70, 60, 'session_end')",
            rusqlite::params![sess],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO health_delta (project_id, ts, layer, from_value, to_value, detected_in)
             VALUES (?1, 5, 'x', 0, 1, 'background')",
            rusqlite::params![a],
        )
        .unwrap();

        let tx = conn.transaction().unwrap();
        let n = super::reparent_rows(&tx, s, a).unwrap();
        assert_eq!((n.location, n.session, n.health_delta), (1, 1, 1));
        assert_eq!(
            count(&tx, "SELECT COUNT(*) FROM location WHERE project_id=?1", s),
            2
        );
        assert_eq!(
            count(&tx, "SELECT COUNT(*) FROM session WHERE project_id=?1", a),
            0
        );
        // A segment names its session, not its project, so it follows without being touched.
        assert_eq!(
            count(
                &tx,
                "SELECT COUNT(*) FROM session_segment WHERE session_id=?1",
                sess
            ),
            1
        );
        tx.commit().unwrap();
    }

    #[test]
    fn collection_membership_is_a_union_and_never_doubles_a_count() {
        let mut conn = open_test_index();
        let s = p(&conn, "s", 100);
        let a = p(&conn, "a", 200);
        conn.execute(
            "INSERT INTO collection (name, kind, sort_index) VALUES ('c','manual',0)",
            [],
        )
        .unwrap();
        let c = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO collection (name, kind, sort_index) VALUES ('d','manual',1)",
            [],
        )
        .unwrap();
        let d = conn.last_insert_rowid();
        for (col, proj) in [(c, s), (c, a), (d, a)] {
            conn.execute(
                "INSERT INTO collection_member (collection_id, project_id) VALUES (?1, ?2)",
                rusqlite::params![col, proj],
            )
            .unwrap();
        }

        let tx = conn.transaction().unwrap();
        super::reparent_rows(&tx, s, a).unwrap();
        assert_eq!(
            count(
                &tx,
                "SELECT COUNT(*) FROM collection_member WHERE project_id=?1",
                s
            ),
            2,
            "both collections, each once"
        );
        assert_eq!(
            count(
                &tx,
                "SELECT COUNT(*) FROM collection_member WHERE project_id=?1",
                a
            ),
            0
        );
        tx.commit().unwrap();
    }

    #[test]
    fn a_launch_target_collision_keeps_the_survivors_row_and_disables_the_other() {
        let mut conn = open_test_index();
        let s = p(&conn, "s", 100);
        let a = p(&conn, "a", 200);
        for (proj, kind, name) in [
            (s, "editor", "ed"),
            (a, "editor", "ed"),
            (a, "terminal", "tm"),
        ] {
            conn.execute(
                "INSERT INTO launch_target (project_id, kind, name, exec_bytes, args_json,
                                            cwd_mode, sort_index, detected, disabled)
                 VALUES (?1, ?2, ?3, ?4, '[]', 'location', 0, 1, 0)",
                rusqlite::params![proj, kind, name, name.as_bytes()],
            )
            .unwrap();
        }

        let tx = conn.transaction().unwrap();
        let n = super::reparent_rows(&tx, s, a).unwrap();
        assert_eq!(n.launch_target, 2);
        assert_eq!(n.launch_target_disabled, 1);
        let disabled: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM launch_target WHERE project_id=?1 AND disabled=1",
                rusqlite::params![s],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            disabled, 1,
            "the absorbed editor row survives, disabled — never deleted"
        );
        assert_eq!(
            count(
                &tx,
                "SELECT COUNT(*) FROM launch_target WHERE project_id=?1",
                s
            ),
            3
        );
        tx.commit().unwrap();
    }

    #[test]
    fn submodule_edges_and_earlier_merge_records_follow_the_survivor() {
        let mut conn = open_test_index();
        let s = p(&conn, "s", 100);
        let a = p(&conn, "a", 200);
        let child = p(&conn, "child", 300);
        // A real project id, not 999: `merge_record.absorbed_project_id` is a foreign key and
        // the fixture opens with `PRAGMA foreign_keys=ON`, so the plan's row cannot be inserted.
        let earlier = p(&conn, "earlier", 400);
        let al = super::super::testutil::insert_location(&conn, a, "/w/a", None);
        conn.execute(
            "INSERT INTO submodule_edge (parent_project_id, child_project_id, parent_location_id,
                                         path_bytes, gitlink_oid)
             VALUES (?1, ?2, ?3, ?4, 'abc')",
            rusqlite::params![a, child, al, b"vendor/lib".as_slice()],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO merge_record (survivor_project_id, absorbed_project_id, merged_at,
                                       association_kind, evidence_json, absorbed_json)
             VALUES (?1, ?2, 50, 'manual', '{}', '{}')",
            rusqlite::params![a, earlier],
        )
        .unwrap();

        let tx = conn.transaction().unwrap();
        let n = super::reparent_rows(&tx, s, a).unwrap();
        assert_eq!(n.submodule_edge, 1);
        assert_eq!(
            count(
                &tx,
                "SELECT COUNT(*) FROM submodule_edge WHERE parent_project_id=?1",
                s
            ),
            1
        );
        assert_eq!(
            count(
                &tx,
                "SELECT COUNT(*) FROM merge_record WHERE survivor_project_id=?1",
                s
            ),
            1,
            "an older merge into the absorbed row now names the survivor"
        );
        tx.commit().unwrap();
    }

    #[test]
    fn a_parent_link_the_survivor_lacks_is_inherited_rather_than_lost() {
        let mut conn = open_test_index();
        let s = p(&conn, "s", 100);
        let a = p(&conn, "a", 200);
        let parent = p(&conn, "parent", 50);
        conn.execute(
            "UPDATE project SET parent_project_id=?2, submodule_path=?3 WHERE id=?1",
            rusqlite::params![a, parent, "vendor/lib"],
        )
        .unwrap();

        let tx = conn.transaction().unwrap();
        super::reparent_rows(&tx, s, a).unwrap();
        assert_eq!(
            field::<Option<i64>>(&tx, s, "parent_project_id"),
            Some(parent)
        );
        assert_eq!(
            field::<Option<String>>(&tx, s, "submodule_path"),
            Some("vendor/lib".to_owned())
        );
        tx.commit().unwrap();
    }

    /// `track` is not a free label. `0003`'s
    /// `CHECK ((track = 'session') = (kind IN ('session', 'focus')))` ties it to the kind, so
    /// the plan's `'main'` cannot be inserted at all — and the tie is what lets the sweep key
    /// on the track rather than restating the five git kinds in SQL.
    fn xp(conn: &rusqlite::Connection, project: i64, kind: &str, dedupe: &str) {
        let track = if matches!(kind, "session" | "focus") {
            "session"
        } else {
            "git"
        };
        conn.execute(
            "INSERT INTO xp_events (ts, tz_offset_min, project_id, subject_key, kind,
                                    dedupe_key, track, meta)
             VALUES (1, 0, ?1, 'sub', ?2, ?3, ?4, '{}')",
            rusqlite::params![project, kind, dedupe, track],
        )
        .unwrap();
    }

    /// R26's shape, checked against the database rather than against the source: the five kinds
    /// this module calls git-derived are exactly the kinds the DDL refuses to file under
    /// `track = 'session'`. If a sixth is added to one side only, this fails.
    #[test]
    fn every_git_derived_kind_is_one_the_ddl_files_under_the_git_track() {
        let conn = open_test_index();
        let p = p(&conn, "s", 100);
        for (i, kind) in super::GIT_DERIVED_XP_KINDS.iter().enumerate() {
            xp(&conn, p, kind, &format!("ok:{i}"));
            let wrong = conn.execute(
                "INSERT INTO xp_events (ts, tz_offset_min, project_id, subject_key, kind,
                                        dedupe_key, track, meta)
                 VALUES (1, 0, ?1, 'sub', ?2, ?3, 'session', '{}')",
                rusqlite::params![p, kind, format!("bad:{i}")],
            );
            assert!(
                wrong.is_err(),
                "{kind} must not be storable as a session event"
            );
        }
        let git: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM xp_events WHERE track = 'git'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            git,
            i64::try_from(super::GIT_DERIVED_XP_KINDS.len()).unwrap()
        );
    }

    #[test]
    fn git_derived_events_are_deleted_for_both_sides_and_session_events_reparent() {
        let mut conn = open_test_index();
        let s = p(&conn, "s", 100);
        let a = p(&conn, "a", 200);
        xp(&conn, s, "commit_day", "commit_day:L::2026-01-01");
        // A fork and its upstream can hold the same day; the remote component is what separates
        // them, and after a merge neither may survive as a stale duplicate.
        xp(
            &conn,
            a,
            "commit_day",
            "commit_day:L:forge.example/mine/w:2026-01-01",
        );
        xp(&conn, a, "release", "release:L::v1");
        xp(&conn, a, "session", "session:42");
        xp(&conn, s, "focus", "focus:7");

        let tx = conn.transaction().unwrap();
        let d = super::recompute_derived(&tx, s, a).unwrap();
        assert_eq!(d.git_events_deleted, 3);
        assert_eq!(d.xp_events_reparented, 1);
        let left: i64 = tx
            .query_row("SELECT COUNT(*) FROM xp_events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(left, 2, "both session-class rows, and nothing git-derived");
        assert_eq!(
            count(&tx, "SELECT COUNT(*) FROM xp_events WHERE project_id=?1", s),
            2
        );
        tx.commit().unwrap();
    }

    #[test]
    fn the_level_floor_is_stamped_before_the_delete_and_only_ever_rises() {
        let mut conn = open_test_index();
        let s = p(&conn, "s", 100);
        let a = p(&conn, "a", 200);
        xp(&conn, s, "commit_day", "k1");
        xp(&conn, a, "commit_day", "k2");
        xp(&conn, a, "commit_day", "k3");

        let tx = conn.transaction().unwrap();
        let d = super::recompute_derived(&tx, s, a).unwrap();
        assert_eq!(d.level_floor, 3);
        let stored: String = tx
            .query_row("SELECT v FROM app_meta WHERE k='level_floor'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(stored, "3");
        tx.commit().unwrap();

        // A second, smaller merge must not lower it. Nothing earned is ever taken away.
        let b = p(&conn, "b", 300);
        let tx = conn.transaction().unwrap();
        let d2 = super::recompute_derived(&tx, s, b).unwrap();
        assert_eq!(d2.level_floor, 3);
        tx.commit().unwrap();
        let stored: String = conn
            .query_row("SELECT v FROM app_meta WHERE k='level_floor'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(stored, "3");
    }

    #[test]
    fn every_derived_cache_is_dropped_for_both_and_the_absorbed_art_is_handed_back() {
        let mut conn = open_test_index();
        let s = p(&conn, "s", 100);
        let a = p(&conn, "a", 200);
        for proj in [s, a] {
            conn.execute(
                "INSERT INTO fts_commits (project_id, subjects) VALUES (?1, 'x')",
                rusqlite::params![proj],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO peek_cache (project_id, computed_at) VALUES (?1, 1)",
                rusqlite::params![proj],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO project_committer (project_id, email, commits)
                 VALUES (?1, 'a@b', 2)",
                rusqlite::params![proj],
            )
            .unwrap();
            // The job slugs are lowercase in the DDL's CHECK; the plan writes 'J4'.
            conn.execute(
                "INSERT INTO project_job_state (project_id, job, state, fail_count, at)
                 VALUES (?1, 'j4', 'ok', 0, 1)",
                rusqlite::params![proj],
            )
            .unwrap();
        }
        conn.execute(
            "INSERT INTO art_scene (project_id, scene_hash, scene_json, schema_version,
                                    state, fail_count)
             VALUES (?1, 'hash-a', '{}', 1, 'ready', 0)",
            rusqlite::params![a],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO art_scene (project_id, scene_hash, scene_json, schema_version,
                                    state, fail_count)
             VALUES (?1, 'hash-s', '{}', 1, 'ready', 0)",
            rusqlite::params![s],
        )
        .unwrap();

        let tx = conn.transaction().unwrap();
        let d = super::recompute_derived(&tx, s, a).unwrap();
        assert_eq!(d.swept_scene_hash.as_deref(), Some("hash-a"));
        for table in ["fts_commits", "peek_cache", "project_committer"] {
            let n: i64 = tx
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
                .unwrap();
            assert_eq!(
                n, 0,
                "{table} is a pure function of history and is recomputed, not moved"
            );
        }
        assert_eq!(
            count(
                &tx,
                "SELECT COUNT(*) FROM project_job_state WHERE project_id=?1",
                a
            ),
            0
        );
        assert_eq!(
            count(&tx, "SELECT COUNT(*) FROM art_scene WHERE project_id=?1", s),
            1,
            "the survivor's scene is kept"
        );
        tx.commit().unwrap();
    }

    #[test]
    fn a_merge_leaves_no_orphaned_rows_and_tombstones_rather_than_deletes() {
        let mut conn = open_test_index();
        let s = p(&conn, "s", 100);
        let a = p(&conn, "a", 200);
        super::super::testutil::insert_location(&conn, a, "/w/a", None);
        xp(&conn, a, "commit_day", "k1");
        xp(&conn, a, "session", "s1");
        conn.execute(
            "UPDATE project SET notes='theirs', is_pinned=1 WHERE id=?1",
            rusqlite::params![a],
        )
        .unwrap();

        let tx = conn.transaction().unwrap();
        let out = super::merge_projects(
            &tx,
            a,
            s,
            super::super::AssociationKind::Manual,
            &serde_json::json!({"rule": "manual"}),
            300,
        )
        .unwrap();
        tx.commit().unwrap();

        assert_eq!((out.survivor, out.absorbed), (s, a));
        assert!(out.needs_history_recompute);
        assert_eq!(out.reparented.location, 1);
        assert_eq!(out.derived.xp_events_reparented, 1);
        assert_eq!(out.derived.git_events_deleted, 1);

        // The row is tombstoned, never deleted, under an id that can never be reused.
        let merged_into: Option<i64> = conn
            .query_row(
                "SELECT merged_into FROM project WHERE id=?1",
                rusqlite::params![a],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(merged_into, Some(s));

        // Nothing is left pointing at it.
        for (table, col) in [
            ("location", "project_id"),
            ("session", "project_id"),
            ("xp_events", "project_id"),
            ("collection_member", "project_id"),
            ("launch_target", "project_id"),
        ] {
            let n: i64 = conn
                .query_row(
                    &format!("SELECT COUNT(*) FROM {table} WHERE {col} = ?1"),
                    rusqlite::params![a],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(n, 0, "{table} still names the absorbed project");
        }

        let target: i64 = conn
            .query_row(
                "SELECT new_project_id FROM project_redirect WHERE old_project_id=?1",
                rusqlite::params![a],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(target, s);

        let (kind, absorbed_json): (String, String) = conn
            .query_row(
                "SELECT association_kind, absorbed_json FROM merge_record WHERE id=?1",
                rusqlite::params![out.merge_record_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(kind, "manual");
        let parsed: serde_json::Value = serde_json::from_str(&absorbed_json).unwrap();
        assert_eq!(parsed["name"], "a");
        assert_eq!(parsed["flags"]["is_pinned"], 1);
        assert_eq!(parsed["reparented"]["location"], 1);

        // The survivor's footer now reads MERGED BY YOU (§8.5.2).
        let footer: Option<String> = conn
            .query_row(
                "SELECT association_kind FROM project WHERE id=?1",
                rusqlite::params![s],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(footer.as_deref(), Some("manual"));
    }

    #[test]
    fn the_whole_merge_rolls_back_together() {
        let mut conn = open_test_index();
        let s = p(&conn, "s", 100);
        let a = p(&conn, "a", 200);
        super::super::testutil::insert_location(&conn, a, "/w/a", None);

        let tx = conn.transaction().unwrap();
        super::merge_projects(
            &tx,
            s,
            a,
            super::super::AssociationKind::Manual,
            &serde_json::json!({}),
            300,
        )
        .unwrap();
        drop(tx); // rolls back

        let still_there: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM location WHERE project_id=?1",
                rusqlite::params![a],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(still_there, 1);
        let merged_into: Option<i64> = conn
            .query_row(
                "SELECT merged_into FROM project WHERE id=?1",
                rusqlite::params![a],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(merged_into, None);
        let redirects: i64 = conn
            .query_row("SELECT COUNT(*) FROM project_redirect", [], |r| r.get(0))
            .unwrap();
        assert_eq!(redirects, 0);
    }

    #[test]
    fn a_stale_id_is_resolved_before_anything_is_written() {
        let mut conn = open_test_index();
        let s = p(&conn, "s", 100);
        let a = p(&conn, "a", 200);
        let c = p(&conn, "c", 300);
        let tx = conn.transaction().unwrap();
        super::merge_projects(
            &tx,
            s,
            a,
            super::super::AssociationKind::Manual,
            &serde_json::json!({}),
            300,
        )
        .unwrap();
        // `a` is now a stale id. Merging it again must land on its survivor.
        let out = super::merge_projects(
            &tx,
            a,
            c,
            super::super::AssociationKind::Manual,
            &serde_json::json!({}),
            400,
        )
        .unwrap();
        assert_eq!((out.survivor, out.absorbed), (s, c));
        tx.commit().unwrap();
    }

    #[test]
    fn merging_a_project_into_itself_is_refused() {
        let mut conn = open_test_index();
        let s = p(&conn, "s", 100);
        let tx = conn.transaction().unwrap();
        assert!(matches!(
            super::merge_projects(
                &tx,
                s,
                s,
                super::super::AssociationKind::Manual,
                &serde_json::json!({}),
                300
            ),
            Err(super::super::IdentityError::SameProject(_))
        ));
        tx.commit().unwrap();
    }

    #[test]
    fn the_hint_reports_what_a_split_would_have_to_decide_and_writes_nothing() {
        let mut conn = open_test_index();
        let s = p(&conn, "s", 100);
        let a = p(&conn, "a", 200);
        super::super::testutil::insert_location(&conn, a, "/w/a", None);
        conn.execute(
            "UPDATE project SET notes='survivor' WHERE id=?1",
            rusqlite::params![s],
        )
        .unwrap();
        conn.execute(
            "UPDATE project SET notes='absorbed', is_pinned=0, is_hidden=1 WHERE id=?1",
            rusqlite::params![a],
        )
        .unwrap();
        conn.execute(
            "UPDATE project SET is_pinned=1, is_hidden=1 WHERE id=?1",
            rusqlite::params![s],
        )
        .unwrap();
        xp(&conn, a, "session", "s1");

        let tx = conn.transaction().unwrap();
        super::merge_projects(
            &tx,
            s,
            a,
            super::super::AssociationKind::Inferred,
            &serde_json::json!({"rule": "inferred", "lineage_key": "L"}),
            300,
        )
        .unwrap();
        tx.commit().unwrap();

        let before: i64 = conn
            .query_row("SELECT COUNT(*) FROM project", [], |r| r.get(0))
            .unwrap();
        let tx = conn.transaction().unwrap();
        let hints = super::unmerge_hint(&tx, s).unwrap();
        tx.commit().unwrap();

        assert_eq!(hints.len(), 1);
        let h = hints.first().unwrap();
        assert_eq!(h.absorbed_project_id, a);
        assert_eq!(h.association_kind, super::super::AssociationKind::Inferred);
        assert!(h.evidence_json.contains("inferred"));
        assert!(h.notes_were_concatenated);
        assert_eq!((h.locations, h.sessions, h.xp_events), (1, 0, 1));
        // The absorbed row was not pinned; the survivor is, because is_pinned is OR'd.
        assert_eq!(h.flags_lost_by_or, vec!["is_pinned".to_owned()]);
        // Both were hidden and the survivor still is, so nothing was AND'd away.
        assert!(h.flags_lost_by_and.is_empty());

        let after: i64 = conn
            .query_row("SELECT COUNT(*) FROM project", [], |r| r.get(0))
            .unwrap();
        assert_eq!(before, after, "the hint reports; it never reverses");
    }

    #[test]
    fn a_project_that_absorbed_nothing_has_no_hint() {
        let mut conn = open_test_index();
        let s = p(&conn, "s", 100);
        let tx = conn.transaction().unwrap();
        assert!(super::unmerge_hint(&tx, s).unwrap().is_empty());
        tx.commit().unwrap();
    }
}
