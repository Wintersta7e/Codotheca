//! §22.5, §22.6, §22.8 and §22.10 — what one listing entry writes, and what it must not.
//!
//! Every "touches nothing else" assertion below is a **diff over the whole row**, not a handful
//! of equalities: a per-column check passes while a column nobody thought of moves.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use codotheca_core::accounts::keychain::SecretToken;
use codotheca_core::identity::alias::HostAliases;
use codotheca_core::identity::binding::{load_binding, write_binding, RemoteBinding};
use codotheca_core::identity::ingest::{
    ingest_listing, ListingIngest, ListingIngestReport, Suppression,
};
use codotheca_core::identity::match_listing::ListingMatch;
use codotheca_core::identity::store::ambiguous_group;
use codotheca_core::index::migrate::{apply_all, MIGRATIONS};
use codotheca_core::index::{open_connection, Index};
use codotheca_core::protocol::RemoteLinkBasis;
use codotheca_core::provider::listing::{OrgListing, Page, RepoListing, Viewer};
use codotheca_core::provider::{Observed, Provider, ProviderResult};

#[derive(Debug)]
struct DeclaringForge;

impl Provider for DeclaringForge {
    fn viewer(&self, _t: &SecretToken) -> ProviderResult<Observed<Viewer>> {
        unreachable!("ingest issues no request")
    }
    fn list_orgs(
        &self,
        _t: &SecretToken,
        _cur: Option<&str>,
    ) -> ProviderResult<Observed<Page<OrgListing>>> {
        unreachable!("ingest issues no request")
    }
    fn list_repos(
        &self,
        _t: &SecretToken,
        _cur: Option<&str>,
    ) -> ProviderResult<Observed<Page<RepoListing>>> {
        unreachable!("ingest issues no request")
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

/// A row a **scan** produced: a lineage, a remote key, and no binding at all.
fn scanned_project(
    conn: &rusqlite::Connection,
    name: &str,
    lineage: Option<&str>,
    remote_key: &str,
    created_at: i64,
) -> i64 {
    conn.execute(
        "INSERT INTO project (name, seed_basename, lineage_key, remote_key, created_at, updated_at)
         VALUES (?1, ?1, ?2, ?3, ?4, ?4)",
        rusqlite::params![name, lineage, remote_key, created_at],
    )
    .unwrap();
    conn.last_insert_rowid()
}

fn location(conn: &rusqlite::Connection, project_id: i64, path: &str) {
    conn.execute(
        "INSERT INTO location (project_id, kind, distro, path_bytes, path_key, path_display,
                               store_key, presence, repo_kind, scan_generation)
         VALUES (?1, 'linux', '', ?2, ?2, ?3, 'store', 'present', 'worktree', 1)",
        rusqlite::params![project_id, path.as_bytes(), path],
    )
    .unwrap();
}

/// Every column of one `project` row, as `(name, debug-formatted value)`.
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

/// The columns that differ between two whole-row snapshots, sorted so the assertion is about the
/// set rather than about `PRAGMA table_info`'s order.
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

const KEY: &str = "forge.example/acme/widget";

// ---------------------------------------------------------------------------------------------
// The binding
// ---------------------------------------------------------------------------------------------

#[test]
fn a_binding_round_trips_and_moves_only_updated_at() {
    let (_dir, mut conn) = migrated();
    let project = scanned_project(&conn, "widget", Some("lin"), KEY, 100);

    let tx = conn.transaction().unwrap();
    assert_eq!(
        load_binding(&tx, project).unwrap(),
        None,
        "not yet resolved is None, never a binding with an empty string"
    );
    let before = row_snapshot(&tx, project);
    let binding = RemoteBinding {
        provider: "github".to_owned(),
        provider_repo_id: "42".to_owned(),
        remote_link_basis: Some(RemoteLinkBasis::RemoteKey),
    };
    write_binding(&tx, project, &binding, 500).unwrap();
    let after = row_snapshot(&tx, project);
    assert_eq!(load_binding(&tx, project).unwrap(), Some(binding.clone()));
    tx.commit().unwrap();

    assert_eq!(
        moved(&before, &after),
        vec![
            "provider".to_owned(),
            "provider_repo_id".to_owned(),
            "remote_link_basis".to_owned(),
            "updated_at".to_owned(),
        ],
        "the binding's three columns and the clock, and nothing else"
    );
    assert_eq!(binding.key(), ("github", "42"));
}

#[test]
fn a_half_written_binding_is_not_a_binding() {
    let (_dir, mut conn) = migrated();
    let project = scanned_project(&conn, "widget", Some("lin"), KEY, 100);
    conn.execute(
        "UPDATE project SET provider = 'github' WHERE id = ?1",
        [project],
    )
    .unwrap();
    let tx = conn.transaction().unwrap();
    assert_eq!(load_binding(&tx, project).unwrap(), None);
    tx.commit().unwrap();
}

// ---------------------------------------------------------------------------------------------
// Ingest
// ---------------------------------------------------------------------------------------------

/// **AC-P2-22-6.** Two live projects on one folded key with **different** lineages, and a listing
/// equal to both. The listing attaches to neither and creates nothing; both are flagged and both
/// appear in the group naming each other.
#[test]
fn two_exact_matches_attach_to_nothing_and_create_nothing() {
    let (_dir, mut conn) = migrated();
    let a = scanned_project(&conn, "widget", Some("lineage-a"), KEY, 100);
    let b = scanned_project(&conn, "widget-rewritten", Some("lineage-b"), KEY, 101);
    let before = project_count(&conn);

    let tx = conn.transaction().unwrap();
    let ingest = ingest_listing(&tx, &listing("42", "acme", "widget"), &aliases(), 500).unwrap();
    assert_eq!(
        ingest.outcome,
        ListingMatch::Ambiguous {
            candidates: vec![a, b]
        }
    );
    assert_eq!(ingest.project_id, None);
    assert!(!ingest.created);

    let group = ambiguous_group(&tx, &aliases()).unwrap();
    tx.commit().unwrap();

    assert_eq!(project_count(&conn), before, "nothing was created");
    for id in [a, b] {
        assert_eq!(
            load_binding(&conn.transaction().unwrap(), id).unwrap(),
            None,
            "neither row's binding moved"
        );
    }
    assert_eq!(
        group.iter().map(|r| r.project_id).collect::<Vec<_>>(),
        vec![a, b],
        "§22.5's pair must both appear"
    );
    assert_eq!(
        group[0].candidate_names,
        vec!["widget-rewritten".to_owned()]
    );
    assert_eq!(group[1].candidate_names, vec!["widget".to_owned()]);
}

/// The lineage arm's own rule, which the second basis must not disturb: a remoteless subject left
/// with **one** candidate yields an **empty** group, because that case is `AttachInferred`'s and
/// not an ambiguity.
#[test]
fn the_lineage_arm_still_drops_a_subject_with_one_candidate() {
    let (_dir, mut conn) = migrated();
    let subject = scanned_project(&conn, "local", Some("lineage-a"), "", 100);
    conn.execute(
        "UPDATE project SET remote_key = NULL, ambiguous_lineage = 1 WHERE id = ?1",
        [subject],
    )
    .unwrap();
    scanned_project(&conn, "one-candidate", Some("lineage-a"), KEY, 101);

    let tx = conn.transaction().unwrap();
    let group = ambiguous_group(&tx, &aliases()).unwrap();
    tx.commit().unwrap();
    assert!(group.is_empty(), "{group:?}");
}

/// **AC-P2-22-7**, the core half. A suppression that produces no report fails the criterion.
#[test]
fn a_suppressed_listing_names_the_project_that_blocked_it() {
    let (_dir, mut conn) = migrated();
    let blocker = scanned_project(
        &conn,
        "widget",
        Some("lin"),
        "other.example/acme/widget",
        100,
    );
    let before = project_count(&conn);

    let tx = conn.transaction().unwrap();
    location(&tx, blocker, "/w/widget");
    let ingest = ingest_listing(&tx, &listing("42", "acme", "widget"), &aliases(), 500).unwrap();
    tx.commit().unwrap();

    assert_eq!(
        ingest.outcome,
        ListingMatch::Suppress {
            blocked_by: blocker
        }
    );
    assert_eq!(project_count(&conn), before, "suppression writes nothing");

    let mut report = ListingIngestReport::default();
    report.push(ingest);
    assert_eq!(
        report.suppressed,
        vec![Suppression {
            listing_key: KEY.to_owned(),
            blocked_by: blocker,
        }]
    );
    assert_eq!(report.listed, 1);
    assert_eq!(report.admitted, 0);
}

#[test]
fn a_created_project_seeds_on_the_bare_name() {
    let (_dir, mut conn) = migrated();
    let tx = conn.transaction().unwrap();
    let ingest = ingest_listing(&tx, &listing("42", "acme", "widget"), &aliases(), 500).unwrap();
    tx.commit().unwrap();

    assert!(ingest.created);
    let id = ingest.project_id.unwrap();
    let (name, seed, key, basis, lineage, kind, shallow): (
        String,
        String,
        String,
        String,
        Option<String>,
        Option<String>,
        i64,
    ) = conn
        .query_row(
            "SELECT name, seed_basename, remote_key, remote_link_basis, lineage_key,
                    association_kind, is_shallow
               FROM project WHERE id = ?1",
            [id],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                ))
            },
        )
        .unwrap();

    assert_eq!(name, "widget", "the BARE name, never acme/widget");
    assert_eq!(seed, "widget");
    assert_eq!(key, KEY, "the unfolded canonical key is what is stored");
    assert_eq!(basis, "remote_key");
    assert_eq!(lineage, None);
    assert_eq!(kind, None);
    assert_eq!(shallow, 0, "the DDL default, not an observation");

    let locations: i64 = conn
        .query_row(
            "SELECT count(*) FROM location WHERE project_id = ?1",
            [id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(locations, 0, "a listing creates no location row");
}

/// §22.11 requires this of §20 and p2-20 can land only half of it: `admit()` collapses two
/// accounts' listings before anything reaches a row, and this is the other half.
#[test]
fn two_accounts_listing_one_repository_produce_one_row() {
    let (_dir, mut conn) = migrated();
    let tx = conn.transaction().unwrap();
    let first = ingest_listing(&tx, &listing("42", "acme", "widget"), &aliases(), 500).unwrap();
    let second = ingest_listing(&tx, &listing("42", "acme", "widget"), &aliases(), 501).unwrap();
    tx.commit().unwrap();

    assert!(first.created);
    assert!(!second.created);
    assert_eq!(
        second.outcome,
        ListingMatch::Attach {
            project_id: first.project_id.unwrap(),
            basis: RemoteLinkBasis::ProviderId,
        },
        "the second listing matches the id the first wrote"
    );
    assert_eq!(
        project_count(&conn),
        1,
        "the index is not what enforces this"
    );
}

/// §22.8. A forge that stops calling a repository a fork does not make the local history stop
/// being one.
#[test]
fn is_fork_is_set_and_never_cleared() {
    let (_dir, mut conn) = migrated();
    let project = scanned_project(&conn, "widget", Some("lin"), KEY, 100);

    let tx = conn.transaction().unwrap();
    let forked = RepoListing {
        is_fork: true,
        ..listing("42", "acme", "widget")
    };
    ingest_listing(&tx, &forked, &aliases(), 500).unwrap();
    let after_fork: i64 = tx
        .query_row(
            "SELECT is_fork FROM project WHERE id = ?1",
            [project],
            |r| r.get(0),
        )
        .unwrap();
    ingest_listing(&tx, &listing("42", "acme", "widget"), &aliases(), 501).unwrap();
    let after_not_fork: i64 = tx
        .query_row(
            "SELECT is_fork FROM project WHERE id = ?1",
            [project],
            |r| r.get(0),
        )
        .unwrap();
    tx.commit().unwrap();

    assert_eq!(after_fork, 1);
    assert_eq!(after_not_fork, 1, "a listing never clears the flag");
}

#[test]
fn a_listing_never_rewrites_remote_key_or_touches_association_kind() {
    let (_dir, mut conn) = migrated();
    // The clone was taken over an alias host, so the stored key is the alias spelling and the
    // listing's is the canonical one. Rewriting it would make the column disagree with the
    // repository on disk, and it is the git-derived xp_events key's component (§1.7).
    let project = scanned_project(
        &conn,
        "widget",
        Some("lin"),
        "ssh.forge.example/acme/widget",
        100,
    );
    conn.execute(
        "UPDATE project SET association_kind = 'inferred' WHERE id = ?1",
        [project],
    )
    .unwrap();

    let tx = conn.transaction().unwrap();
    let before = row_snapshot(&tx, project);
    ingest_listing(&tx, &listing("42", "acme", "widget"), &aliases(), 500).unwrap();
    let after = row_snapshot(&tx, project);
    tx.commit().unwrap();

    let touched = moved(&before, &after);
    assert_eq!(
        touched,
        vec![
            "provider".to_owned(),
            "provider_repo_id".to_owned(),
            "remote_link_basis".to_owned(),
            "updated_at".to_owned(),
        ],
        "an attach moved a column outside the binding"
    );
}

/// The ruling this plan records rather than infers: both ingest orders must agree on the basis,
/// or AC-P2-22-1 is unpassable. **If this test is deleted the ruling silently stops holding.**
#[test]
fn both_orders_agree_on_the_basis() {
    // sync then scan: the listing creates the row.
    let (_a, mut sync_first) = migrated();
    let tx = sync_first.transaction().unwrap();
    let created = ingest_listing(&tx, &listing("42", "acme", "widget"), &aliases(), 500).unwrap();
    tx.commit().unwrap();

    // scan then sync: a scan made the row and the listing attaches to it.
    let (_b, mut scan_first) = migrated();
    let scanned = scanned_project(&scan_first, "widget", Some("lin"), KEY, 100);
    let tx = scan_first.transaction().unwrap();
    ingest_listing(&tx, &listing("42", "acme", "widget"), &aliases(), 500).unwrap();
    tx.commit().unwrap();

    let basis_of = |conn: &rusqlite::Connection, id: i64| -> String {
        conn.query_row(
            "SELECT remote_link_basis FROM project WHERE id = ?1",
            [id],
            |r| r.get(0),
        )
        .unwrap()
    };
    assert_eq!(
        basis_of(&sync_first, created.project_id.unwrap()),
        "remote_key"
    );
    assert_eq!(basis_of(&scan_first, scanned), "remote_key");
}

#[test]
fn a_listing_whose_clone_url_names_no_repository_is_reported_rather_than_dropped() {
    let (_dir, mut conn) = migrated();
    let tx = conn.transaction().unwrap();
    let malformed = RepoListing {
        clone_url: "/srv/git/widget.git".to_owned(),
        ..listing("42", "acme", "widget")
    };
    let failed = ingest_listing(&tx, &malformed, &aliases(), 500);
    tx.commit().unwrap();
    assert!(
        matches!(
            failed,
            Err(codotheca_core::identity::IdentityError::ListingNotCanonical { .. })
        ),
        "{failed:?}"
    );
    assert_eq!(project_count(&conn), 0);
}

#[test]
fn the_report_counts_entries_and_names_every_suppression() {
    let mut report = ListingIngestReport::default();
    report.push(ListingIngest {
        outcome: ListingMatch::Create,
        project_id: Some(1),
        created: true,
        listing_key: "forge.example/a/one".to_owned(),
    });
    report.push(ListingIngest {
        outcome: ListingMatch::Ambiguous {
            candidates: vec![2, 3],
        },
        project_id: None,
        created: false,
        listing_key: "forge.example/a/two".to_owned(),
    });
    report.push(ListingIngest {
        outcome: ListingMatch::Ambiguous {
            candidates: vec![3, 4],
        },
        project_id: None,
        created: false,
        listing_key: "forge.example/a/three".to_owned(),
    });
    for key in ["forge.example/a/four", "forge.example/a/five"] {
        report.push(ListingIngest {
            outcome: ListingMatch::Suppress { blocked_by: 9 },
            project_id: None,
            created: false,
            listing_key: key.to_owned(),
        });
    }

    assert_eq!(report.listed, 5);
    assert_eq!(report.admitted, 1);
    assert_eq!(
        report.ambiguous,
        vec![2, 3, 4],
        "the projects, deduplicated and in first-seen order"
    );
    assert_eq!(
        report.suppressed.len(),
        2,
        "one project blocking two entries reads as two — never deduplicated"
    );
    assert!(report.suppressed.iter().all(|s| s.blocked_by == 9));
}
