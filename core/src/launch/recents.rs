//! §4bis.2's strong evidence: which projects an editor has actually opened.
//!
//! Every parser here is pure over bytes or text, so a format change is a unit test away rather
//! than a machine away. Only [`read_sqlite_state`] and [`recent_paths`] touch the filesystem.

use std::path::{Path, PathBuf};

use crate::launch::catalogue::RecentsSource;

/// `file:///c%3A/x` and a remote-authority URI both reduce to a path.
#[must_use]
pub fn file_uri_to_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.split_once("://").map_or(uri, |(_, r)| r);
    let body = rest.split_once('/').map_or(rest, |(_, b)| b);
    let decoded = percent_decode(body);
    if decoded.is_empty() {
        return None;
    }
    // A decoded leading drive letter means the separator is a backslash.
    let mut chars = decoded.chars();
    let looks_windows =
        matches!((chars.next(), chars.next()), (Some(c), Some(':')) if c.is_ascii_alphabetic());
    Some(PathBuf::from(if looks_windows {
        decoded.replace('/', "\\")
    } else {
        format!("/{decoded}")
    }))
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes.get(i) {
            Some(b'%') => {
                let hex = input
                    .get(i + 1..i + 3)
                    .and_then(|h| u8::from_str_radix(h, 16).ok());
                if let Some(byte) = hex {
                    out.push(byte);
                    i += 3;
                } else {
                    // A stray `%` that is not a valid escape stays a literal `%`.
                    out.push(b'%');
                    i += 1;
                }
            }
            Some(byte) => {
                out.push(*byte);
                i += 1;
            }
            None => break,
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Folder entries only: a recently opened *file* says nothing about which repositories an
/// editor is used for, which is the only question §4bis.2 step 1 asks.
#[must_use]
pub fn parse_vscode_recents(json: &str) -> Vec<PathBuf> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    value
        .get("entries")
        .and_then(serde_json::Value::as_array)
        .map_or_else(Vec::new, |entries| {
            entries
                .iter()
                .filter_map(|e| e.get("folderUri").and_then(serde_json::Value::as_str))
                .filter_map(file_uri_to_path)
                .collect()
        })
}

/// A byte scan for `key="…"`, so no XML dependency enters the core for one options file.
#[must_use]
pub fn parse_jetbrains_recents(xml: &str, user_home: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some(at) = rest.find("key=\"") {
        let Some(tail) = rest.get(at + 5..) else {
            break;
        };
        let Some(end) = tail.find('"') else { break };
        let Some(raw) = tail.get(..end) else { break };
        if raw.starts_with("$USER_HOME$/") || raw.starts_with('/') || raw.contains(":\\") {
            out.push(PathBuf::from(raw.replace("$USER_HOME$", user_home)));
        }
        rest = tail.get(end + 1..).unwrap_or_default();
    }
    out
}

/// The strings in the array `pointer` names in `json`, as paths. Empty when the JSON does not
/// parse or the pointer names no array.
#[must_use]
pub fn parse_json_list(json: &str, pointer: &str) -> Vec<PathBuf> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    value
        .pointer(pointer)
        .and_then(serde_json::Value::as_array)
        .map_or_else(Vec::new, |a| {
            a.iter()
                .filter_map(serde_json::Value::as_str)
                .map(PathBuf::from)
                .collect()
        })
}

/// The `ItemTable` value stored under `key` in a foreign state database; `None` when the file,
/// the table or the key is absent, or the database will not open.
///
/// Opens a **foreign** database read-only. This is not a second writer: §1.10's one-writer
/// invariant covers Codotheca's index, and this connection is `mode=ro`, is never written, and
/// is dropped before the function returns.
#[must_use]
pub fn read_sqlite_state(db: &Path, key: &str) -> Option<String> {
    if !db.is_file() {
        return None;
    }
    let uri = format!("file:{}?mode=ro&immutable=1", db.display());
    let conn = rusqlite::Connection::open_with_flags(
        uri,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
    )
    .ok()?;
    conn.query_row("SELECT value FROM ItemTable WHERE key = ?1", [key], |row| {
        row.get::<_, String>(0)
    })
    .ok()
}

/// The folders an application records having opened, read from its `source` under
/// `config_root`. Empty when it records none or the record cannot be read.
#[must_use]
pub fn recent_paths(source: RecentsSource, config_root: &Path, user_home: &str) -> Vec<PathBuf> {
    match source {
        RecentsSource::None => Vec::new(),
        RecentsSource::SqliteState { rel, key } => read_sqlite_state(&config_root.join(rel), key)
            .map_or_else(Vec::new, |s| parse_vscode_recents(&s)),
        RecentsSource::JetBrainsXml { rel } => std::fs::read_to_string(config_root.join(rel))
            .map_or_else(|_| Vec::new(), |s| parse_jetbrains_recents(&s, user_home)),
        RecentsSource::JsonList { rel, pointer } => std::fs::read_to_string(config_root.join(rel))
            .map_or_else(|_| Vec::new(), |s| parse_json_list(&s, pointer)),
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use super::*;
    use std::path::PathBuf;

    #[test]
    fn folder_uris_become_paths_and_file_entries_are_ignored() {
        let json = r#"{"entries":[
            {"folderUri":"file:///home/u/one"},
            {"fileUri":"file:///home/u/a.txt"},
            {"folderUri":"vscode-remote://wsl%2Bdistro-a/home/u/two"}
        ]}"#;
        let out = parse_vscode_recents(json);
        assert_eq!(
            out,
            vec![PathBuf::from("/home/u/one"), PathBuf::from("/home/u/two")]
        );
    }

    #[test]
    fn a_percent_encoded_windows_file_uri_decodes() {
        assert_eq!(
            file_uri_to_path("file:///c%3A/code/my%20thing"),
            Some(PathBuf::from(r"c:\code\my thing"))
        );
    }

    #[test]
    fn the_jetbrains_options_file_expands_its_home_placeholder() {
        let xml = r#"<application><component name="RecentProjectsManager"><option name="additionalInfo">
            <map><entry key="$USER_HOME$/code/one" /><entry key="/srv/two" /></map>
        </option></component></application>"#;
        assert_eq!(
            parse_jetbrains_recents(xml, "/home/u"),
            vec![PathBuf::from("/home/u/code/one"), PathBuf::from("/srv/two")]
        );
    }

    #[test]
    fn a_json_pointer_list_is_read_and_a_missing_pointer_yields_nothing() {
        let json = r#"{"folder_history":["/a","/b"],"other":1}"#;
        assert_eq!(
            parse_json_list(json, "/folder_history"),
            vec![PathBuf::from("/a"), PathBuf::from("/b")]
        );
        assert!(parse_json_list(json, "/absent").is_empty());
        assert!(parse_json_list("not json", "/folder_history").is_empty());
    }

    #[test]
    fn a_state_database_is_read_read_only_and_a_missing_key_is_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("state.vscdb");
        let conn = rusqlite::Connection::open(&db).unwrap();
        conn.execute_batch(
            "CREATE TABLE ItemTable (key TEXT PRIMARY KEY, value BLOB);
             INSERT INTO ItemTable VALUES ('history.recentlyOpenedPathsList', '{\"entries\":[]}');",
        )
        .unwrap();
        drop(conn);
        assert_eq!(
            read_sqlite_state(&db, "history.recentlyOpenedPathsList").as_deref(),
            Some("{\"entries\":[]}")
        );
        assert_eq!(read_sqlite_state(&db, "nothing.here"), None);
        assert_eq!(
            read_sqlite_state(&dir.path().join("absent.vscdb"), "k"),
            None
        );
    }
}
