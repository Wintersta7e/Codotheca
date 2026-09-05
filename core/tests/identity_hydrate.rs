//! §22.4 — hydration, the scan direction.
//!
//! The ordinary sequence *connect → sync → clone in a terminal → rescan* mints a second tile
//! today, because a not-cloned project's `lineage_key` is NULL and it is therefore in no
//! candidate set `decide` can see.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use codotheca_core::accounts::keychain::SecretToken;
use codotheca_core::identity::alias::HostAliases;
use codotheca_core::identity::decide::IdentityProbe;
use codotheca_core::identity::hydrate::{
    find_hydration_target, HydrationTarget, HYDRATION_TARGET_SQL,
};
use codotheca_core::identity::ingest::ingest_listing;
use codotheca_core::identity::store::resolve_identity;
use codotheca_core::identity::IdentityError;
use codotheca_core::index::migrate::{apply_all, MIGRATIONS};
use codotheca_core::index::{open_connection, Index};
use codotheca_core::provider::listing::{OrgListing, Page, RepoListing, Viewer};
use codotheca_core::provider::{Observed, Provider, ProviderResult};

#[derive(Debug)]
struct DeclaringForge;

impl Provider for DeclaringForge {
    fn viewer(&self, _t: &SecretToken) -> ProviderResult<Observed<Viewer>> {
        unreachable!("hydration issues no request")
    }
    fn list_orgs(
        &self,
        _t: &SecretToken,
        _cur: Option<&str>,
    ) -> ProviderResult<Observed<Page<OrgListing>>> {
        unreachable!("hydration issues no request")
    }
    fn list_repos(
        &self,
        _t: &SecretToken,
        _cur: Option<&str>,
    ) -> ProviderResult<Observed<Page<RepoListing>>> {
        unreachable!("hydration issues no request")
    }
    fn lookup_repo(
        &self,
        _t: &SecretToken,
        _owner: &str,
        _name: &str,
    ) -> ProviderResult<Observed<Option<RepoListing>>> {
        unreachable!("this fixture forge issues no request")
    }
    fn canonical_host(&self) -> &'static str {
        "forge.example"
    }
    fn host_aliases(&self) -> &[&str] {
        &["forge.example", "www.forge.example", "ssh.forge.example"]
    }
}

fn aliases() -> HostAliases {
    HostAliases::from_provider(&DeclaringForge)
}

fn migrated() -> (tempfile::TempDir, rusqlite::Connection) {
    let dir = tempfile::tempdir().unwrap();
    let mut conn = open_connection(&Index::db_path(dir.path())).unwrap();
    apply_all(&mut conn, MIGRATIONS).unwrap();
    (dir, conn)
}

fn listing(repo_id: &str, owner: &str, name: &str) -> RepoListing {
    RepoListing {
        provider: "github",
        provider_repo_id: repo_id.to_owned(),
        clone_url: format!("https://forge.example/{owner}/{name}.git"),
        owner: owner.to_owned(),
        name: name.to_owned(),
        can_push: Some(true),
        is_fork: false,
        fork_parent_clone_url: None,
        is_archived: false,
        is_private: false,
        in_org: None,
    }
}

/// A clone of `<owner>/<name>` as a scan would probe it.
fn clone_probe(root: &str, url: &str, is_shallow: bool) -> IdentityProbe {
    IdentityProbe {
        common_dir_key: None,
        is_shallow,
        root_oids: if is_shallow {
            Vec::new()
        } else {
            vec![root.to_owned()]
        },
        remote_urls: vec![("origin".to_owned(), url.to_owned())],
    }
}

fn row_snapshot(conn: &rusqlite::Connection, id: i64) -> Vec<(String, String)> {
    let names: Vec<String> = {
        let mut st = conn.prepare("PRAGMA table_info(project)").unwrap();
        let rows = st.query_map([], |r| r.get::<_, String>(1)).unwrap();
        rows.map(Result::unwrap).collect()
    };
    let list = names
        .iter()
        .map(|c| format!("\"{c}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let mut st = conn
        .prepare(&format!("SELECT {list} FROM project WHERE id = ?1"))
        .unwrap();
    st.query_row([id], |row| {
        let mut out = Vec::new();
        for (i, name) in names.iter().enumerate() {
            let value: rusqlite::types::Value = row.get(i)?;
            out.push((name.clone(), format!("{value:?}")));
        }
        Ok(out)
    })
    .unwrap()
}

fn moved(before: &[(String, String)], after: &[(String, String)]) -> Vec<String> {
    assert_eq!(before.len(), after.len(), "the column set itself moved");
    let mut out: Vec<String> = before
        .iter()
        .zip(after)
        .filter(|((_, was), (_, now))| was != now)
        .map(|((name, _), _)| name.clone())
        .collect();
    out.sort();
    out
}

fn project_count(conn: &rusqlite::Connection) -> i64 {
    conn.query_row("SELECT count(*) FROM project", [], |r| r.get(0))
        .unwrap()
}

/// The sync half: a listing creates a not-cloned project and returns its id.
fn synced(conn: &mut rusqlite::Connection, name: &str) -> i64 {
    let tx = conn.transaction().unwrap();
    let ingest = ingest_listing(&tx, &listing("42", "acme", name), &aliases(), 500).unwrap();
    tx.commit().unwrap();
    assert!(ingest.created);
    ingest.project_id.unwrap()
}

/// **AC-P2-22-4.** The whole row before and after, and the diff is exactly four columns.
/// **Never four separate equalities**: a per-column check passes while `seed_basename` moves.
#[test]
fn hydration_writes_four_columns_and_no_more() {
    let (_dir, mut conn) = migrated();
    let project = synced(&mut conn, "widget");
    // A shallow clone hydrated this row once already, so the full clone below moves `is_shallow`
    // back — otherwise the column cannot move at all from a fresh not-cloned row and a
    // four-column diff would be four by accident.
    conn.execute("UPDATE project SET is_shallow = 1 WHERE id = ?1", [project])
        .unwrap();

    let tx = conn.transaction().unwrap();
    // A sibling fork on the same lineage, so `is_fork` moves too.
    resolve_identity(
        &tx,
        &clone_probe("root-1", "https://forge.example/other/widget.git", false),
        "other",
        &aliases(),
        800,
    )
    .unwrap();
    let before = row_snapshot(&tx, project);
    let outcome = resolve_identity(
        &tx,
        &clone_probe("root-1", "https://forge.example/acme/widget.git", false),
        "widget-checkout",
        &aliases(),
        900,
    )
    .unwrap();
    let after = row_snapshot(&tx, project);
    tx.commit().unwrap();

    assert_eq!(outcome.project_id, project, "hydrated, not created");
    assert!(!outcome.created);
    assert_eq!(
        project_count(&conn),
        2,
        "the sibling fork, and the hydrated row"
    );
    assert_eq!(
        moved(&before, &after),
        vec![
            "is_fork".to_owned(),
            "is_shallow".to_owned(),
            "lineage_key".to_owned(),
            "updated_at".to_owned(),
        ],
        "hydration wrote a column outside §22.4's set"
    );
}

/// The same, with the fork flag moving too, so all four of §22.4's columns are exercised.
#[test]
fn hydration_may_set_is_fork_and_never_clears_it() {
    let (_dir, mut conn) = migrated();
    let upstream = synced(&mut conn, "widget");
    // A second local project under another owner makes the clone a fork by §1.1's rule.
    let tx = conn.transaction().unwrap();
    resolve_identity(
        &tx,
        &clone_probe("root-1", "https://forge.example/other/widget.git", false),
        "widget",
        &aliases(),
        800,
    )
    .unwrap();
    let before = row_snapshot(&tx, upstream);
    let outcome = resolve_identity(
        &tx,
        &clone_probe("root-1", "https://forge.example/acme/widget.git", false),
        "widget-checkout",
        &aliases(),
        900,
    )
    .unwrap();
    let after = row_snapshot(&tx, upstream);
    tx.commit().unwrap();

    assert_eq!(outcome.project_id, upstream);
    assert!(outcome.is_fork);
    assert_eq!(
        moved(&before, &after),
        vec![
            "is_fork".to_owned(),
            "lineage_key".to_owned(),
            "updated_at".to_owned(),
        ]
    );
}

/// **AC-P2-22-2.** The art does not re-roll. The `id` clause is what separates hydration from
/// create-plus-merge.
///
/// AC-P2-22-2 names `art_seed` and no such column exists: `project` carries `seed_basename`,
/// `art_scene_hash` and `reroll_offset`, and §7.3a derives every art value from `seed_basename`,
/// so `art_scene_hash` is the derived value that moves if the seed re-rolls. Both are asserted.
#[test]
fn the_art_does_not_re_roll() {
    let (_dir, mut conn) = migrated();
    let project = synced(&mut conn, "widget");
    conn.execute(
        "UPDATE project SET art_scene_hash = 'scene-hash-from-the-seed' WHERE id = ?1",
        [project],
    )
    .unwrap();

    let tx = conn.transaction().unwrap();
    // Cloned into a DIFFERENTLY-named directory and rescanned.
    let outcome = resolve_identity(
        &tx,
        &clone_probe("root-1", "https://forge.example/acme/widget.git", false),
        "widget-2",
        &aliases(),
        900,
    )
    .unwrap();
    tx.commit().unwrap();

    let (id, name, seed, hash, offset): (i64, String, String, String, i64) = conn
        .query_row(
            "SELECT id, name, seed_basename, art_scene_hash, reroll_offset
               FROM project WHERE id = ?1",
            [project],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .unwrap();
    assert_eq!(outcome.project_id, project);
    assert_eq!(
        id, project,
        "the row's id is unchanged — not a create-plus-merge"
    );
    assert_eq!(name, "widget");
    assert_eq!(
        seed, "widget",
        "the listing's bare name, not the directory's"
    );
    assert_eq!(hash, "scene-hash-from-the-seed");
    assert_eq!(offset, 0);
    assert_eq!(project_count(&conn), 1);
}

/// **AC-P2-22-5.** A guard with no failing case is not a guard.
#[test]
fn the_xp_guard_fires_and_rolls_the_transaction_back() {
    let (_dir, mut conn) = migrated();
    let project = synced(&mut conn, "widget");
    conn.execute(
        "INSERT INTO xp_events (ts, project_id, subject_key, kind, dedupe_key, track)
         VALUES (10, ?1, 'subject', 'commit_day', 'commit_day:lin:key:2026-01-01', 'git')",
        [project],
    )
    .unwrap();

    let tx = conn.transaction().unwrap();
    let failed = resolve_identity(
        &tx,
        &clone_probe("root-1", "https://forge.example/acme/widget.git", false),
        "widget",
        &aliases(),
        900,
    );
    match &failed {
        Err(IdentityError::HydrateWouldOrphanXp {
            project_id, count, ..
        }) => {
            assert_eq!(*project_id, project);
            assert_eq!(*count, 1);
        }
        other => panic!("expected HydrateWouldOrphanXp, got {other:?}"),
    }
    // Dropping the transaction is what rolls it back; the caller must not commit past this.
    drop(tx);

    let lineage: Option<String> = conn
        .query_row(
            "SELECT lineage_key FROM project WHERE id = ?1",
            [project],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(lineage, None, "the row was left unhydrated");
    assert_eq!(project_count(&conn), 1, "and nothing was created either");
}

/// §1.1's no-lineage case, and the one that duplicates with certainty without the rule.
#[test]
fn a_shallow_clone_of_a_listed_repository_hydrates() {
    let (_dir, mut conn) = migrated();
    let project = synced(&mut conn, "widget");

    let tx = conn.transaction().unwrap();
    let outcome = resolve_identity(
        &tx,
        &clone_probe("", "https://forge.example/acme/widget.git", true),
        "widget",
        &aliases(),
        900,
    )
    .unwrap();
    tx.commit().unwrap();

    assert_eq!(outcome.project_id, project);
    assert!(!outcome.created);
    let (lineage, shallow): (Option<String>, i64) = conn
        .query_row(
            "SELECT lineage_key, is_shallow FROM project WHERE id = ?1",
            [project],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(lineage, None, "a shallow clone still has no lineage");
    assert_eq!(shallow, 1, "and that is now an observation, not a default");
    assert_eq!(project_count(&conn), 1, "exactly one row");
}

/// A clone taken over a declared alias host still finds the row the listing created on the
/// canonical one.
#[test]
fn a_clone_on_an_alias_host_hydrates_the_canonical_row() {
    let (_dir, mut conn) = migrated();
    let project = synced(&mut conn, "widget");

    let tx = conn.transaction().unwrap();
    let outcome = resolve_identity(
        &tx,
        &clone_probe("root-1", "git@ssh.forge.example:acme/widget.git", false),
        "widget",
        &aliases(),
        900,
    )
    .unwrap();
    tx.commit().unwrap();
    assert_eq!(outcome.project_id, project);
    assert_eq!(project_count(&conn), 1);
}

#[test]
fn two_not_cloned_rows_on_one_key_create_and_flag() {
    let (_dir, mut conn) = migrated();
    // Two not-cloned rows on one folded key: the sync-side ambiguity §22.5 describes, reached
    // here directly because two accounts' listings collapse before ingest.
    let tx = conn.transaction().unwrap();
    for (name, key) in [
        ("widget", "forge.example/acme/widget"),
        ("widget-again", "www.forge.example/acme/widget"),
    ] {
        tx.execute(
            "INSERT INTO project (name, seed_basename, remote_key, created_at, updated_at)
             VALUES (?1, ?1, ?2, 100, 100)",
            rusqlite::params![name, key],
        )
        .unwrap();
    }
    let outcome = resolve_identity(
        &tx,
        &clone_probe("root-1", "https://forge.example/acme/widget.git", false),
        "widget",
        &aliases(),
        900,
    )
    .unwrap();
    let target = find_hydration_target(&tx, "forge.example/acme/widget", &aliases()).unwrap();
    tx.commit().unwrap();

    assert!(outcome.created, "never pick one of two");
    assert!(outcome.ambiguous);
    assert_eq!(project_count(&conn), 3);
    assert!(
        matches!(target, HydrationTarget::Many(ref ids) if ids.len() == 3),
        "{target:?}"
    );
    let flagged: i64 = conn
        .query_row(
            "SELECT count(*) FROM project WHERE ambiguous_lineage = 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(flagged, 3, "the whole group is flagged, not one of it");
}

/// §22.4: *"No index is added for §22.4's not-cloned lookup — `idx_project_remote` and
/// `location(project_id)` already serve it"*, which is only true if the statement lets them.
///
/// A scan reaches this **once per repository that would create a row**, so a table scan here is
/// quadratic over a first scan of the library — and no test that only checks the answer would say
/// so.
#[test]
fn the_hydration_lookup_uses_the_index() {
    let (_dir, conn) = migrated();
    let mut st = conn
        .prepare(&format!("EXPLAIN QUERY PLAN {HYDRATION_TARGET_SQL}"))
        .unwrap();
    let plan = st
        .query_map(["forge.example/acme/widget"], |r| r.get::<_, String>(3))
        .unwrap()
        .map(Result::unwrap)
        .collect::<Vec<_>>()
        .join(" | ");
    eprintln!("hydration target: {plan}");
    assert!(!plan.is_empty(), "no plan at all");
    assert!(
        plan.contains("idx_project_remote"),
        "the not-cloned lookup must not table-scan: {plan}"
    );
    assert!(!plan.contains("SCAN project"), "{plan}");
    assert!(
        plan.contains("idx_location_project"),
        "the zero-location test must not scan location either: {plan}"
    );
}

#[test]
fn a_project_with_a_copy_on_disk_is_not_a_hydration_target() {
    let (_dir, mut conn) = migrated();
    let project = synced(&mut conn, "widget");
    let tx = conn.transaction().unwrap();
    tx.execute(
        "INSERT INTO location (project_id, kind, distro, path_bytes, path_key, path_display,
                               store_key, presence, repo_kind, scan_generation)
         VALUES (?1, 'linux', '', ?2, ?2, '/w/widget', 'store', 'present', 'worktree', 1)",
        rusqlite::params![project, b"/w/widget".to_vec()],
    )
    .unwrap();
    let target = find_hydration_target(&tx, "forge.example/acme/widget", &aliases()).unwrap();
    tx.commit().unwrap();
    assert_eq!(target, HydrationTarget::None);
}

/// The hydrate path re-evaluates the lineage, the same call and the same reason as the `NewFork`
/// arm: a remoteless sibling that had one candidate may have two once this lineage is known.
#[test]
fn the_hydrate_path_re_evaluates_the_lineage() {
    let (_dir, mut conn) = migrated();
    let listed = synced(&mut conn, "widget");

    let tx = conn.transaction().unwrap();
    // The remoteless sibling arrives first, so it gets its own row rather than attaching, and one
    // remoted fork joins the lineage after it. One candidate is not yet an ambiguity.
    let orphan = resolve_identity(
        &tx,
        &IdentityProbe {
            common_dir_key: None,
            is_shallow: false,
            root_oids: vec!["root-1".to_owned()],
            remote_urls: Vec::new(),
        },
        "local",
        &aliases(),
        700,
    )
    .unwrap();
    resolve_identity(
        &tx,
        &clone_probe("root-1", "https://forge.example/other/widget.git", false),
        "other",
        &aliases(),
        800,
    )
    .unwrap();
    assert!(!orphan.ambiguous, "one candidate is not an ambiguity yet");

    // Now the listed row is hydrated by a clone on the same lineage: the orphan has two.
    resolve_identity(
        &tx,
        &clone_probe("root-1", "https://forge.example/acme/widget.git", false),
        "widget",
        &aliases(),
        900,
    )
    .unwrap();
    tx.commit().unwrap();

    let flagged: i64 = conn
        .query_row(
            "SELECT ambiguous_lineage FROM project WHERE id = ?1",
            [orphan.project_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(flagged, 1);
    let hydrated: Option<String> = conn
        .query_row(
            "SELECT lineage_key FROM project WHERE id = ?1",
            [listed],
            |r| r.get(0),
        )
        .unwrap();
    assert!(hydrated.is_some(), "the listed row really was hydrated");
}

/// Hydration reaches none of the merge machinery. **`projects.merge` keeps its zero call sites.**
#[test]
fn hydration_writes_no_tombstone_and_no_merge_record() {
    let (_dir, mut conn) = migrated();
    let project = synced(&mut conn, "widget");
    let tx = conn.transaction().unwrap();
    resolve_identity(
        &tx,
        &clone_probe("root-1", "https://forge.example/acme/widget.git", false),
        "widget",
        &aliases(),
        900,
    )
    .unwrap();
    tx.commit().unwrap();

    for table in ["merge_record", "project_redirect"] {
        let n: i64 = conn
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0, "{table} gained a row");
    }
    let tombstoned: i64 = conn
        .query_row(
            "SELECT count(*) FROM project WHERE merged_into IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(tombstoned, 0);
    assert_eq!(project_count(&conn), 1);
    assert_eq!(project, 1);
}
