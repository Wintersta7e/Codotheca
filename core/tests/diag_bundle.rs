#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

//! §11.4's diagnostics bundle: useful, and anonymised by default.

use codotheca_core::index::Index;
use codotheca_core::surfaces::{anonymise, diag};

fn seed(conn: &rusqlite::Connection) {
    conn.execute_batch(
        "INSERT INTO project (id, name, seed_basename, created_at, updated_at,
                              last_touched_at, primary_language, error_kind)
         VALUES (1, 'alpha', 'alpha', 1, 1, 1, 'Rust', 'PERMISSION_DENIED');
         INSERT INTO location (id, project_id, kind, distro, path_bytes, path_key,
                               path_display, volume_key, store_key, presence, repo_kind)
         VALUES (1, 1, 'linux', '', X'2f61', X'2f61',
                 '/one/two/three/alpha', 'vol-abcdef', 'store-1', 'present', 'worktree');
         INSERT INTO scan_root (id, kind, distro, path_bytes, path_key, path_display,
                                enabled, added_by, descend_into_repos, added_at)
         VALUES (1, 'linux', '', X'2f61', X'2f61', '/one/two', 1, 'user', 0, 1);",
    )
    .expect("seed");
}

fn seeded(dir: &std::path::Path) -> Index {
    let index = Index::open(dir).expect("open");
    seed(index.conn());
    index
}

#[test]
fn a_path_keeps_its_basename_its_depth_and_a_volume_shape_and_nothing_else() {
    assert_eq!(
        anonymise::anonymise_path("/one/two/three/alpha", "vol-1"),
        "vol-1/…3…/alpha"
    );
    assert_eq!(anonymise::anonymise_path("/alpha", "vol-1"), "vol-1/alpha");
    assert_eq!(anonymise::anonymise_path("", "vol-1"), "vol-1");
}

#[test]
fn a_windows_path_folds_the_same_way_as_a_unix_one() {
    assert_eq!(
        anonymise::anonymise_path(r"D:\one\two\alpha", "vol-1"),
        "vol-1/…3…/alpha",
        "the drive letter is a segment like any other and must not survive"
    );
}

#[test]
fn one_volume_key_always_gets_the_same_shape_and_two_never_collide() {
    let mut shapes = anonymise::VolumeShapes::new();
    let a = shapes.shape("vol-abcdef");
    let b = shapes.shape("vol-ghijkl");
    assert_eq!(shapes.shape("vol-abcdef"), a);
    assert_ne!(a, b);
    assert_eq!(a, "vol-1");
    assert_eq!(b, "vol-2");
}

#[test]
fn a_location_with_no_stable_volume_identifier_says_so_rather_than_taking_a_number() {
    let mut shapes = anonymise::VolumeShapes::new();
    assert_eq!(
        shapes.shape(""),
        anonymise::UNKNOWN_VOLUME,
        "a NULL volume_key is `no stable identifier exists`, not another volume"
    );
    assert_eq!(
        shapes.shape("vol-abcdef"),
        "vol-1",
        "and it does not consume a number, so two real volumes still read 1 and 2"
    );
}

#[test]
fn the_default_bundle_carries_no_real_path_anywhere_in_its_json() {
    let dir = tempfile::tempdir().expect("tempdir");
    let index = seeded(dir.path());

    let doc = diag::build(index.conn(), false, 900).expect("build");
    let text = serde_json::to_string(&doc).expect("serialise");
    assert!(
        !text.contains("/one/two/three"),
        "no directory structure survives"
    );
    // [p3] The segment as a **token**, not as a bare substring. `!text.contains("one")` held by
    // luck until §30.9's switch list put `abandoned_with_debt` in the settings block — a
    // `DebtSource` slug that happens to spell the segment inside a longer word. The three forms
    // below are the only ways an interior path segment can actually survive: inside a posix path,
    // inside a windows path, or alone as a JSON string.
    for leaked in ["/one", "one/", "\\one", "\"one\""] {
        assert!(
            !text.contains(leaked),
            "no interior segment survives at all: found {leaked}"
        );
    }
    assert!(text.contains("alpha"), "basenames are the product and stay");
    assert!(text.contains("vol-1"));
    assert_eq!(doc["anonymised"], serde_json::json!(true));
}

#[test]
fn show_real_paths_puts_them_back_and_says_so() {
    let dir = tempfile::tempdir().expect("tempdir");
    let index = seeded(dir.path());

    let doc = diag::build(index.conn(), true, 900).expect("build");
    let text = serde_json::to_string(&doc).expect("serialise");
    assert!(text.contains("/one/two/three/alpha"));
    assert_eq!(doc["anonymised"], serde_json::json!(false));
}

#[test]
fn the_bundle_names_the_log_and_never_embeds_it() {
    let dir = tempfile::tempdir().expect("tempdir");
    let index = seeded(dir.path());
    let doc = diag::build(index.conn(), false, 900).expect("build");
    assert!(doc["log"]["path"].is_string());
    assert!(
        doc["log"]["text"].is_null(),
        "a free-text log cannot be anonymised field by field"
    );
}

#[test]
fn the_stated_sections_are_all_present() {
    let dir = tempfile::tempdir().expect("tempdir");
    let index = seeded(dir.path());
    let doc = diag::build(index.conn(), false, 900).expect("build");
    for section in [
        "generatedAt",
        "anonymised",
        "schemaVersion",
        "protocolVersion",
        "coreVersion",
        "settings",
        "roots",
        "projects",
        "locations",
        "sessions",
        "notes",
        "scanProblems",
        "jobStates",
        "log",
    ] {
        assert!(
            doc.get(section).is_some(),
            "{section} is missing from the bundle"
        );
    }
}

#[test]
fn the_written_file_is_the_document_and_its_reported_size() {
    let dir = tempfile::tempdir().expect("tempdir");
    let index = seeded(dir.path());
    let bundle = diag::write_bundle(&index, false, 900).expect("write");
    assert!(bundle.anonymised);
    let path = std::path::Path::new(&bundle.path_display);
    let text = std::fs::read_to_string(path).expect("read back");
    assert_eq!(i64::try_from(text.len()).expect("fits"), bundle.size_bytes);
    assert!(
        !text.contains("/one/two/three"),
        "the file on disk is anonymised too, not only the returned document"
    );
    assert!(
        bundle.path_display.contains(diag::BUNDLE_FILE_PREFIX),
        "the bundle names itself so a user can find what they are attaching"
    );
    assert!(
        !path.with_extension("json.tmp").exists(),
        "the temp file is renamed, never left beside the bundle"
    );
}
