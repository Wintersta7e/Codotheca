//! `codotheca-corpus` — materialise the test corpus (§15.1).
//!
//! Deterministic and re-runnable. Every fixture is described by shape; nothing here names any
//! project, and every path is composed from `--out` at run time.
//!
//! Every line this binary prints goes to **stderr**. §2.1 gives stdout to protocol frames and
//! nothing else, the crate denies `clippy::print_stdout`, and `check-stdout-discipline.mjs`
//! lints `core/src` as a whole — a generator that wrote its summary to stdout would fail all
//! three, and the CI step below pipes it beside the core's own output.

#![forbid(unsafe_code)]

use std::io::Write;

use codotheca_core::corpus::{ensure, CorpusOptions};

const USAGE: &str = "usage: codotheca-corpus --out <dir> [--only a,b] [--force] [--large] \
                     [--untracked-files N] [--deep-history-commits N]";

/// What the argument parser produced, or the exit code to leave with.
struct Parsed {
    root: String,
    options_only: Option<Vec<String>>,
    force: bool,
    large: bool,
    untracked: Option<u32>,
    deep: Option<u32>,
}

fn parse(err: &mut impl Write) -> Result<Parsed, std::process::ExitCode> {
    let mut args = std::env::args().skip(1);
    let mut root: Option<String> = None;
    let mut parsed = Parsed {
        root: String::new(),
        options_only: None,
        force: false,
        large: false,
        untracked: None,
        deep: None,
    };

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--out" => root = args.next(),
            "--only" => {
                parsed.options_only = args
                    .next()
                    .map(|v| v.split(',').map(str::trim).map(str::to_owned).collect());
            }
            "--force" => parsed.force = true,
            "--large" => parsed.large = true,
            "--untracked-files" => parsed.untracked = args.next().and_then(|v| v.parse().ok()),
            "--deep-history-commits" => parsed.deep = args.next().and_then(|v| v.parse().ok()),
            "--help" | "-h" => {
                let _ = writeln!(err, "{USAGE}");
                return Err(std::process::ExitCode::SUCCESS);
            }
            other => {
                let _ = writeln!(err, "unknown argument {other}\n{USAGE}");
                return Err(std::process::ExitCode::FAILURE);
            }
        }
    }

    if let Some(root) = root {
        parsed.root = root;
        Ok(parsed)
    } else {
        let _ = writeln!(err, "{USAGE}");
        Err(std::process::ExitCode::FAILURE)
    }
}

fn main() -> std::process::ExitCode {
    let mut err = std::io::stderr();
    let parsed = match parse(&mut err) {
        Ok(parsed) => parsed,
        Err(code) => return code,
    };

    let mut options = CorpusOptions::new(parsed.root);
    options.only = parsed.options_only;
    options.force = parsed.force;
    options.large = parsed.large;
    if let Some(n) = parsed.untracked {
        options.untracked_files = n;
    }
    if let Some(n) = parsed.deep {
        options.deep_history_commits = n;
    }

    match ensure(&options) {
        Ok(manifest) => {
            let built = manifest.fixtures.iter().filter(|f| f.materialised).count();
            let skipped = manifest.fixtures.len() - built;
            let _ = writeln!(
                err,
                "{} fixtures, {skipped} skipped, git {} -> {}",
                built,
                manifest.git_version,
                manifest.root.join("manifest.json").display()
            );
            for fixture in manifest.fixtures.iter().filter(|f| !f.materialised) {
                let reason = fixture.skip_reason.as_deref().unwrap_or("unknown");
                let _ = writeln!(err, "skipped {}: {reason}", fixture.name);
            }
            std::process::ExitCode::SUCCESS
        }
        Err(e) => {
            let _ = writeln!(err, "corpus failed: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}
