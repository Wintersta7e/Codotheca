//! `codotheca-sidecar-fixture` — plant an index the core cannot open, for the end-to-end runs of
//! the startup report windows (§11.2a) and of the rebuild chain (§48.7.1), and read what the chain
//! left once the app has quit.
//!
//! Every line this binary prints goes to **stderr**: §2.1 gives stdout to protocol frames and
//! nothing else, and the crate denies `clippy::print_stdout`. What it planted is written to
//! `<data-dir>/fixture.json` and what it found to `<data-dir>/verify.json`, which is what a spec
//! reads.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use codotheca_core::index::sidecar::{counts, export, read, RestoreRule, Scope, Sidecar, SECTIONS};
use codotheca_core::index::{Index, IndexError};
use codotheca_core::testing::sidecar::{
    build_library, compare_documents, plant_startup_failure, FixtureLibrary, StartupPlant,
};
use serde_json::{json, Value};

const USAGE: &str =
    "usage: codotheca-sidecar-fixture plant --kind corrupt|future|migration-failed \
                     --data-dir <dir>
       codotheca-sidecar-fixture plant --kind populated --data-dir <dir> --repos <dir>
       codotheca-sidecar-fixture verify --data-dir <dir>";

/// The result file, beside the planted index.
const FIXTURE_FILE: &str = "fixture.json";

/// What `verify` found, beside the rebuilt index.
const VERIFY_FILE: &str = "verify.json";

/// The populated plant's sidecar as it was exported, for `verify` to compare the rebuilt index
/// with: the app rewrites the sidecar itself, and the copy the rebuild sets aside is named by
/// the time it ran.
const PLANTED_SIDECAR: &str = "planted-sidecar.json";

/// A fixed clock: the plant is test data, and no window this feeds prints the time it was made.
const PLANTED_AT: i64 = 1_700_000_000;

/// After every repository's fixed commit date and every planted row.
const VERIFIED_AT: i64 = PLANTED_AT + 86_400;

enum Mode {
    Startup(StartupPlant, PathBuf),
    Populated { data: PathBuf, repos: PathBuf },
    Verify(PathBuf),
}

fn parse() -> Option<Mode> {
    let mut args = std::env::args().skip(1);
    let verb = args.next()?;
    let (mut kind, mut data, mut repos) = (None, None, None);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--kind" => kind = args.next(),
            "--data-dir" => data = args.next().map(PathBuf::from),
            "--repos" => repos = args.next().map(PathBuf::from),
            _ => return None,
        }
    }
    match (verb.as_str(), kind.as_deref(), repos) {
        ("plant", Some("populated"), Some(repos)) => Some(Mode::Populated { data: data?, repos }),
        ("plant", Some(word), None) => Some(Mode::Startup(StartupPlant::from_word(word)?, data?)),
        ("verify", None, None) => Some(Mode::Verify(data?)),
        _ => None,
    }
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// The projects the shelf lists once a scan finds them again: each visible project with a copy
/// still on disk, by the name its card shows.
fn shelf(index: &Index) -> Result<Vec<String>, IndexError> {
    let names = index
        .conn()
        .prepare(
            "SELECT DISTINCT p.name FROM project p JOIN location l ON l.project_id = p.id
              WHERE p.is_hidden = 0 AND l.presence = 'present' ORDER BY p.name",
        )?
        .query_map([], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    Ok(names)
}

/// A library, its sidecar, then the index corrupted beside a junk journal pair: the
/// `corrupt_index` window, with everything a rebuild restores waiting in the sidecar.
fn plant_populated(data: &Path, repos: &Path) -> Result<Value, IndexError> {
    let FixtureLibrary {
        index, subjects, ..
    } = build_library(data, repos, PLANTED_AT)?;
    let written = index.export_sidecar(PLANTED_AT + 10)?;
    let shelf = shelf(&index)?;
    drop(index);
    std::fs::copy(&written.path, data.join(PLANTED_SIDECAR))?;
    let files = plant_startup_failure(StartupPlant::Corrupt, data, PLANTED_AT)?;

    let db = file_name(&Index::db_path(data));
    let quarantinable = [
        db.clone(),
        format!("{db}-wal"),
        format!("{db}-shm"),
        file_name(&written.path),
    ];
    let immediate: Vec<&str> = SECTIONS
        .iter()
        .filter(|s| matches!(s.scope, Scope::Global | Scope::NoScan))
        .map(|s| s.name)
        .collect();
    Ok(json!({
        "kind": "populated",
        "files": files,
        "quarantinable": quarantinable,
        "counts": written.counts,
        "subjects": subjects,
        "immediate": immediate,
        "shelf": shelf,
    }))
}

fn sessions_of(doc: &Sidecar, subject: &str) -> usize {
    doc.payload
        .projects
        .iter()
        .filter(|p| p.subject == subject)
        .map(|p| p.sessions.len())
        .sum()
}

/// The index the app left, exported and compared with the planted document.
fn verify(data: &Path) -> Result<Value, IndexError> {
    let planted = read(&data.join(PLANTED_SIDECAR))?;
    let index = Index::open_at(data, VERIFIED_AT)?;
    let after = export(index.conn(), planted.generation, VERIFIED_AT)?;
    let comparison = compare_documents(&planted, &after)?;
    let pending: i64 = index
        .conn()
        .query_row("SELECT COUNT(*) FROM sidecar_pending", [], |r| r.get(0))?;

    let now = counts(&after);
    let counted: BTreeMap<String, [u64; 2]> = counts(&planted)
        .into_iter()
        .map(|(key, n)| {
            let restored = now.get(&key).copied().unwrap_or(0);
            (key, [n, restored])
        })
        .collect();
    let session_mismatch: Vec<&str> = planted
        .payload
        .projects
        .iter()
        .filter(|p| sessions_of(&after, &p.subject) != p.sessions.len())
        .map(|p| p.subject.as_str())
        .collect();
    let raise_only: Vec<&str> = SECTIONS
        .iter()
        .filter(|s| matches!(s.rule, RestoreRule::RaiseOnly))
        .map(|s| s.name)
        .collect();
    // No raise-only section is registered yet. The first one brings the comparison its values
    // need; until it does, it is named here rather than passed unchecked.
    let lowered: Vec<String> = raise_only
        .iter()
        .map(|name| format!("{name}: no raise-only comparison is written for this section"))
        .collect();
    Ok(json!({
        "counts": counted,
        "compared": comparison.compared.values().sum::<usize>(),
        "differences": comparison.differences,
        "pending": pending,
        "sessionMismatch": session_mismatch,
        "raiseOnly": raise_only.len(),
        "lowered": lowered,
    }))
}

fn main() -> ExitCode {
    let mut err = std::io::stderr();
    let Some(mode) = parse() else {
        let _ = writeln!(err, "{USAGE}");
        return ExitCode::FAILURE;
    };
    let (dir, file, outcome) = match &mode {
        Mode::Startup(plant, dir) => (
            dir,
            FIXTURE_FILE,
            plant_startup_failure(*plant, dir, PLANTED_AT)
                .map(|files| json!({ "kind": plant.word(), "files": files })),
        ),
        Mode::Populated { data, repos } => (data, FIXTURE_FILE, plant_populated(data, repos)),
        Mode::Verify(data) => (data, VERIFY_FILE, verify(data)),
    };
    let found = match outcome {
        Ok(found) => found,
        Err(e) => {
            let _ = writeln!(err, "writing {file} failed: {e}");
            return ExitCode::FAILURE;
        }
    };
    if let Err(e) = std::fs::write(dir.join(file), found.to_string()) {
        let _ = writeln!(err, "writing {file} failed: {e}");
        return ExitCode::FAILURE;
    }
    let _ = writeln!(err, "{file}: {found}");
    ExitCode::SUCCESS
}
