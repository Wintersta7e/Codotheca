//! Reading the core's own source for the gates that hold a rule in code rather than in a test of
//! behaviour. Comments are stripped before any match, so a sentence about a statement is never
//! the statement.

use std::path::Path;

/// `source` with everything from `//` to the end of each line removed — line, doc and inner doc
/// comments alike.
pub(crate) fn strip_comments(source: &str) -> String {
    source
        .lines()
        .map(|line| line.split("//").next().unwrap_or_default())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Every `.rs` file under `core/<relative>`, walked recursively, as `(path relative to core/src
/// with `/` separators, comment-stripped text)`, sorted by path.
pub(crate) fn rust_sources(relative: &str) -> Vec<(String, String)> {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let src = manifest.join("src");
    let mut out = Vec::new();
    let mut pending = vec![manifest.join(relative)];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir)
            .unwrap_or_else(|e| panic!("{} must exist: {e}", dir.display()))
            .flatten()
        {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let name = path
                    .strip_prefix(&src)
                    .unwrap()
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join("/");
                out.push((
                    name,
                    strip_comments(&std::fs::read_to_string(&path).unwrap()),
                ));
            }
        }
    }
    out.sort();
    out
}

/// `text` upper-cased with every whitespace run collapsed to one space, so a SQL statement split
/// across lines or indented matches its one-line spelling.
pub(crate) fn sql_normal(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_uppercase()
}

/// Every decimal integer literal in `source`, its underscores and type suffix removed: a digit run
/// not preceded by an identifier character or a `.`, and not followed by a fraction.
pub(crate) fn integer_literals(source: &str) -> Vec<i128> {
    let bytes = source.as_bytes();
    let mut found = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let starts = bytes[i].is_ascii_digit()
            && (i == 0
                || !(bytes[i - 1].is_ascii_alphanumeric() || matches!(bytes[i - 1], b'_' | b'.')));
        if !starts {
            i += 1;
            continue;
        }
        let mut end = i;
        while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
            end += 1;
        }
        let token = source.get(i..end).unwrap();
        let fraction =
            bytes.get(end) == Some(&b'.') && bytes.get(end + 1).is_some_and(u8::is_ascii_digit);
        let digits: String = token
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '_')
            .filter(char::is_ascii_digit)
            .collect();
        let hex = token.starts_with("0x") || token.starts_with("0b") || token.starts_with("0o");
        if !fraction && !hex {
            if let Ok(value) = digits.parse() {
                found.push(value);
            }
        }
        i = end;
    }
    found
}
