//! Compiled only under `testkit`: these link `codotheca_core::corpus`, which the feature
//! gates. Without the gate a bare `cargo test` fails to compile rather than skipping.
#![cfg(feature = "testkit")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use codotheca_core::corpus::{
    ensure, fixtures as ids, generate, CorpusManifest, CorpusOptions, CORPUS_VERSION, VOLUME_A,
    VOLUME_B,
};

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("codotheca-corpus-test-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

#[test]
fn an_empty_corpus_still_declares_its_volumes_and_round_trips() {
    let dir = scratch("empty");
    let mut options = CorpusOptions::new(&dir);
    options.only = Some(Vec::new());
    let manifest = generate(&options).unwrap();

    assert_eq!(manifest.corpus_version, CORPUS_VERSION);
    assert!(
        manifest.git_version.starts_with("2."),
        "got {}",
        manifest.git_version
    );
    assert!(manifest.fixtures.is_empty());
    assert_eq!(manifest.volumes.len(), 2);

    let a = manifest.volume(VOLUME_A).unwrap();
    let b = manifest.volume(VOLUME_B).unwrap();
    assert!(a.path.is_dir() && b.path.is_dir());
    assert_ne!(a.volume_key, b.volume_key);
    assert_ne!(a.store_key, b.store_key);

    let reloaded = CorpusManifest::load(&dir).unwrap();
    assert_eq!(reloaded, manifest);
}

#[test]
fn a_missing_fixture_is_an_error_and_never_a_silent_none() {
    let dir = scratch("missing");
    let mut options = CorpusOptions::new(&dir);
    options.only = Some(Vec::new());
    let manifest = generate(&options).unwrap();
    assert!(manifest.fixture("nothing-like-this").is_none());
    assert!(manifest.require("nothing-like-this").is_err());
}

#[test]
fn hermetic_git_ignores_the_developers_own_configuration() {
    let dir = scratch("hermetic");
    let mut options = CorpusOptions::new(&dir);
    options.only = Some(Vec::new());
    let manifest = generate(&options).unwrap();
    let git = manifest.git().unwrap();
    let name = git
        .run(&manifest.root, 0, &["config", "user.name"])
        .unwrap();
    assert_eq!(name, "Corpus Author");
    let autocrlf = git
        .run(&manifest.root, 0, &["config", "core.autocrlf"])
        .unwrap();
    assert_eq!(autocrlf, "false");
}

fn full(scratch: &str) -> CorpusManifest {
    let dir = std::env::temp_dir().join(format!("codotheca-corpus-test-{scratch}"));
    let mut options = CorpusOptions::new(dir);
    options.untracked_files = 10;
    generate(&options).unwrap()
}

#[test]
fn only_pulls_in_the_dependencies_of_what_was_asked_for_in_build_order() {
    let dir = std::env::temp_dir().join("codotheca-corpus-test-deps");
    let mut options = CorpusOptions::new(dir);
    options.only = Some(vec![ids::SHALLOW.to_owned()]);
    let m = generate(&options).unwrap();
    let names: Vec<&str> = m.fixtures.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, vec![ids::UPSTREAM, ids::SHALLOW]);
}

#[test]
fn the_default_selection_builds_every_fixture_in_order() {
    let m = full("all");
    for name in ids::ORDER {
        assert!(
            m.fixture(name).is_some(),
            "{name} is missing from the manifest"
        );
    }
    let order: Vec<&str> = ids::ORDER.to_vec();
    let built: Vec<&str> = m.fixtures.iter().map(|f| f.name.as_str()).collect();
    let mut expected = order.clone();
    expected.retain(|n| built.contains(n));
    assert_eq!(built, expected, "fixtures must appear in ORDER");
}

#[test]
fn two_independent_runs_produce_identical_commit_ids() {
    let first = full("det-a");
    let second = full("det-b");
    for a in &first.fixtures {
        let Some(b) = second.fixture(&a.name) else {
            panic!("{} missing", a.name)
        };
        assert_eq!(
            a.expect.head_oid, b.expect.head_oid,
            "{} head differs",
            a.name
        );
        assert_eq!(
            a.expect.root_oids, b.expect.root_oids,
            "{} roots differ",
            a.name
        );
    }
}

#[test]
fn no_committed_blob_carries_a_carriage_return() {
    // The Windows autocrlf leak, asserted directly rather than inferred from a hash.
    let m = full("crlf");
    let git = m.git().unwrap();
    let f = m.require(ids::UPSTREAM).unwrap();
    let blob = git
        .run_bytes(&f.path, 0, &["cat-file", "blob", "HEAD:README.md"], &[])
        .unwrap();
    assert!(!blob.contains(&b'\r'));
}

#[test]
fn ensure_reuses_an_existing_corpus_and_force_rebuilds_it() {
    let dir = std::env::temp_dir().join("codotheca-corpus-test-ensure");
    let _ = std::fs::remove_dir_all(&dir);
    let mut options = CorpusOptions::new(&dir);
    options.only = Some(vec![ids::ZERO_COMMIT.to_owned()]);
    let first = ensure(&options).unwrap();

    let marker = dir.join("marker.txt");
    std::fs::write(&marker, b"kept").unwrap();
    let second = ensure(&options).unwrap();
    assert_eq!(first, second);
    assert!(
        marker.exists(),
        "ensure must not rebuild when the manifest already matches"
    );

    options.force = true;
    let third = ensure(&options).unwrap();
    assert_eq!(third.corpus_version, first.corpus_version);
    assert!(!marker.exists(), "--force rebuilds from nothing");
}

#[test]
fn the_cli_writes_a_manifest_a_test_can_read() {
    let dir = std::env::temp_dir().join("codotheca-corpus-test-cli");
    let _ = std::fs::remove_dir_all(&dir);
    let exe = env!("CARGO_BIN_EXE_codotheca-corpus");
    let status = std::process::Command::new(exe)
        .args([
            "--out",
            &dir.display().to_string(),
            "--only",
            ids::BARE,
            "--force",
        ])
        .status()
        .unwrap();
    assert!(status.success());
    let m = CorpusManifest::load(&dir).unwrap();
    assert!(m.fixture(ids::BARE).is_some());
    assert!(
        m.fixture(ids::UPSTREAM).is_some(),
        "the dependency came along"
    );
}

#[test]
fn the_cli_writes_nothing_to_stdout() {
    // §2.1: stdout carries protocol frames and nothing else, and this binary ships inside the
    // same crate. Its summary and its skip lines are stderr.
    let dir = std::env::temp_dir().join("codotheca-corpus-test-cli-stdout");
    let _ = std::fs::remove_dir_all(&dir);
    let exe = env!("CARGO_BIN_EXE_codotheca-corpus");
    let out = std::process::Command::new(exe)
        .args([
            "--out",
            &dir.display().to_string(),
            "--only",
            ids::ZERO_COMMIT,
            "--force",
        ])
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(
        out.stdout.is_empty(),
        "stdout: {:?}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert!(!out.stderr.is_empty(), "the summary has to go somewhere");
}

#[test]
fn the_cli_accepts_a_relative_out_directory() {
    // The CI step passes `--out corpus-check`. Every builder sets git's cwd to one directory
    // and passes a path as an argument, so a relative root builds the tree twice over and
    // leaves no repository behind — the step would fail on the first fixture.
    let base = std::env::temp_dir().join("codotheca-corpus-test-relative");
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    let exe = env!("CARGO_BIN_EXE_codotheca-corpus");
    let out = std::process::Command::new(exe)
        .current_dir(&base)
        .args(["--out", "corpus-check", "--only", ids::BARE, "--force"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let m = CorpusManifest::load(&base.join("corpus-check")).unwrap();
    let f = m.require(ids::BARE).unwrap();
    assert!(f.path.is_absolute(), "{}", f.path.display());
    assert!(
        f.path.ends_with("corpus-check/vol-a/bare"),
        "{}",
        f.path.display()
    );
    assert!(
        f.path.join("HEAD").is_file(),
        "the fixture is a real repository"
    );
}
