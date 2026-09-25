#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! §32.6's lockfile read: **six names, depth 3, five parsers, and a cap exceedance that is never a
//! partial parse.**
//!
//! The failure this file exists to catch is a **false clean**: a short triple set produces a lit
//! tick claiming *no known vulnerable dependencies* over a read that never looked. Every assertion
//! below either prints the count it scanned or names the row it expected, because a walk that
//! matched nothing in a fixture that contains lockfiles is a failing walk.

use std::path::Path;

use codotheca_core::advisories::lockfiles::{
    read_lockfiles, walk_lockfiles, LOCKFILE_BYTE_CAP, LOCKFILE_COUNT_CAP, LOCKFILE_MAX_DEPTH,
    LOCKFILE_NAMES,
};
use codotheca_core::advisories::parse::{parse_lockfile, LockfileRead};
use codotheca_core::index::migrate::{apply_all, MIGRATIONS};
use codotheca_core::index::{open_connection, Index};
use codotheca_core::protocol::{Ecosystem, ProjectId};
use codotheca_core::provider::PackageVersion;

const NOW: i64 = 1_800_000_000;

fn fresh() -> (tempfile::TempDir, rusqlite::Connection) {
    let dir = tempfile::tempdir().unwrap();
    let mut conn = open_connection(&Index::db_path(dir.path())).unwrap();
    apply_all(&mut conn, MIGRATIONS).unwrap();
    (dir, conn)
}

fn insert_project(conn: &rusqlite::Connection, name: &str) -> ProjectId {
    conn.execute(
        "INSERT INTO project (name, seed_basename, created_at, updated_at)
         VALUES (?1, ?1, 1, 1)",
        [name],
    )
    .unwrap();
    ProjectId(conn.last_insert_rowid())
}

fn write(root: &Path, rel: &str, body: &str) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, body).unwrap();
}

fn pair(name: &str, version: &str) -> PackageVersion {
    PackageVersion {
        name: name.to_owned(),
        version: version.to_owned(),
    }
}

fn parsed(read: LockfileRead) -> Vec<PackageVersion> {
    match read {
        LockfileRead::Parsed(items) => items,
        LockfileRead::NotRead => panic!("expected a parse, got NotRead"),
    }
}

fn triples(conn: &rusqlite::Connection, project: ProjectId) -> Vec<(String, String, String)> {
    let mut stmt = conn
        .prepare(
            "SELECT ecosystem, package_name, version FROM project_dependency
              WHERE project_id = ?1 ORDER BY ecosystem, package_name, version",
        )
        .unwrap();
    let rows = stmt
        .query_map([project.0], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    rows
}

fn read_state(conn: &rusqlite::Connection, project: ProjectId, path: &str) -> Option<String> {
    conn.query_row(
        "SELECT read_state FROM project_lockfile WHERE project_id = ?1 AND source_path = ?2",
        rusqlite::params![project.0, path],
        |r| r.get(0),
    )
    .ok()
}

/// **AC-P3-32-17.** Depth 1, 2 and 3 are matched; depth 4 is not.
///
/// **Depth 1 would miss a monorepo's per-package lockfiles**, and the read would then produce a
/// partial triple set for a project that looks fully scanned.
#[test]
fn ac_p3_32_17_the_walk_reaches_depth_three_and_no_further() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(root, "Cargo.lock", "");
    write(root, "one/package-lock.json", "{}");
    write(root, "one/two/yarn.lock", "");
    write(root, "one/two/three/poetry.lock", "");
    // Inside an install directory, a dependency's own lockfile declares somebody else's graph.
    write(root, "node_modules/dep/Cargo.lock", "");

    let walk = walk_lockfiles(root);
    let found: Vec<&str> = walk.files.iter().map(|f| f.source_path.as_str()).collect();
    eprintln!(
        "advisory_lockfiles: {} file(s) matched, {} dir(s) entered: {found:?}",
        walk.files.len(),
        walk.dirs_entered
    );
    assert!(
        !walk.files.is_empty(),
        "a walk that matched nothing over a fixture holding lockfiles proves nothing"
    );
    assert_eq!(
        found,
        vec!["Cargo.lock", "one/package-lock.json", "one/two/yarn.lock"]
    );
    assert!(walk.dirs_entered > 0, "the walk entered no directory");
    assert!(walk.complete);
    assert_eq!(LOCKFILE_MAX_DEPTH, 3);
}

/// **AC-P3-32-16.** A file over [`LOCKFILE_BYTE_CAP`] is `not_read`, writes **zero** triples, and
/// is **never** a partial parse.
///
/// The cap is read from the constant and never written out as a literal.
#[test]
fn ac_p3_32_16_a_file_over_the_cap_is_not_read() {
    let (_d, conn) = fresh();
    let project = insert_project(&conn, "big");
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    // A real `package-lock.json` whose byte length exceeds the cap: the entries are valid, so a
    // reader that ignored the cap would parse them and report a shorter-than-true set only when
    // the file was also truncated. Here the whole file is refused.
    let filler = "x".repeat(4096);
    let mut body = String::from("{\"lockfileVersion\":3,\"packages\":{\"\":{},");
    let mut n = 0u32;
    while u64::try_from(body.len()).unwrap() <= LOCKFILE_BYTE_CAP {
        use std::fmt::Write as _;
        write!(
            body,
            "\"node_modules/p{n}\":{{\"version\":\"1.0.0\",\"_\":\"{filler}\"}},"
        )
        .unwrap();
        n += 1;
    }
    body.push_str("\"node_modules/last\":{\"version\":\"1.0.0\"}}}");
    write(root, "package-lock.json", &body);
    let size = std::fs::metadata(root.join("package-lock.json"))
        .unwrap()
        .len();
    eprintln!("advisory_lockfiles: the oversized lockfile is {size} bytes");
    assert!(size > LOCKFILE_BYTE_CAP);

    let mut conn = conn;
    let tx = conn.transaction().unwrap();
    let written = read_lockfiles(&tx, project, root, NOW).unwrap();
    tx.commit().unwrap();

    assert_eq!(written, 0, "a capped file contributes no triples at all");
    assert_eq!(
        read_state(&conn, project, "package-lock.json").as_deref(),
        Some("notRead")
    );
    assert!(triples(&conn, project).is_empty());
    // The scan **ran**, which is what makes this `unknown` rather than `has not run`.
    let scanned: i64 = conn
        .query_row(
            "SELECT count(*) FROM project_dependency_scan WHERE project_id = ?1",
            [project.0],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(scanned, 1);
}

/// Each of the five parsers over a real lockfile of its format, then **the same file with one line
/// corrupted inside the read region**: `NotRead`, and not the subset it managed to collect.
#[test]
fn every_parser_is_total_or_not_read() {
    let v3 = r#"{"lockfileVersion":3,"packages":{
        "":{"name":"root"},
        "node_modules/left":{"version":"1.0.0"},
        "node_modules/@scope/right":{"version":"2.0.0"}}}"#;
    assert_eq!(
        parsed(parse_lockfile("package-lock.json", v3.as_bytes())),
        vec![pair("@scope/right", "2.0.0"), pair("left", "1.0.0")]
    );
    // An entry with no version is a construct this reader does not understand. Skipping it would
    // shorten the set, and a short set is a false clean.
    let v3_broken = v3.replace(
        r#""node_modules/left":{"version":"1.0.0"}"#,
        r#""node_modules/left":{}"#,
    );
    assert_eq!(
        parse_lockfile("package-lock.json", v3_broken.as_bytes()),
        LockfileRead::NotRead
    );

    let yarn = "# yarn lockfile v1\n\n\nleft@^1.0.0, left@^1.2.0:\n  version \"1.2.3\"\n  resolved \"x\"\n\n\"@scope/right@^2.0.0\":\n  version \"2.0.0\"\n";
    assert_eq!(
        parsed(parse_lockfile("yarn.lock", yarn.as_bytes())),
        vec![pair("left", "1.2.3"), pair("@scope/right", "2.0.0")]
    );
    let yarn_broken = "left@^1.0.0:\n  resolved \"x\"\n\nright@^1.0.0:\n  version \"1.0.0\"\n";
    assert_eq!(
        parse_lockfile("yarn.lock", yarn_broken.as_bytes()),
        LockfileRead::NotRead
    );

    let pnpm = "lockfileVersion: '9.0'\n\npackages:\n\n  left@1.2.3:\n    resolution: {integrity: sha512-x}\n\n  '@scope/right@2.0.0(react@18.0.0)':\n    resolution: {integrity: sha512-y}\n\nsnapshots:\n\n  left@1.2.3: {}\n";
    assert_eq!(
        parsed(parse_lockfile("pnpm-lock.yaml", pnpm.as_bytes())),
        vec![pair("left", "1.2.3"), pair("@scope/right", "2.0.0")]
    );
    let pnpm_broken = "packages:\n\n  left\n";
    assert_eq!(
        parse_lockfile("pnpm-lock.yaml", pnpm_broken.as_bytes()),
        LockfileRead::NotRead
    );

    let cargo = "version = 3\n\n[[package]]\nname = \"left\"\nversion = \"1.2.3\"\n\n[[package]]\nname = \"right\"\nversion = \"2.0.0\"\ndependencies = [\n \"left\",\n]\n";
    assert_eq!(
        parsed(parse_lockfile("Cargo.lock", cargo.as_bytes())),
        vec![pair("left", "1.2.3"), pair("right", "2.0.0")]
    );
    let cargo_broken =
        "[[package]]\nname = \"left\"\n\n[[package]]\nname = \"right\"\nversion = \"2.0.0\"\n";
    assert_eq!(
        parse_lockfile("Cargo.lock", cargo_broken.as_bytes()),
        LockfileRead::NotRead
    );

    // `poetry.lock` and `uv.lock` share one reader because they share one format — three files,
    // two ecosystems, one parser.
    let poetry = "[[package]]\nname = \"requests\"\nversion = \"2.31.0\"\n";
    for name in ["poetry.lock", "uv.lock"] {
        assert_eq!(
            parsed(parse_lockfile(name, poetry.as_bytes())),
            vec![pair("requests", "2.31.0")],
            "{name}"
        );
    }
}

/// v1, v2 and v3 all yield their triples: **the format difference is a parser detail and the
/// ecosystem is one.**
#[test]
fn all_three_package_lock_layouts_yield_their_triples() {
    let v1 = r#"{"lockfileVersion":1,"dependencies":{
        "left":{"version":"1.0.0","dependencies":{"nested":{"version":"0.1.0"}}},
        "right":{"version":"2.0.0"}}}"#;
    let mut got = parsed(parse_lockfile("package-lock.json", v1.as_bytes()));
    got.sort_by(|a, b| a.name.cmp(&b.name));
    assert_eq!(
        got,
        vec![
            pair("left", "1.0.0"),
            pair("nested", "0.1.0"),
            pair("right", "2.0.0")
        ]
    );

    let v2 = r#"{"lockfileVersion":2,"packages":{"":{},"node_modules/left":{"version":"1.0.0"}},
                 "dependencies":{"left":{"version":"9.9.9"}}}"#;
    assert_eq!(
        parsed(parse_lockfile("package-lock.json", v2.as_bytes())),
        vec![pair("left", "1.0.0")],
        "v2 carries both maps and `packages` is the authoritative one"
    );
}

/// A project whose walk found nothing writes a scan row with `files_matched = 0`; a project never
/// walked has **no row**. The two are distinguishable **by `SELECT`**, not by convention.
#[test]
fn a_walk_that_found_nothing_differs_from_a_walk_that_never_ran() {
    let (_d, conn) = fresh();
    let scanned = insert_project(&conn, "scanned");
    let untouched = insert_project(&conn, "untouched");
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("README.md"), "no lockfiles here").unwrap();

    let mut conn = conn;
    let tx = conn.transaction().unwrap();
    let written = read_lockfiles(&tx, scanned, dir.path(), NOW).unwrap();
    tx.commit().unwrap();
    assert_eq!(written, 0);

    let row: Option<(i64, i64)> = conn
        .query_row(
            "SELECT files_matched, complete FROM project_dependency_scan WHERE project_id = ?1",
            [scanned.0],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .ok();
    assert_eq!(row, Some((0, 1)), "ran and found nothing");
    let none: Option<i64> = conn
        .query_row(
            "SELECT files_matched FROM project_dependency_scan WHERE project_id = ?1",
            [untouched.0],
            |r| r.get(0),
        )
        .ok();
    assert_eq!(none, None, "never walked has no row at all");
}

/// **AC-P3-32-18, first half.** Every row this read writes is a **worktree** observation, and the
/// rows written are enumerated and counted rather than assumed.
#[test]
fn ac_p3_32_18_the_read_writes_worktree_rows_and_counts_them() {
    let (_d, conn) = fresh();
    let project = insert_project(&conn, "mono");
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(
        root,
        "Cargo.lock",
        "[[package]]\nname = \"serde\"\nversion = \"1.0.0\"\n",
    );
    write(
        root,
        "web/package-lock.json",
        r#"{"lockfileVersion":3,"packages":{"":{},"node_modules/left":{"version":"1.0.0"}}}"#,
    );
    // The same triple from a second file: one row, because the key is the triple.
    write(
        root,
        "api/package-lock.json",
        r#"{"lockfileVersion":3,"packages":{"":{},"node_modules/left":{"version":"1.0.0"}}}"#,
    );

    let mut conn = conn;
    let tx = conn.transaction().unwrap();
    let written = read_lockfiles(&tx, project, root, NOW).unwrap();
    tx.commit().unwrap();

    let rows = triples(&conn, project);
    eprintln!(
        "advisory_lockfiles: {written} triple row(s) written, {} stored: {rows:?}",
        rows.len()
    );
    assert_eq!(written, 2, "the duplicate triple wrote no second row");
    assert_eq!(
        rows,
        vec![
            ("npm".to_owned(), "left".to_owned(), "1.0.0".to_owned()),
            ("rust".to_owned(), "serde".to_owned(), "1.0.0".to_owned()),
        ]
    );
    // The basis is stated once, on the sweep row §28 owns, and never as a column here.
    let has_basis: i64 = conn
        .query_row(
            "SELECT count(*) FROM pragma_table_info('project_dependency') WHERE name = 'basis'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(has_basis, 0, "a basis that never varies is not a column");

    // A second read replaces: a deleted lockfile leaves no triples behind.
    std::fs::remove_file(root.join("web/package-lock.json")).unwrap();
    std::fs::remove_file(root.join("api/package-lock.json")).unwrap();
    let reread_tx = conn.transaction().unwrap();
    read_lockfiles(&reread_tx, project, root, NOW + 60).unwrap();
    reread_tx.commit().unwrap();
    assert_eq!(
        triples(&conn, project),
        vec![("rust".to_owned(), "serde".to_owned(), "1.0.0".to_owned())]
    );
}

/// The six names resolve to three ecosystems, in one place, and every one is reachable.
#[test]
fn the_six_names_are_the_whole_of_what_the_walk_matches() {
    let dir = tempfile::tempdir().unwrap();
    for (name, _) in LOCKFILE_NAMES {
        write(dir.path(), name, "");
    }
    write(dir.path(), "Gemfile.lock", "");
    write(dir.path(), "composer.lock", "");

    let walk = walk_lockfiles(dir.path());
    let mut found: Vec<&str> = walk.files.iter().map(|f| f.source_path.as_str()).collect();
    found.sort_unstable();
    let mut expected: Vec<&str> = LOCKFILE_NAMES.iter().map(|(n, _)| *n).collect();
    expected.sort_unstable();
    eprintln!(
        "advisory_lockfiles: {} of {} names matched",
        found.len(),
        LOCKFILE_NAMES.len()
    );
    assert_eq!(found, expected);
    assert_eq!(LOCKFILE_NAMES.len(), 6);
    let mut ecosystems: Vec<Ecosystem> = Vec::new();
    for (_, eco) in LOCKFILE_NAMES {
        if !ecosystems.contains(&eco) {
            ecosystems.push(eco);
        }
    }
    assert_eq!(ecosystems.len(), Ecosystem::ALL.len());
}

/// The count cap is a **bound and not a failure**, but it is one the verdict has to know about:
/// `complete` goes false, and a bounded read of an unbounded tree cannot claim to have seen
/// everything.
#[test]
fn the_count_cap_marks_the_walk_incomplete() {
    let dir = tempfile::tempdir().unwrap();
    for i in 0..=LOCKFILE_COUNT_CAP {
        write(dir.path(), &format!("p{i}/Cargo.lock"), "");
    }
    let walk = walk_lockfiles(dir.path());
    eprintln!(
        "advisory_lockfiles: {} file(s) under a cap of {LOCKFILE_COUNT_CAP}, complete={}",
        walk.files.len(),
        walk.complete
    );
    assert_eq!(walk.files.len(), LOCKFILE_COUNT_CAP);
    assert!(!walk.complete);
}
