//! `codotheca-sidecar-fixture` — plant an index the core cannot open, for the end-to-end runs of
//! the startup report windows (§11.2a).
//!
//! Every line this binary prints goes to **stderr**: §2.1 gives stdout to protocol frames and
//! nothing else, and the crate denies `clippy::print_stdout`. What it planted is written to
//! `<data-dir>/fixture.json`, which is what a spec reads.

#![forbid(unsafe_code)]

use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use codotheca_core::testing::sidecar::{plant_startup_failure, StartupPlant};

const USAGE: &str =
    "usage: codotheca-sidecar-fixture plant --kind corrupt|future|migration-failed --data-dir <dir>";

/// The result file, beside the planted index.
const FIXTURE_FILE: &str = "fixture.json";

/// A fixed clock: the plant is test data, and no window this feeds prints the time it was made.
const PLANTED_AT: i64 = 1_700_000_000;

fn parse() -> Option<(StartupPlant, PathBuf)> {
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() != Some("plant") {
        return None;
    }
    let (mut plant, mut dir) = (None, None);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--kind" => plant = args.next().as_deref().and_then(StartupPlant::from_word),
            "--data-dir" => dir = args.next().map(PathBuf::from),
            _ => return None,
        }
    }
    Some((plant?, dir?))
}

fn main() -> ExitCode {
    let mut err = std::io::stderr();
    let Some((plant, dir)) = parse() else {
        let _ = writeln!(err, "{USAGE}");
        return ExitCode::FAILURE;
    };
    let files = match plant_startup_failure(plant, &dir, PLANTED_AT) {
        Ok(files) => files,
        Err(e) => {
            let _ = writeln!(err, "plant {} failed: {e}", plant.word());
            return ExitCode::FAILURE;
        }
    };
    let result = serde_json::json!({ "kind": plant.word(), "files": files });
    if let Err(e) = std::fs::write(dir.join(FIXTURE_FILE), result.to_string()) {
        let _ = writeln!(err, "writing {FIXTURE_FILE} failed: {e}");
        return ExitCode::FAILURE;
    }
    let _ = writeln!(err, "planted {}: {}", plant.word(), files.join(", "));
    ExitCode::SUCCESS
}
