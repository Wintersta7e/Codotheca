#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! `AC-P4-50-1`, the core half: a ticked WSL distro row adds that distro's root.
//!
//! Every distro row renders the same home-parent path, so the path alone cannot say which
//! distro was meant. The distro travels beside it in `roots.add`, and the stored root carries it;
//! a root added with no distro is the host's own folder, never the distro's.

use std::sync::Arc;

use codotheca_core::firstrun::{classify, roots, sources, FirstRunEnv};
use codotheca_core::index::path::PathPlatform;
use codotheca_core::protocol::Bytes;
use codotheca_core::scan::skiplist::SkipList;
use codotheca_core::wsl::distros::{DistroInfo, DistroState};
use serde_json::{json, Value};

const NOW: i64 = 1_760_000_000;

#[derive(Debug)]
struct TwoDistros;

impl classify::DistroProbe for TwoDistros {
    fn distros(&self) -> Vec<DistroInfo> {
        vec![
            DistroInfo {
                name: "alpha".to_owned(),
                state: DistroState::Running,
            },
            DistroInfo {
                name: "beta".to_owned(),
                state: DistroState::Stopped,
            },
        ]
    }
}

fn env(home: &std::path::Path) -> FirstRunEnv {
    FirstRunEnv {
        sources: sources::SourceEnv {
            home: home.to_path_buf(),
            app_data: None,
            xdg_config: None,
        },
        classifier: Arc::new(classify::FixedClassifier::new(vec![])),
        distros: Arc::new(TwoDistros),
        platform: PathPlatform::Unix,
        skip: SkipList::default(),
        cache: roots::SuggestionCache::new(),
    }
}

fn call(conn: &mut rusqlite::Connection, env: &FirstRunEnv, command: &str, args: &Value) -> Value {
    codotheca_core::firstrun::dispatch(conn, env, command, args, NOW)
        .expect("a first-run command")
        .unwrap_or_else(|failure| panic!("{command} failed: {failure:?}"))
}

/// What the shell sends for a ticked suggestion: the core's own path and the core's own distro.
fn add_args(suggestion: &Value) -> Value {
    let path = suggestion["pathDisplay"].as_str().unwrap();
    json!({
        "pathBytes": serde_json::to_value(Bytes(path.as_bytes().to_vec())).unwrap(),
        "confirmLarge": true,
        "distro": suggestion["distro"],
    })
}

#[test]
fn ac_p4_50_1_a_distro_root_keeps_its_distro() {
    let home = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let mut index = codotheca_core::index::Index::open_at(data.path(), NOW).unwrap();
    let e = env(home.path());

    let suggested = call(index.conn_mut(), &e, "roots.suggest", &json!({}));
    let distro_rows: Vec<&Value> = suggested
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row["kind"] == "wsl")
        .collect();
    assert_eq!(distro_rows.len(), 2, "one row per distro: {suggested}");
    // The precondition that makes the distro load-bearing: both rows render one path.
    assert_eq!(distro_rows[0]["pathDisplay"], distro_rows[1]["pathDisplay"]);

    let beta = distro_rows
        .iter()
        .find(|row| row["distro"] == "beta")
        .expect("the beta row");
    let added = call(index.conn_mut(), &e, "roots.add", &add_args(beta));
    assert!(added["refusedBecause"].is_null(), "refused: {added}");
    assert_eq!(added["root"]["distro"], "beta", "{added}");
    assert_eq!(added["root"]["kind"], "wsl", "{added}");

    let listed = call(index.conn_mut(), &e, "roots.list", &json!({}));
    let listed = listed.as_array().unwrap();
    assert_eq!(
        listed.len(),
        1,
        "ticking one distro adds exactly one root: {listed:?}"
    );
    assert_eq!(listed[0]["distro"], "beta");

    // The other distro's identical path is a different root, not a duplicate of the first.
    let alpha = distro_rows
        .iter()
        .find(|row| row["distro"] == "alpha")
        .expect("the alpha row");
    let second = call(index.conn_mut(), &e, "roots.add", &add_args(alpha));
    assert!(second["refusedBecause"].is_null(), "refused: {second}");
    assert_eq!(second["root"]["distro"], "alpha", "{second}");
}

#[test]
fn ac_p4_50_1_a_dialog_folder_stays_native() {
    let home = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let picked = tempfile::tempdir().unwrap();
    let mut index = codotheca_core::index::Index::open_at(data.path(), NOW).unwrap();
    let e = env(home.path());

    let path = picked.path().to_str().unwrap();
    let added = call(
        index.conn_mut(),
        &e,
        "roots.add",
        &json!({
            "pathBytes": serde_json::to_value(Bytes(path.as_bytes().to_vec())).unwrap(),
            "confirmLarge": true,
            "distro": "",
        }),
    );
    assert!(added["refusedBecause"].is_null(), "refused: {added}");
    assert_eq!(added["root"]["distro"], "", "{added}");
    assert_eq!(added["root"]["kind"], "linux", "{added}");
}
