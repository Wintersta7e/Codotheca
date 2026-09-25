//! `#[cfg(test)]` fixtures for the identity module.
//!
//! Every helper here is a **fixture**, never a production writer. The distinction matters most
//! for [`insert_location`], whose production counterpart is `super::store::upsert_location`:
//! the two differ only by module, so read the path before reaching for either.
#![allow(
    // Fixtures behind the default-off `testkit` feature; nothing outside a test binary can call
    // them, so the documented-public-API lints are noise here.
    clippy::missing_panics_doc,
    clippy::must_use_candidate,
    // `expect` and `panic` are denied crate-wide because a panic kills the process the shell
    // supervises. That reasoning does not reach here — the same exemption `crate::testing::index`
    // carries, for the same reason: a fixture that cannot build its own database must abort
    // loudly rather than hand a test a half-made index to assert against.
    clippy::expect_used,
    clippy::panic
)]

use rusqlite::{params, Connection};

use crate::accounts::keychain::SecretToken;
use crate::identity::alias::HostAliases;
use crate::index::migrate::{apply_all, MIGRATIONS};
use crate::provider::listing::{OrgListing, Page, RepoListing, Viewer};
use crate::provider::{CiRunsRead, Observed, Provider, ProviderResult, RepoFactsRead};

/// A forge that declares an alias set and issues no request.
///
/// §22.2's fold is provider-declared, so a test that needs a [`HostAliases`] needs a `Provider` —
/// and this one is a fixture rather than an adapter: every request method is unreachable.
#[derive(Debug)]
pub struct DeclaringForge;

impl Provider for DeclaringForge {
    fn viewer(&self, _t: &SecretToken) -> ProviderResult<Observed<Viewer>> {
        unreachable!("a fixture forge issues no request")
    }
    fn list_orgs(
        &self,
        _t: &SecretToken,
        _cur: Option<&str>,
    ) -> ProviderResult<Observed<Page<OrgListing>>> {
        unreachable!("a fixture forge issues no request")
    }
    fn list_repos(
        &self,
        _t: &SecretToken,
        _cur: Option<&str>,
    ) -> ProviderResult<Observed<Page<RepoListing>>> {
        unreachable!("a fixture forge issues no request")
    }
    fn lookup_repo(
        &self,
        _t: &SecretToken,
        _owner: &str,
        _name: &str,
    ) -> ProviderResult<Observed<Option<RepoListing>>> {
        unreachable!("a fixture forge issues no request")
    }
    fn repo_facts(
        &self,
        _t: &SecretToken,
        _owner: &str,
        _name: &str,
        _etag: Option<&str>,
    ) -> ProviderResult<Observed<RepoFactsRead>> {
        unreachable!("a fixture forge issues no request")
    }
    fn ci_runs(
        &self,
        _t: &SecretToken,
        _owner: &str,
        _name: &str,
        _etag: Option<&str>,
    ) -> ProviderResult<Observed<CiRunsRead>> {
        unreachable!("a fixture forge issues no request")
    }
    fn advisories(
        &self,
        _ecosystem: crate::protocol::Ecosystem,
        _affects: &[crate::provider::PackageVersion],
        _cur: Option<&str>,
    ) -> ProviderResult<Observed<Page<crate::provider::AdvisoryPayload>>> {
        unreachable!("a fixture forge issues no request")
    }
    fn canonical_host(&self) -> &'static str {
        "forge.example"
    }
    fn host_aliases(&self) -> &[&str] {
        &["forge.example", "www.forge.example", "ssh.forge.example"]
    }
}

/// The alias set every identity fixture in this crate compares against.
pub fn forge_aliases() -> HostAliases {
    HostAliases::from_provider(&DeclaringForge)
}

/// One local clone, as a **scan** would probe it.
///
/// No git and no disk: `resolve_identity` takes an `IdentityProbe`, so a fixture that generated
/// a real corpus would build repositories nothing under test ever reads.
#[derive(Debug, Clone, Copy)]
pub struct LocalClone {
    /// The directory basename a scan would pass as the seed for a *created* row.
    pub basename: &'static str,
    /// The URL `git config` would report for `origin`.
    pub url: &'static str,
    /// `None` for a shallow or unborn clone, which is §1.1's no-lineage case.
    pub lineage: Option<&'static str>,
    /// Whether the probe reports a shallow clone.
    pub is_shallow: bool,
    /// `git rev-parse --git-common-dir`, folded. A rescan of the same path is `AttachDefinitive`
    /// because of this value, which is what makes a scan pass idempotent.
    pub common_dir: &'static str,
}

/// §22.13's fixture library: N listings, M local clones, and the K rows both ingest orders must
/// reach.
#[derive(Debug, Clone)]
pub struct ListingLibrary {
    /// The forge listings a sync would ingest.
    pub listings: Vec<RepoListing>,
    /// The local clones a scan would probe.
    pub clones: Vec<LocalClone>,
    /// The `project` row count both orders settle on.
    pub expected_projects: usize,
}

fn listing(provider_repo_id: &str, owner: &str, name: &str, is_fork: bool) -> RepoListing {
    RepoListing {
        provider: "github",
        provider_repo_id: provider_repo_id.to_owned(),
        clone_url: format!("https://forge.example/{owner}/{name}.git"),
        owner: owner.to_owned(),
        name: name.to_owned(),
        can_push: Some(true),
        is_fork,
        fork_parent_clone_url: None,
        is_archived: false,
        is_private: false,
        in_org: None,
    }
}

/// The library §22.13's first criterion describes, with every case it names.
///
/// | Fixture | What it exercises |
/// |---|---|
/// | `widget` | the ordinary case: one listing, one clone, one row |
/// | `gadget` / `gadget-old` | a **renamed** repository — the clone's stored key is the old path |
/// | `tool` | an **alias-host** clone: stored `ssh.…`, listed on the canonical host |
/// | `thing` | a **shallow** clone of a listed repository — no lineage at all |
/// | `upstream` + `mine/upstream` | a **fork with its upstream**, sharing one lineage |
/// | `rewritten` ×2 | a **rewritten-history duplicate**: one key, two lineages (§22.5) |
/// | `offsite` | an **undeclared**-alias-host clone, which folds to itself and matches nothing |
#[must_use]
pub fn listing_library() -> ListingLibrary {
    ListingLibrary {
        listings: vec![
            listing("1", "acme", "widget", false),
            listing("2", "acme", "gadget", false),
            listing("3", "acme", "tool", false),
            listing("4", "acme", "thing", false),
            listing("5", "acme", "upstream", false),
            listing("6", "mine", "upstream", true),
            listing("7", "acme", "rewritten", false),
        ],
        clones: vec![
            LocalClone {
                basename: "widget",
                url: "https://forge.example/acme/widget.git",
                lineage: Some("root-widget"),
                is_shallow: false,
                common_dir: "/w/widget/.git",
            },
            LocalClone {
                basename: "gadget",
                url: "https://forge.example/acme/gadget-old.git",
                lineage: Some("root-gadget"),
                is_shallow: false,
                common_dir: "/w/gadget/.git",
            },
            LocalClone {
                basename: "tool",
                url: "git@ssh.forge.example:acme/tool.git",
                lineage: Some("root-tool"),
                is_shallow: false,
                common_dir: "/w/tool/.git",
            },
            LocalClone {
                basename: "thing",
                url: "https://forge.example/acme/thing.git",
                lineage: None,
                is_shallow: true,
                common_dir: "/w/thing/.git",
            },
            LocalClone {
                basename: "upstream",
                url: "https://forge.example/acme/upstream.git",
                lineage: Some("root-upstream"),
                is_shallow: false,
                common_dir: "/w/upstream/.git",
            },
            LocalClone {
                basename: "myfork",
                url: "https://forge.example/mine/upstream.git",
                lineage: Some("root-upstream"),
                is_shallow: false,
                common_dir: "/w/myfork/.git",
            },
            LocalClone {
                basename: "rewritten-a",
                url: "https://forge.example/acme/rewritten.git",
                lineage: Some("root-rewritten-a"),
                is_shallow: false,
                common_dir: "/w/rewritten-a/.git",
            },
            LocalClone {
                basename: "rewritten-b",
                url: "https://forge.example/acme/rewritten.git",
                lineage: Some("root-rewritten-b"),
                is_shallow: false,
                common_dir: "/w/rewritten-b/.git",
            },
            LocalClone {
                basename: "offsite",
                url: "git@forge-work:acme/offsite.git",
                lineage: Some("root-offsite"),
                is_shallow: false,
                common_dir: "/w/offsite/.git",
            },
        ],
        // Seven listings and nine clones, of which six clones fold onto a listing and three do
        // not: `gadget-old` (the pre-rename key), `rewritten-b` (the second history) and
        // `offsite` (an undeclared host). 7 + 3 = 10.
        expected_projects: 10,
    }
}

/// An in-memory index with **the migrations the shipped build applies**, in order.
///
/// It goes through `index::migrate::MIGRATIONS` rather than reading `core/migrations/` off
/// disk. A directory walk would apply a `.sql` file that nothing has registered in that slice —
/// so a column this module needs could be present in every identity test and absent from every
/// real database, and the whole suite would agree with a schema the product does not have.
pub fn open_test_index() -> Connection {
    let mut conn = Connection::open_in_memory().expect("in-memory sqlite");
    // `foreign_keys` is a no-op inside a transaction, so it is set before the migrations run.
    conn.execute_batch(
        "PRAGMA journal_mode=MEMORY; PRAGMA foreign_keys=ON; PRAGMA synchronous=OFF;",
    )
    .expect("pragmas");
    apply_all(&mut conn, MIGRATIONS).expect("migrations apply");
    conn
}

/// The fields an identity test ever varies. Everything else takes the DDL's default.
#[derive(Debug, Clone, Copy)]
pub struct NewProject {
    /// The project's name, also written as its `seed_basename`.
    pub name: &'static str,
    /// Its lineage, or `None` for a project with no history read.
    pub lineage_key: Option<&'static str>,
    /// Its canonical remote, or `None` for a remoteless project.
    pub remote_key: Option<&'static str>,
    /// Its `created_at` and `updated_at`, in Unix seconds — what orders candidates.
    pub created_at: i64,
}

/// A `project` row with the identity columns set and nothing claimed that has not been observed.
///
/// **`last_touched_at` is deliberately not written.** §5.1 owns it and `0001` says NULL until a
/// scan job produces one; stamping `created_at` there would make every test here agree that a
/// never-scanned project has a touch time, which is the "unknown rendered as a value" defect.
pub fn insert_project(conn: &Connection, p: NewProject) -> i64 {
    conn.execute(
        "INSERT INTO project (lineage_key, remote_key, name, seed_basename, created_at,
                              updated_at)
         VALUES (?1, ?2, ?3, ?3, ?4, ?4)",
        params![p.lineage_key, p.remote_key, p.name, p.created_at],
    )
    .expect("insert project");
    conn.last_insert_rowid()
}

/// A fixed-shape `location` row, for a test that needs one to exist.
///
/// **This is a fixture and never the production writer.** Every `location` row the app writes
/// goes through [`super::store::upsert_location`], which takes a `LocationInput`, is idempotent
/// on `(kind, distro, path_key)`, and derives `common_dir_key` from raw bytes. This builder
/// hard-codes a Linux location on a present store because no test here varies those;
/// hard-coding them in *production* is the defect R1 exists to remove, so do not reach for this
/// outside `#[cfg(test)]`.
pub fn insert_location(
    conn: &Connection,
    project_id: i64,
    path: &str,
    common_dir_key: Option<&[u8]>,
) -> i64 {
    conn.execute(
        "INSERT INTO location
            (project_id, kind, distro, path_bytes, path_key, path_display, volume_key, store_key,
             presence, scan_generation, common_dir_key, repo_kind)
         VALUES (?1, 'linux', '', ?2, ?2, ?3, 'vol', 'store', 'present', 1, ?4, 'worktree')",
        params![project_id, path.as_bytes(), path, common_dir_key],
    )
    .expect("insert location");
    conn.last_insert_rowid()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    fn table_sql(conn: &rusqlite::Connection, table: &str) -> String {
        conn.query_row(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name=?1",
            rusqlite::params![table],
            |r| r.get::<_, String>(0),
        )
        .unwrap()
    }

    fn has_column(conn: &rusqlite::Connection, table: &str, column: &str) -> bool {
        let mut st = conn
            .prepare(&format!("PRAGMA table_info({table})"))
            .unwrap();
        let mut rows = st.query([]).unwrap();
        while let Some(r) = rows.next().unwrap() {
            if r.get::<_, String>(1).unwrap() == column {
                return true;
            }
        }
        false
    }

    #[test]
    fn the_identity_columns_are_all_present_on_a_migrated_index() {
        let conn = super::open_test_index();
        assert!(has_column(&conn, "project", "association_kind"));
        assert!(has_column(&conn, "project", "merged_into"));
        assert!(has_column(&conn, "location", "common_dir_key"));
        assert!(has_column(&conn, "launch_target", "disabled"));
    }

    #[test]
    fn project_ids_are_autoincrement_so_a_tombstoned_id_can_never_be_reused() {
        // §1.5 requires `id INTEGER PRIMARY KEY AUTOINCREMENT`, so a rowid can never be reused
        // and alias a future project.
        let conn = super::open_test_index();
        assert!(table_sql(&conn, "project")
            .to_uppercase()
            .contains("AUTOINCREMENT"));
    }

    #[test]
    fn the_fixture_builders_produce_rows_that_satisfy_every_not_null_column() {
        let conn = super::open_test_index();
        let p = super::insert_project(
            &conn,
            super::NewProject {
                name: "alpha",
                lineage_key: Some("aa"),
                remote_key: None,
                created_at: 100,
            },
        );
        let l = super::insert_location(&conn, p, "/w/alpha", Some(b"/w/alpha/.git"));
        assert!(p > 0 && l > 0);
    }

    /// A project nothing has scanned has no `last_touched_at`. §5.1 owns the column and
    /// `0001`'s own comment says NULL until a scan job produces one; a fixture that stamped
    /// `created_at` there would teach every later test that the value is always present.
    #[test]
    fn a_fixture_project_has_no_last_touched_at() {
        let conn = super::open_test_index();
        let p = super::insert_project(
            &conn,
            super::NewProject {
                name: "alpha",
                lineage_key: None,
                remote_key: None,
                created_at: 100,
            },
        );
        let touched: Option<i64> = conn
            .query_row(
                "SELECT last_touched_at FROM project WHERE id=?1",
                rusqlite::params![p],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(touched, None);
    }
}
