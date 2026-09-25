#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! §47.9 B — the tripwire: every config key the gate's git lists in an audited namespace is
//! classified, or this fails naming it.
//!
//! Config decides what a verb writes, and git adds keys. A key a newer git lists and
//! `core/src/gitw/config_keys.rs` does not know is caught **by the gate that runs that git** —
//! the WSL gate, the Windows-native gate and CI each list their own. The count per namespace is
//! printed and derived, never asserted.

use std::collections::BTreeMap;
use std::process::Command;

use codotheca_core::gitw::{AUDITED_CORE_KEYS, AUDITED_NAMESPACES, CONFIG_KEY_CLASSES};

/// The keys of `listing` in an audited namespace, or on the audited `core.*` list.
fn audited<'a>(listing: &[&'a str]) -> Vec<&'a str> {
    listing
        .iter()
        .copied()
        .filter(|key| {
            let namespace = key.split('.').next().unwrap_or("");
            AUDITED_NAMESPACES.contains(&namespace) || AUDITED_CORE_KEYS.contains(key)
        })
        .collect()
}

/// The audited keys of `listing` the classification table does not know.
fn unclassified<'a>(listing: &[&'a str]) -> Vec<&'a str> {
    audited(listing)
        .into_iter()
        .filter(|key| !CONFIG_KEY_CLASSES.iter().any(|(known, _)| known == key))
        .collect()
}

fn git_output(args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("LC_ALL", "C")
        .output()
        .expect("the gate's git runs");
    assert!(out.status.success(), "git {args:?} failed");
    String::from_utf8(out.stdout).expect("utf-8")
}

/// **AC-P4-47-5.** Every key `git help --config` lists in §47.9 B's namespaces is classified.
#[test]
fn every_audited_config_key_the_gates_git_lists_is_classified() {
    let version = git_output(&["--version"]);
    let listing_text = git_output(&["help", "--config"]);
    let listing: Vec<&str> = listing_text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    let keys = audited(&listing);
    let mut per_namespace: BTreeMap<&str, usize> = BTreeMap::new();
    for key in &keys {
        *per_namespace
            .entry(key.split('.').next().unwrap_or(""))
            .or_default() += 1;
    }
    eprintln!(
        "git-config-tripwire: {} — {} keys listed, {} audited, {} classified in the table; per \
         namespace {per_namespace:?}",
        version.trim(),
        listing.len(),
        keys.len(),
        CONFIG_KEY_CLASSES.len()
    );
    assert!(
        !listing.is_empty() && !keys.is_empty(),
        "git help --config listed nothing audited, so nothing was checked"
    );
    let missing = unclassified(&listing);
    assert!(
        missing.is_empty(),
        "the gate's git lists audited config keys the classification table does not know — \
         classify each in core/src/gitw/config_keys.rs: {missing:?}"
    );
}

/// The table holds each key once: a key classified twice is two readings that can disagree.
#[test]
fn the_classification_table_names_each_key_once() {
    let mut seen: Vec<&str> = CONFIG_KEY_CLASSES.iter().map(|(key, _)| *key).collect();
    let total = seen.len();
    seen.sort_unstable();
    seen.dedup();
    assert_eq!(seen.len(), total, "a key is classified twice");
    for key in &seen {
        let namespace = key.split('.').next().unwrap_or("");
        assert!(
            AUDITED_NAMESPACES.contains(&namespace) || AUDITED_CORE_KEYS.contains(key),
            "{key} is classified but not audited"
        );
    }
    eprintln!("git-config-tripwire: {total} classified keys, each once");
}

/// **The bite**: a listing carrying one key nobody classified fails, naming it.
#[test]
fn an_invented_key_in_an_audited_namespace_fails_by_name() {
    let listing = [
        "fetch.prune",
        "fetch.inventedByANewerGit",
        "core.editor",
        "push.default",
    ];
    assert_eq!(unclassified(&listing), vec!["fetch.inventedByANewerGit"]);
    assert!(
        !audited(&listing).contains(&"push.default"),
        "push.* is not audited: no intent pushes (U1)"
    );
}
