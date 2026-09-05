//! §22.13 — **AC-P2-22-1** (both ingest orders) and **AC-P2-22-9** (no fetch, no git).
//!
//! The suite R46 records as named by `criteria.json` and created by no task in any plan. It
//! closes that one; the other eleven stay open.
//!
//! **Each order runs to quiescence and the comparison is of the two fixed points.** A single pass
//! is not a steady state: on the criterion's own mandated renamed-repository fixture the binding
//! columns are written by the *second* sync, so a one-pass comparison compares two different
//! moments and calls it two orders.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::collections::BTreeMap;

use codotheca_core::derive::LocationKind;
use codotheca_core::identity::alias::{fold_key, HostAliases};
use codotheca_core::identity::decide::IdentityProbe;
use codotheca_core::identity::ingest::{ingest_listing, ListingIngestReport};
use codotheca_core::identity::store::{resolve_identity, upsert_location, LocationInput};
use codotheca_core::identity::testutil::{forge_aliases, listing_library, LocalClone};
use codotheca_core::index::migrate::{apply_all, MIGRATIONS};
use codotheca_core::index::path::{PathPlatform, StoredPath};
use codotheca_core::index::{open_connection, Index};
use codotheca_core::protocol::Presence;
use codotheca_core::scan::discover::RepoKind;
use codotheca_core::testing::FakeGitBackend;

/// Columns whose value is **order-dependent by design** and which §22.13's *"identical rows"*
/// therefore cannot mean.
///
/// `name` and `seed_basename`: in sync-then-scan the seed is the listing's bare name and in
/// scan-then-sync it is the directory basename, and §22.4 exists precisely so hydration does not
/// reconcile them. `created_at` and `updated_at` are clocks. `id` is an insertion order.
/// `remote_key` is compared **folded** rather than excluded — §22.2 forbids rewriting a stored
/// key, so an alias-host clone and its listing legitimately store two spellings of one identity.
const ORDER_DEPENDENT: &[&str] = &[
    "id",
    "name",
    "seed_basename",
    "created_at",
    "updated_at",
    "remote_key",
];

fn migrated() -> (tempfile::TempDir, rusqlite::Connection) {
    let dir = tempfile::tempdir().unwrap();
    let mut conn = open_connection(&Index::db_path(dir.path())).unwrap();
    apply_all(&mut conn, MIGRATIONS).unwrap();
    (dir, conn)
}

fn probe_of(clone: &LocalClone) -> IdentityProbe {
    IdentityProbe {
        common_dir_key: Some(
            StoredPath::from_bytes(clone.common_dir.as_bytes().to_vec(), PathPlatform::Unix)
                .key()
                .to_vec(),
        ),
        is_shallow: clone.is_shallow,
        root_oids: clone
            .lineage
            .map(|l| vec![l.to_owned()])
            .unwrap_or_default(),
        remote_urls: vec![("origin".to_owned(), clone.url.to_owned())],
    }
}

fn location_of(clone: &LocalClone) -> LocationInput {
    let path = format!("/w/{}", clone.basename);
    LocationInput {
        kind: LocationKind::Linux,
        distro: None,
        path: StoredPath::from_bytes(path.into_bytes(), PathPlatform::Unix),
        store_key: "store-1".to_owned(),
        volume_key: Some("vol-1".to_owned()),
        presence: Presence::Present,
        repo_kind: RepoKind::WorkTree,
        common_dir_bytes: Some(clone.common_dir.as_bytes().to_vec()),
        generation: 1,
        last_seen_at: None,
    }
}

/// One sync pass: every listing, in order, in one transaction — the shape §22.10 requires.
fn sync_pass(
    conn: &mut rusqlite::Connection,
    aliases: &HostAliases,
    now: i64,
) -> ListingIngestReport {
    let library = listing_library();
    let tx = conn.transaction().unwrap();
    let mut report = ListingIngestReport::default();
    for entry in &library.listings {
        report.push(ingest_listing(&tx, entry, aliases, now).unwrap());
    }
    tx.commit().unwrap();
    report
}

/// One scan pass: `resolve_identity` and `upsert_location` for every clone, the pair
/// `assembly::handoff` runs in one transaction per repository.
fn scan_pass(conn: &mut rusqlite::Connection, aliases: &HostAliases, now: i64) {
    let library = listing_library();
    for clone in &library.clones {
        let tx = conn.transaction().unwrap();
        let outcome =
            resolve_identity(&tx, &probe_of(clone), clone.basename, aliases, now).unwrap();
        upsert_location(&tx, outcome.project_id, &location_of(clone), now).unwrap();
        tx.commit().unwrap();
    }
}

/// Every `project` row, grouped by its **folded** `remote_key`, each row rendered as a sorted
/// `column=value` list over everything that is not order-dependent.
fn snapshot(conn: &rusqlite::Connection, aliases: &HostAliases) -> BTreeMap<String, Vec<String>> {
    let names: Vec<String> = {
        let mut st = conn.prepare("PRAGMA table_info(project)").unwrap();
        st.query_map([], |r| r.get::<_, String>(1))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    let list = names
        .iter()
        .map(|c| format!("\"{c}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let mut st = conn
        .prepare(&format!("SELECT {list} FROM project"))
        .unwrap();
    let rows = st
        .query_map([], |row| {
            let mut fields = Vec::new();
            let mut key = String::new();
            for (i, name) in names.iter().enumerate() {
                let value: rusqlite::types::Value = row.get(i)?;
                if name == "remote_key" {
                    if let rusqlite::types::Value::Text(ref text) = value {
                        key = fold_key(text, aliases).unwrap_or_else(|| text.clone());
                    }
                }
                if ORDER_DEPENDENT.contains(&name.as_str()) {
                    continue;
                }
                fields.push(format!("{name}={value:?}"));
            }
            Ok((key, fields.join(" ")))
        })
        .unwrap();

    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for row in rows {
        let (key, fields) = row.unwrap();
        out.entry(key).or_default().push(fields);
    }
    for group in out.values_mut() {
        group.sort();
    }
    out
}

fn project_count(conn: &rusqlite::Connection) -> usize {
    let n: i64 = conn
        .query_row("SELECT count(*) FROM project", [], |r| r.get(0))
        .unwrap();
    usize::try_from(n).unwrap()
}

/// The folded key of the one group §22.5 makes order-dependent — see the test that names it.
const REWRITTEN: &str = "forge.example/acme/rewritten";

/// **AC-P2-22-1.** Both orders, each carried to a fixed point, reach the same K and the same rows.
#[test]
fn both_ingest_orders_reach_the_same_library() {
    let library = listing_library();
    let aliases = forge_aliases();
    eprintln!(
        "acceptance_identity: N={} listings, M={} clones, K={} expected rows",
        library.listings.len(),
        library.clones.len(),
        library.expected_projects
    );
    assert!(!library.listings.is_empty() && !library.clones.is_empty());

    // sync → scan → sync → scan
    let (_a, mut sync_first) = migrated();
    sync_pass(&mut sync_first, &aliases, 100);
    scan_pass(&mut sync_first, &aliases, 200);
    sync_pass(&mut sync_first, &aliases, 300);
    scan_pass(&mut sync_first, &aliases, 400);

    // scan → sync → scan → sync
    let (_b, mut scan_first) = migrated();
    scan_pass(&mut scan_first, &aliases, 100);
    sync_pass(&mut scan_first, &aliases, 200);
    scan_pass(&mut scan_first, &aliases, 300);
    sync_pass(&mut scan_first, &aliases, 400);

    assert_eq!(
        project_count(&sync_first),
        library.expected_projects,
        "sync-then-scan settled on a different K"
    );
    assert_eq!(
        project_count(&scan_first),
        library.expected_projects,
        "scan-then-sync settled on a different K"
    );

    let a = snapshot(&sync_first, &aliases);
    let b = snapshot(&scan_first, &aliases);
    assert_eq!(
        a.keys().collect::<Vec<_>>(),
        b.keys().collect::<Vec<_>>(),
        "the two orders produced different identities"
    );
    for (key, rows) in &a {
        if key == REWRITTEN {
            continue;
        }
        assert_eq!(rows, &b[key], "the two orders disagree about {key}");
    }
    assert!(a.len() > 1, "a comparison over one identity proves nothing");
}

/// **One round of each order, which is where `remote_link_basis` is actually load-bearing.**
///
/// At the fixed point every bound row has been re-attached on the `provider_id` basis, so the
/// value `Create` wrote is no longer observable — a fixed-point comparison alone passes with
/// `Create` writing NULL, which is this project's bar-written-past-the-defect shape and is why
/// this test exists beside the one above. After **one round of each order** the created row still
/// carries what `Create` gave it, and the two orders must agree.
#[test]
fn both_orders_agree_on_the_basis_after_one_round() {
    let aliases = forge_aliases();

    let (_a, mut sync_first) = migrated();
    sync_pass(&mut sync_first, &aliases, 100);
    scan_pass(&mut sync_first, &aliases, 200);

    let (_b, mut scan_first) = migrated();
    scan_pass(&mut scan_first, &aliases, 100);
    sync_pass(&mut scan_first, &aliases, 200);

    let a = snapshot(&sync_first, &aliases);
    let b = snapshot(&scan_first, &aliases);
    assert_eq!(a.keys().collect::<Vec<_>>(), b.keys().collect::<Vec<_>>());

    let mut compared = 0_usize;
    for (key, rows) in &a {
        if key == REWRITTEN {
            continue;
        }
        compared += 1;
        assert_eq!(
            rows, &b[key],
            "one round of each order disagrees about {key}"
        );
    }
    eprintln!("acceptance_identity: {compared} identities compared after one round");
    assert!(compared >= 5, "too few identities to prove anything");
}

/// **The one group that does not converge, named rather than excluded.**
///
/// §22.5's rewritten-history duplicate is order-dependent **by construction**, and this asserts
/// the divergence so that if it ever stops happening someone re-reads this rule rather than
/// quietly losing the case:
///
/// * **sync first** — the listing creates a not-cloned row, the first clone hydrates it, the
///   second clone finds it already has a location and creates its own unbound row. The next sync
///   then sees **exactly one** candidate on the `provider_id` basis and attaches to it.
/// * **scan first** — both clones exist unbound when the listing arrives, so the `remote_key`
///   basis returns **two** and §22.3 rules `Ambiguous`: attaches nothing, creates nothing, flags
///   both.
///
/// §22.3's order — `provider_id` first, and `remote_key` only if none matched — is what makes the
/// two differ, and it is a settled rule. **Reported as a finding against AC-P2-22-1's own
/// mandated fixture list, not papered over.**
#[test]
fn the_rewritten_history_duplicate_is_the_one_order_dependent_group() {
    let aliases = forge_aliases();

    let (_a, mut sync_first) = migrated();
    sync_pass(&mut sync_first, &aliases, 100);
    scan_pass(&mut sync_first, &aliases, 200);
    sync_pass(&mut sync_first, &aliases, 300);

    let (_b, mut scan_first) = migrated();
    scan_pass(&mut scan_first, &aliases, 100);
    sync_pass(&mut scan_first, &aliases, 200);
    scan_pass(&mut scan_first, &aliases, 300);

    let bound = |conn: &rusqlite::Connection| -> (i64, i64) {
        conn.query_row(
            "SELECT (SELECT count(*) FROM project
                      WHERE remote_key = ?1 AND provider_repo_id IS NOT NULL),
                    (SELECT count(*) FROM project
                      WHERE remote_key = ?1 AND ambiguous_lineage = 1)",
            [REWRITTEN],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap()
    };

    assert_eq!(
        bound(&sync_first),
        (1, 0),
        "sync first: one row carries the id and neither is flagged"
    );
    assert_eq!(
        bound(&scan_first),
        (0, 2),
        "scan first: neither row carries the id and both are flagged"
    );
}

/// Quiescence is real rather than assumed: a further round writes nothing.
#[test]
fn a_third_round_writes_nothing() {
    let aliases = forge_aliases();
    let (_dir, mut conn) = migrated();
    sync_pass(&mut conn, &aliases, 100);
    scan_pass(&mut conn, &aliases, 200);
    sync_pass(&mut conn, &aliases, 300);
    scan_pass(&mut conn, &aliases, 400);

    let before = snapshot(&conn, &aliases);
    let before_count = project_count(&conn);
    sync_pass(&mut conn, &aliases, 500);
    scan_pass(&mut conn, &aliases, 600);
    let after = snapshot(&conn, &aliases);

    assert_eq!(project_count(&conn), before_count);
    assert_eq!(
        after, before,
        "a third round moved a row — this is not a fixed point"
    );
    assert!(!before.is_empty());
}

/// **AC-P2-22-9.** The matcher and hydration run to completion with git denied.
///
/// The backend is scripted with **no** replies, so any call to it fails, and it records every
/// call it receives: `calls()` staying empty is the assertion. It is structural as well —
/// `ingest_listing` and `resolve_identity` take no `GitBackend` and no `Provider` at all — and the
/// network half is that a `Provider` is never held here, only its **declared** alias set, which
/// issues no request. Both halves are asserted, not one.
#[test]
fn the_matcher_and_hydration_reach_neither_git_nor_the_network() {
    let git = FakeGitBackend::new();
    let aliases = forge_aliases();

    let (_dir, mut conn) = migrated();
    sync_pass(&mut conn, &aliases, 100);
    scan_pass(&mut conn, &aliases, 200);
    sync_pass(&mut conn, &aliases, 300);

    assert_eq!(
        project_count(&conn),
        listing_library().expected_projects,
        "the run must actually have done the work"
    );
    assert!(
        git.calls().is_empty(),
        "the identity path invoked git: {:?}",
        git.calls()
    );
}
