#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! [p4] §43's static gates over the core: the `motivation` topic has one emitter, day one has one
//! writer, and the level gates nothing.
//!
//! Each reads `core/src` comment-stripped, so a sentence about a statement is never the
//! statement; prints what it read; and fails at zero, because a gate whose passing run read
//! nothing proved nothing. The walk is hand-rolled like the other source gates, and a file that
//! vanished between the directory read and the file read is skipped before it is counted.

#[allow(dead_code)]
#[path = "support/source_scan.rs"]
mod source_scan;

use std::path::{Path, PathBuf};

use source_scan::strip_comments;

fn core_src() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

/// Every `.rs` file under `core/src`, as (path relative to it with `/` separators,
/// comment-stripped text), sorted by path.
fn core_sources() -> Vec<(String, String)> {
    fn walk(dir: &Path, root: &Path, out: &mut Vec<(String, String)>) {
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
            Err(error) => panic!("{}: {error}", dir.display()),
        };
        for entry in entries {
            let entry = entry.expect("a readable entry");
            let path = entry.path();
            if entry.file_type().expect("a file type").is_dir() {
                walk(&path, root, out);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                match std::fs::read_to_string(&path) {
                    Ok(text) => out.push((
                        path.strip_prefix(root)
                            .unwrap_or(&path)
                            .display()
                            .to_string()
                            .replace('\\', "/"),
                        strip_comments(&text),
                    )),
                    // Gone before it was read: nothing to check, and not counted.
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => panic!("{}: {error}", path.display()),
                }
            }
        }
    }
    let root = core_src();
    let mut out = Vec::new();
    walk(&root, &root, &mut out);
    out.sort();
    out
}

fn squeezed(code: &str) -> String {
    code.chars().filter(|c| !c.is_whitespace()).collect()
}

fn under(path: &str, dirs: &[&str]) -> bool {
    dirs.iter().any(|dir| path.starts_with(dir))
}

/// [p4] §43.15: one publisher, keyed on the connection's change counter, emits the topic. A
/// second emitter would send the band a payload the counter never saw.
#[test]
fn ac_p4_43_1_one_emitter() {
    let sources = core_sources();
    let emitters: Vec<&str> = sources
        .iter()
        .filter(|(_, code)| squeezed(code).contains("emit(\"motivation\","))
        .map(|(path, _)| path.as_str())
        .collect();
    eprintln!(
        "motivation_gates: {} core source file(s) scanned; {emitters:?} emit the motivation topic",
        sources.len()
    );
    assert!(
        !sources.is_empty(),
        "the walk read no file, so it proved nothing"
    );
    assert_eq!(
        emitters,
        ["motivation/publish.rs"],
        "the motivation topic has one emitter"
    );
}

/// A statement that writes a row: a SQL write verb, or a call to one of `app_meta`'s key/value
/// writers.
const WRITES: &[&str] = &[
    "INSERT",
    "UPDATE",
    "REPLACE",
    "DELETE",
    "put(",
    "set_app_meta(",
];

/// [p4] §43.9: day one is written once, by the acknowledgement of the first paint. Reads
/// elsewhere are allowed; a second writer would move every ring and badge that reads it.
#[test]
fn ac_p4_43_14_one_day_one_writer() {
    let sources = core_sources();
    let mut writes: Vec<String> = Vec::new();
    for (path, code) in &sources {
        for statement in code.split([';', '{', '}']) {
            let names_key = statement.contains("KEY_MOTIVATION_DAY_ONE")
                || statement.contains("motivation_day_one");
            if names_key && WRITES.iter().any(|verb| statement.contains(verb)) {
                writes.push(path.clone());
            }
        }
    }
    eprintln!(
        "motivation_gates: {} core source file(s) scanned; day one written at {writes:?}",
        sources.len()
    );
    assert!(
        !sources.is_empty(),
        "the walk read no file, so it proved nothing"
    );
    assert!(
        !writes.is_empty(),
        "no statement writes day one, so the search matched nothing"
    );
    assert!(
        writes
            .iter()
            .all(|path| path == "motivation/acknowledge.rs"),
        "day one has one writer, the acknowledgement: {writes:?}"
    );
}

/// §38's ledger, its floor included, and §43's presence module: the only places the level is
/// read or compared.
const LEVEL_HOMES: &[&str] = &["ledger/", "motivation/"];

const COMPARISONS: &[&str] = &["==", "!=", ">=", "<=", ">", "<"];

fn trimmed(token: &str) -> &str {
    token.trim_matches(|c: char| matches!(c, '(' | ')' | '&' | '*' | '{' | '}' | ',' | ';' | '?'))
}

fn is_integer(token: &str) -> bool {
    let digits = token.strip_prefix('-').unwrap_or(token);
    digits.chars().next().is_some_and(|c| c.is_ascii_digit())
        && digits
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn names_a_level(token: &str) -> bool {
    let last = token.rsplit(['.', ':']).next().unwrap_or(token);
    last.to_ascii_lowercase().contains("level")
}

/// `level <op> <integer>` or the reverse, by tokens: rustfmt spaces every binary comparison and
/// never a generic's angle brackets, so `Vec<u8>` is not an operator here.
fn level_comparisons(code: &str) -> Vec<usize> {
    let mut lines = Vec::new();
    for (index, line) in code.lines().enumerate() {
        let tokens: Vec<&str> = line.split_whitespace().map(trimmed).collect();
        let hit = tokens.windows(3).any(|w| {
            COMPARISONS.contains(&w[1])
                && ((names_a_level(w[0]) && is_integer(w[2]))
                    || (is_integer(w[0]) && names_a_level(w[2])))
        });
        if hit {
            lines.push(index + 1);
        }
    }
    lines
}

/// [p4] §43.12: the level is a presentation. Nothing outside the ledger and the presence module
/// reads it, and nothing compares it with a constant — a threshold would make it a gate.
#[test]
fn ac_p4_43_19_the_level_gates_nothing() {
    let sources = core_sources();
    let mut callers: Vec<&str> = Vec::new();
    let mut compared: Vec<String> = Vec::new();
    for (path, code) in &sources {
        let calls = code.matches("read_level(").count() - code.matches("fn read_level(").count();
        if calls > 0 {
            callers.push(path);
        }
        if !under(path, LEVEL_HOMES) {
            compared.extend(
                level_comparisons(code)
                    .into_iter()
                    .map(|line| format!("{path}:{line}")),
            );
        }
    }
    let stray: Vec<&&str> = callers
        .iter()
        .filter(|path| !under(path, LEVEL_HOMES))
        .collect();
    eprintln!(
        "motivation_gates: {} core source file(s) scanned; read_level called in {callers:?}; \
         {} level comparison(s) outside the ledger and the presence module",
        sources.len(),
        compared.len()
    );
    assert!(
        !sources.is_empty(),
        "the walk read no file, so it proved nothing"
    );
    assert!(
        !callers.is_empty(),
        "no caller of read_level found, so the search matched nothing"
    );
    assert!(
        stray.is_empty(),
        "the level is read outside §38's ledger and §43's presence module: {stray:?}"
    );
    assert!(
        compared.is_empty(),
        "a level is compared with a constant outside them: {compared:?}"
    );
}
