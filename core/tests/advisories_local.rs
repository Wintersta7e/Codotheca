#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! ENV-4: the advisory lookup sends registry-published names only.
//!
//! Every name a parser yields is sent to the advisory database. A workspace member, a `file:` or
//! `link:` directory, a path crate or a git checkout is the user's own code: its name is private,
//! and no advisory can exist for it, so sending it leaks the name and buys nothing. A git, url or
//! tarball source is not registry-published either, and its name can collide with an unrelated
//! registry package. Each fixture below holds one registry package beside one package of every
//! local and non-registry kind its format records, and must yield the registry pair alone.

use std::path::Path;

use codotheca_core::advisories::parse::{parse_lockfile, LockfileRead};
use codotheca_core::provider::PackageVersion;

fn fixture(dir: &str, name: &str) -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/lockfiles")
        .join(dir)
        .join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn parsed(name: &str, bytes: &[u8]) -> Vec<PackageVersion> {
    match parse_lockfile(name, bytes) {
        LockfileRead::Parsed(pairs) => pairs,
        LockfileRead::NotRead => panic!("{name} was not read"),
    }
}

fn only_registry(dir: &str, name: &str, version: &str) {
    let file = dir.split('-').next().unwrap();
    let lockfile = match file {
        "npm" => "package-lock.json",
        "cargo" => "Cargo.lock",
        "yarn" => "yarn.lock",
        "pnpm" => "pnpm-lock.yaml",
        "poetry" => "poetry.lock",
        "uv" => "uv.lock",
        other => panic!("no lockfile for {other}"),
    };
    let pairs = parsed(lockfile, &fixture(dir, lockfile));
    assert_eq!(
        pairs,
        vec![PackageVersion {
            name: name.to_owned(),
            version: version.to_owned(),
        }],
        "{dir}/{lockfile}"
    );
}

/// v3 keys a workspace member and a `file:` directory by their path, outside `node_modules/`,
/// and the install-path entries pointing at them are `link`s. A git, `file:` tarball, plain url
/// tarball or forge-archive `resolved` is not a registry tarball.
#[test]
fn ac_p4_50_6_npm_sends_registry_only() {
    only_registry("npm", "left-pad-like", "1.3.0");
    // v1 records a local directory, a git checkout or a tarball url as its version.
    only_registry("npm-v1", "left-pad-like", "1.3.0");
}

/// A path crate and the root package carry no `source` line; a git crate's starts `git+`.
#[test]
fn ac_p4_50_6_cargo_sends_registry_only() {
    only_registry("cargo", "left-pad-like", "1.0.0");
}

/// Classic records `file:` and `link:` ranges; Berry adds `workspace:` and `portal:`. A git,
/// `github:` or url tarball resolution is not the registry, in either dialect.
#[test]
fn ac_p4_50_6_yarn_sends_registry_only() {
    only_registry("yarn", "left-pad-like", "1.3.0");
    only_registry("yarn-berry", "left-pad-like", "1.3.0");
}

/// Workspace members and `link:` targets live only under `importers`; a `file:` directory is a
/// `packages` key of its own. A git or tarball package is keyed by its url, with a `repo` or an
/// off-registry `tarball` resolution.
#[test]
fn ac_p4_50_6_pnpm_sends_registry_only() {
    only_registry("pnpm", "left-pad-like", "1.3.0");
}

/// `[package.source]` of type `directory`, `file`, `git` and `url` are not the registry.
#[test]
fn ac_p4_50_6_poetry_sends_registry_only() {
    only_registry("poetry", "left-pad-like", "1.3.0");
}

/// `editable`, `virtual`, `directory`, `path`, `git` and `url` sources are not the registry.
#[test]
fn ac_p4_50_6_uv_sends_registry_only() {
    only_registry("uv", "left-pad-like", "1.3.0");
}

/// This repository's own lockfiles, read as a user's would be: its workspace members and its
/// own crate are names nothing has published.
#[test]
fn ac_p4_50_6_this_repository_sends_no_own_name() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let npm = parsed(
        "package-lock.json",
        &std::fs::read(root.join("../package-lock.json")).unwrap(),
    );
    let cargo = parsed(
        "Cargo.lock",
        &std::fs::read(root.join("Cargo.lock")).unwrap(),
    );
    eprintln!(
        "this repository: {} npm pairs, {} cargo pairs",
        npm.len(),
        cargo.len()
    );
    // A read that yielded nothing would pass the assertion below vacuously.
    assert!(npm.len() > 100, "npm pairs: {}", npm.len());
    assert!(cargo.len() > 100, "cargo pairs: {}", cargo.len());
    for pair in npm.iter().chain(&cargo) {
        assert!(
            !["app", "protocol", "codotheca-core"].contains(&pair.name.as_str())
                && !pair.name.contains("codotheca"),
            "own name sent: {}@{}",
            pair.name,
            pair.version
        );
    }
}
