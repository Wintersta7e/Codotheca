//! The three structural gates §22 asks for, each of which a comment cannot satisfy.
//!
//! **(a) `decide` is untouched** — AC-P2-22-12.
//! **(b) One canonicaliser** — AC-P2-22-10.
//! **(c) No merge** — AC-P2-22-3.
//!
//! Each prints the count of what it scanned. **A gate whose passing run scans zero files is a
//! failing gate.**

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use codotheca_core::identity::decide::IdentityDecision;
use codotheca_core::identity::AssociationKind;
use codotheca_core::index::migrate::{apply_all, MIGRATIONS};
use codotheca_core::index::{open_connection, Index};

const SCHEMA_JSON: &str = include_str!("../../protocol/schema/protocol.json");

fn identity_sources() -> Vec<(String, String)> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("identity");
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "rs") {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            out.push((name, std::fs::read_to_string(&path).unwrap()));
        }
    }
    out.sort();
    assert!(
        out.len() >= 15,
        "expected the whole identity module, found {}",
        out.len()
    );
    out
}

/// Line comments and doc comments removed, so a rule about **code** is not satisfied or violated
/// by prose *about* the code. Grepping a declaration matches the paragraph describing it, and
/// that has produced a wrong ruling here twice.
fn code_only(source: &str) -> String {
    source
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn migrated() -> (tempfile::TempDir, rusqlite::Connection) {
    let dir = tempfile::tempdir().unwrap();
    let mut conn = open_connection(&Index::db_path(dir.path())).unwrap();
    apply_all(&mut conn, MIGRATIONS).unwrap();
    (dir, conn)
}

// ---------------------------------------------------------------------------------------------
// (a) `decide` is untouched — AC-P2-22-12
// ---------------------------------------------------------------------------------------------

/// `AssociationKind` gains no fifth variant, and the four it has are accepted by **both** DDL
/// CHECKs against a real migrated database.
///
/// The variant list is read off `protocol/schema/protocol.json`, which is where R31 puts it: the
/// generated Rust and the generated TypeScript are both produced from that file and `gen:check`
/// fails if either drifts, so this compares the columns against the one declaration rather than
/// against a third hand-written copy.
#[test]
fn association_kind_gains_no_variant_and_both_columns_accept_all_four() {
    let doc: serde_json::Value = serde_json::from_str(SCHEMA_JSON).unwrap();
    let declared: Vec<String> = doc["types"]["AssociationKind"]["variants"]
        .as_array()
        .expect("AssociationKind is declared in the schema")
        .iter()
        .map(|v| v.as_str().unwrap().to_owned())
        .collect();
    assert_eq!(
        declared,
        vec![
            "definitive".to_owned(),
            "strong".to_owned(),
            "inferred".to_owned(),
            "manual".to_owned()
        ],
        "§22 declares no new association kind"
    );

    let (_dir, conn) = migrated();
    conn.execute(
        "INSERT INTO project (id, name, seed_basename, created_at, updated_at)
         VALUES (1, 'a', 'a', 1, 1), (2, 'b', 'b', 1, 1)",
        [],
    )
    .unwrap();
    for kind in [
        AssociationKind::Definitive,
        AssociationKind::Strong,
        AssociationKind::Inferred,
        AssociationKind::Manual,
    ] {
        assert!(
            declared.iter().any(|d| d == kind.as_str()),
            "{} is emitted by the core and not declared by the schema",
            kind.as_str()
        );
        conn.execute(
            "UPDATE project SET association_kind = ?1 WHERE id = 1",
            [kind.as_str()],
        )
        .unwrap_or_else(|e| panic!("project.association_kind refused {}: {e}", kind.as_str()));
        conn.execute(
            "INSERT INTO merge_record (survivor_project_id, absorbed_project_id, merged_at,
                                       association_kind, evidence_json, absorbed_json)
             VALUES (1, 2, 1, ?1, '{}', '{}')",
            [kind.as_str()],
        )
        .unwrap_or_else(|e| {
            panic!(
                "merge_record.association_kind refused {}: {e}",
                kind.as_str()
            )
        });
    }
    eprintln!(
        "identity_boundaries: {} association kinds accepted by both columns",
        declared.len()
    );
}

/// `IdentityDecision` gains no variant. **The match below has no wildcard arm**, so a seventh
/// outcome is a compile error rather than a review finding — and `match_listing`'s four outcomes
/// live on their own type beside it, which is what "a second pure function beside `decide`" means
/// structurally.
#[test]
fn identity_decision_still_has_exactly_six_outcomes() {
    fn name(decision: &IdentityDecision) -> &'static str {
        match decision {
            IdentityDecision::AttachDefinitive { .. } => "attach_definitive",
            IdentityDecision::AttachStrong { .. } => "attach_strong",
            IdentityDecision::AttachInferred { .. } => "attach_inferred",
            IdentityDecision::NewFork { .. } => "new_fork",
            IdentityDecision::NewAmbiguous { .. } => "new_ambiguous",
            IdentityDecision::New => "new",
        }
    }
    let all = [
        IdentityDecision::AttachDefinitive { project_id: 1 },
        IdentityDecision::AttachStrong { project_id: 1 },
        IdentityDecision::AttachInferred { project_id: 1 },
        IdentityDecision::NewFork { related: vec![1] },
        IdentityDecision::NewAmbiguous {
            candidates: vec![1],
        },
        IdentityDecision::New,
    ];
    let names: Vec<&str> = all.iter().map(name).collect();
    assert_eq!(names.len(), 6);
    eprintln!(
        "identity_boundaries: {} decide outcomes: {names:?}",
        names.len()
    );
}

// ---------------------------------------------------------------------------------------------
// (b) One canonicaliser — AC-P2-22-10
// ---------------------------------------------------------------------------------------------

/// **`remote.rs` is the only URL parser in the identity module**, and no module assembles a
/// `<host>/<owner>/<name>` string by hand.
///
/// `RepoListing` carries no `host` field — p2-20 removed it on this plan's argument — so a
/// three-part key cannot be assembled from a listing at all, and this scan has nothing to find by
/// construction. It is written anyway, because that construction can be undone by one field.
#[test]
fn only_one_module_parses_a_url_and_none_assembles_a_key() {
    let files = identity_sources();
    let mut scanned = 0_usize;
    let mut parsers = Vec::new();
    for (name, source) in &files {
        scanned += 1;
        let code = code_only(source);
        if code.contains(r#"split("://")"#) || code.contains(r#"split_once("://")"#) {
            parsers.push(name.clone());
        }
        for needle in [r"{}/{}/{}", r"{host}/{owner}/{name}", r"{}/{owner}/{name}"] {
            assert!(
                !code.contains(needle),
                "{name} assembles a key by hand ({needle}); \
                 every remote_key comes from canonical_remote_key"
            );
        }
    }
    assert_eq!(
        parsers,
        vec!["remote.rs".to_owned()],
        "the identity module has more than one URL parser"
    );
    eprintln!("identity_boundaries: scanned {scanned} identity source file(s) for a second parser");
    assert!(scanned > 0, "a gate whose passing run scans nothing fails");
}

/// Every module this plan added takes its key from `canonical_remote_key` or from a value that
/// already came through it, and the listing side goes through the one splitter.
#[test]
fn the_listing_side_is_split_in_exactly_one_place() {
    let files = identity_sources();
    let mut splitters = Vec::new();
    for (name, source) in &files {
        if code_only(source).contains("fn listing_parts_from") {
            splitters.push(name.clone());
        }
    }
    assert_eq!(splitters, vec!["match_listing.rs".to_owned()]);

    // And nothing else **reads** a listing's `clone_url` — the field a second splitter would
    // need. The needle is the field *access*, `.clone_url`, not the name: a fixture that builds a
    // `RepoListing` writes `clone_url:` in a struct literal and is not a second splitter, and a
    // rule that could not tell those apart would be satisfied by whichever it happened to catch.
    let readers: Vec<String> = files
        .iter()
        .filter(|(_, source)| code_only(source).contains(".clone_url"))
        .map(|(name, _)| name.clone())
        .collect();
    assert_eq!(
        readers,
        vec!["match_listing.rs".to_owned()],
        "a second module reads a listing's clone URL: {readers:?}"
    );
    eprintln!(
        "identity_boundaries: {} of {} files read a listing clone URL",
        readers.len(),
        files.len()
    );
}

// ---------------------------------------------------------------------------------------------
// (c) No merge — AC-P2-22-3
// ---------------------------------------------------------------------------------------------

/// Hydration reaches none of the merge machinery, asserted over the **source** as well as over
/// behaviour: `core/tests/identity_hydrate.rs` proves no row is written, and this proves no
/// module of this plan can write one.
///
/// `acceptance/callsites.json`'s `projects-merge` rule already scans `core/src` **and `app/src`**
/// whole, with `merge.rs` and `commands.rs` as its only core allowances, so a call from
/// `hydrate.rs` trips it today. The rule is not widened; this is the Rust-side twin at module
/// grain, and it prints its count.
#[test]
fn no_module_of_this_plan_reaches_the_merge_machinery() {
    let allowed = ["merge.rs", "commands.rs", "redirect.rs"];
    let files = identity_sources();
    let mut scanned = 0_usize;
    let mut callers = Vec::new();
    for (name, source) in &files {
        if allowed.contains(&name.as_str()) {
            continue;
        }
        scanned += 1;
        let code = code_only(source);
        for needle in [
            "merge_projects",
            "projects.merged",
            "merge_record",
            "project_redirect",
        ] {
            if code.contains(needle) {
                callers.push(format!("{name}: {needle}"));
            }
        }
    }
    assert_eq!(
        callers,
        Vec::<String>::new(),
        "a module outside the merge machinery reaches it"
    );
    eprintln!("identity_boundaries: scanned {scanned} non-merge identity source file(s)");
    assert!(scanned > 0, "a gate whose passing run scans nothing fails");
    assert!(
        files.len() > scanned,
        "the allowance list matched nothing — the scan is not excluding what it claims to"
    );
}
