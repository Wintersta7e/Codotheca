#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! §39.4–§39.6 and §39.10 as source scans: each ring reads only its own ledger, the active day is
//! a union of three day sets and nothing else, the deletion analyser never reads Ship's push
//! columns, and no file of the rings module writes a ledger row or names a notification.
//!
//! Every scan prints how many files and bytes it read and fails at zero: a scan that read nothing
//! would pass while proving nothing.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use codotheca_core::rings::active::union_days;

/// A file under the crate root, with its text. A missing file fails the scan by name.
fn source(relative: &str) -> (PathBuf, String) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(relative);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{} must exist: {e}", path.display()));
    (path, text)
}

/// Every `.rs` file under `relative` (a directory, walked recursively, or one file).
fn sources_under(relative: &str) -> Vec<(PathBuf, String)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join(relative);
    if root.is_file() {
        return vec![source(relative)];
    }
    let mut out = Vec::new();
    let mut pending = vec![root];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir)
            .unwrap_or_else(|e| panic!("{} must exist: {e}", dir.display()))
            .flatten()
        {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let text = std::fs::read_to_string(&path).unwrap();
                out.push((path, text));
            }
        }
    }
    out.sort();
    out
}

/// The ledger tokens a text names: every table after `FROM`, `JOIN`, `INTO` or `UPDATE`, every
/// ledger kind literal, and every Ship push column.
fn ledger_tokens(text: &str) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    let words: Vec<&str> = text.split_whitespace().collect();
    for pair in words.windows(2) {
        if matches!(pair[0], "FROM" | "JOIN" | "INTO" | "UPDATE") {
            let table: String = pair[1]
                .chars()
                .take_while(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '_')
                .collect();
            if table.starts_with(|c: char| c.is_ascii_lowercase()) {
                found.insert(table);
            }
        }
    }
    for literal in [
        "'commit_day'",
        "'debt_day'",
        "'focus'",
        "push_record",
        "last_push_at",
        "last_push_tz_offset_min",
    ] {
        if text.contains(literal) {
            found.insert(literal.to_owned());
        }
    }
    found
}

/// One ring reader: its files, the tokens it may name, and a table it may name only beside a kind.
struct Reader {
    name: &'static str,
    files: &'static [&'static str],
    may: &'static [&'static str],
    only_with: Option<(&'static str, &'static str)>,
}

/// §39.4–§39.6: one ledger per reader. `project`, `location` and `app_meta` are context any
/// reader may name; `xp_events` is a ledger only with its kind.
const READERS: &[Reader] = &[
    Reader {
        name: "Focus",
        files: &["src/rings/focus.rs", "src/session/measure.rs"],
        may: &[
            "session",
            "session_segment",
            "project",
            "location",
            "app_meta",
        ],
        only_with: None,
    },
    Reader {
        name: "Ship",
        files: &["src/rings/ship.rs"],
        may: &[
            "location",
            "push_record",
            "last_push_at",
            "last_push_tz_offset_min",
            "project",
            "app_meta",
        ],
        only_with: None,
    },
    Reader {
        name: "Tend",
        files: &["src/rings/tend.rs"],
        may: &[
            "debt_item",
            "debt_sweep",
            "xp_events",
            "'debt_day'",
            "project",
            "location",
            "app_meta",
        ],
        only_with: Some(("xp_events", "'debt_day'")),
    },
    Reader {
        name: "(g)",
        files: &["src/rings/commit_days.rs"],
        may: &[
            "xp_events",
            "'commit_day'",
            "project",
            "location",
            "app_meta",
        ],
        only_with: Some(("xp_events", "'commit_day'")),
    },
];

#[test]
fn ac_p4_39_1_each_reader_names_only_its_ledger() {
    let (_, measure) = source("src/session/measure.rs");
    assert!(
        measure.contains("pub fn focus_seconds_by_day("),
        "src/session/measure.rs no longer defines `focus_seconds_by_day`, so the Focus scan \
         reads the wrong file"
    );
    for reader in READERS {
        let mut found = BTreeSet::new();
        let mut bytes = 0;
        for file in reader.files {
            let (_, text) = source(file);
            bytes += text.len();
            found.extend(ledger_tokens(&text));
        }
        eprintln!(
            "{}: {} files, {bytes} bytes, tokens {found:?}",
            reader.name,
            reader.files.len()
        );
        assert!(
            !reader.files.is_empty() && bytes > 0,
            "{}: the scan read nothing",
            reader.name
        );
        for token in &found {
            assert!(
                reader.may.contains(&token.as_str()),
                "{}: names `{token}`, which is not its ledger",
                reader.name
            );
        }
        if let Some((table, kind)) = reader.only_with {
            assert!(
                !found.contains(table) || found.contains(kind),
                "{}: names `{table}` without `{kind}`",
                reader.name
            );
        }
    }
}

/// A deterministic stream of day sets, so a failing case reproduces.
struct Days(u64);

impl Days {
    const fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn set(&mut self) -> BTreeSet<i64> {
        let len = self.next() % 12;
        (0..len)
            .map(|_| i64::try_from(self.next() % 40).unwrap() + 20_700)
            .collect()
    }
}

#[test]
fn ac_p4_39_1_the_union_takes_three_day_sets() {
    let (_, text) = source("src/rings/active.rs");
    eprintln!(
        "active.rs: 1 file, {} bytes, tokens {:?}",
        text.len(),
        ledger_tokens(&text)
    );
    assert!(!text.is_empty(), "active.rs: the scan read nothing");
    assert!(
        ledger_tokens(&text).is_empty(),
        "active.rs names a ledger: {:?}",
        ledger_tokens(&text)
    );

    let (_, rest) = text
        .split_once("pub fn union_days(")
        .expect("active.rs defines `union_days`");
    let (signature, _) = rest.split_once('{').unwrap();
    let (params, returns) = signature.rsplit_once(')').unwrap();
    let params: Vec<&str> = params
        .split(',')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect();
    eprintln!("union_days({signature}");
    assert_eq!(
        params.len(),
        3,
        "union_days takes three day sets: {signature}"
    );
    for param in &params {
        let ty = param.split_once(':').map(|(_, ty)| ty.trim());
        assert_eq!(ty, Some("&BTreeSet<i64>"), "{signature}");
    }
    assert_eq!(returns.trim(), "-> BTreeSet<i64>", "{signature}");

    let mut days = Days(0x9E37_79B9_7F4A_7C15);
    for case in 0..500 {
        let (g, s, t) = (days.set(), days.set(), days.set());
        let expected: BTreeSet<i64> = g.union(&s).copied().collect();
        let expected: BTreeSet<i64> = expected.union(&t).copied().collect();
        assert_eq!(union_days(&g, &s, &t), expected, "case {case}");
    }
}

/// §45's deletion analyser and the two acts it gates.
const ANALYSER_ROOTS: &[&str] = &[
    "src/uninstall",
    "src/removal",
    "src/analyser",
    "src/git/analyse.rs",
];

#[test]
fn ac_p4_39_1_the_analyser_never_reads_the_push_columns() {
    for root in ANALYSER_ROOTS {
        let files = sources_under(root);
        let bytes: usize = files.iter().map(|(_, text)| text.len()).sum();
        eprintln!("{root}: {} files, {bytes} bytes", files.len());
        assert!(
            !files.is_empty() && bytes > 0,
            "{root}: the scan read nothing"
        );
        for (path, text) in &files {
            for column in ["push_record", "last_push_at", "last_push_tz_offset_min"] {
                assert!(
                    !text.contains(column),
                    "{}: the analyser reads Ship's `{column}`",
                    path.display()
                );
            }
        }
    }
}

/// The ledger's writers, each a `pub fn` somewhere in the crate.
const WRITERS: &[(&str, &str)] = &[
    ("src/debt/xp.rs", "pay_debt_day"),
    ("src/ledger/focus.rs", "credit_segment_close"),
    ("src/ledger/verdict.rs", "pay_verdict_award"),
    ("src/ledger/verdict.rs", "pay_prepass_award"),
];

#[test]
fn ac_p4_39_12_the_rings_module_writes_no_ledger_row() {
    for (file, writer) in WRITERS {
        let (_, text) = source(file);
        assert!(
            text.contains(&format!("pub fn {writer}(")),
            "{file} no longer defines `{writer}`; the scan below would ban a name nothing has"
        );
    }
    let files = sources_under("src/rings");
    let bytes: usize = files.iter().map(|(_, text)| text.len()).sum();
    eprintln!("src/rings: {} files, {bytes} bytes", files.len());
    assert!(
        !files.is_empty() && bytes > 0,
        "src/rings: the scan read nothing"
    );
    for (path, text) in &files {
        let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
        for write in [
            "INSERT INTO xp_events",
            "UPDATE xp_events",
            "DELETE FROM xp_events",
        ] {
            assert!(
                !flat.contains(write),
                "{}: the rings module writes the ledger (`{write}`)",
                path.display()
            );
        }
        for (_, writer) in WRITERS {
            assert!(
                !text.contains(writer),
                "{}: the rings module calls the ledger writer `{writer}`",
                path.display()
            );
        }
        let lower = text.to_lowercase();
        for notice in ["notif", "eventsink", ".emit("] {
            assert!(
                !lower.contains(notice),
                "{}: the rings module names a notification (`{notice}`)",
                path.display()
            );
        }
    }
}
