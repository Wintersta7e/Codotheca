//! The three small named files of §10.1a. Nothing here walks a disk: these are parsers, and
//! the whole trust argument of the roots screen is that only these files are read.

use std::path::{Path, PathBuf};

/// Which of the three files a suggestion came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SourceKind {
    GitConfig,
    VsCode,
    JetBrains,
}

impl SourceKind {
    /// The stable machine token that crosses the wire as `RootSuggestion.provenanceDetail`.
    /// The rendered label is shell-owned prose (§2.4) and is not built here.
    #[must_use]
    pub fn detail(self) -> &'static str {
        match self {
            Self::GitConfig => "includeif",
            Self::VsCode => "vscode",
            Self::JetBrains => "jetbrains",
        }
    }
}

/// One path named by one source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceHit {
    pub kind: SourceKind,
    pub path: PathBuf,
    /// True when the path already names a directory *of* repositories, as a `gitdir:` directive
    /// does. False when it names one repository, as an editor's recent list does — those take
    /// their parent.
    pub is_container: bool,
}

/// Every `gitdir:` / `gitdir/i:` pattern in a git config, in file order. `onbranch:` is not a
/// location and is skipped.
#[must_use]
pub fn parse_gitconfig_gitdirs(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("[includeIf ") else {
            continue;
        };
        let Some(inner) = rest.strip_suffix(']') else {
            continue;
        };
        let inner = inner.trim().trim_matches('"');
        let pattern = inner
            .strip_prefix("gitdir:")
            .or_else(|| inner.strip_prefix("gitdir/i:"));
        if let Some(pattern) = pattern {
            if !pattern.is_empty() {
                out.push(pattern.to_owned());
            }
        }
    }
    out
}

/// A `gitdir:` pattern reduced to the directory it names, or `None` when it names no directory.
///
/// A pattern with no separator is git's "match this name anywhere" form; it points at no place
/// on this machine and must not become a scan root.
#[must_use]
pub fn expand_gitdir_pattern(pattern: &str, home: &Path) -> Option<PathBuf> {
    let trimmed = pattern.trim_end_matches("**").trim_end_matches(['/', '\\']);
    if trimmed.is_empty() {
        return None;
    }
    let expanded = if let Some(rest) = trimmed
        .strip_prefix("~/")
        .or_else(|| trimmed.strip_prefix("~\\"))
    {
        home.join(rest.replace('\\', "/"))
    } else if trimmed == "~" {
        return None;
    } else if trimmed.contains('/') || trimmed.contains('\\') {
        PathBuf::from(trimmed)
    } else {
        return None;
    };
    Some(expanded)
}

/// A `file:` URI as a path, or `None` for any other scheme or an authority component.
#[must_use]
pub fn decode_file_uri(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    // `file://server/share` names another machine. Only the empty authority is local.
    let path = rest.strip_prefix('/')?;
    let decoded = percent_decode(path);
    if decoded.is_empty() {
        return None;
    }
    // `/d:/dev/x` is a drive path wearing a leading separator.
    let looks_like_drive = decoded.as_bytes().get(1) == Some(&b':');
    Some(PathBuf::from(if looks_like_drive {
        decoded
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
                let hi = bytes.get(i + 1).and_then(|c| (*c as char).to_digit(16));
                let lo = bytes.get(i + 2).and_then(|c| (*c as char).to_digit(16));
                if let (Some(hi), Some(lo)) = (hi, lo) {
                    out.push(u8::try_from(hi * 16 + lo).unwrap_or(b'?'));
                    i += 3;
                    continue;
                }
                out.push(b'%');
                i += 1;
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

/// Recent workspace folders out of the editor's global-storage JSON, in file order, deduplicated
/// by first appearance. Both shapes the store has carried are read; a single file is not a
/// workspace and is skipped.
#[must_use]
pub fn parse_recent_workspaces_json(bytes: &[u8]) -> Vec<PathBuf> {
    let Ok(root) = serde_json::from_slice::<serde_json::Value>(bytes) else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = Vec::new();
    let mut push = |uri: Option<&str>| {
        if let Some(path) = uri.and_then(decode_file_uri) {
            if !out.contains(&path) {
                out.push(path);
            }
        }
    };
    if let Some(folders) = root
        .pointer("/backupWorkspaces/folders")
        .and_then(|v| v.as_array())
    {
        for entry in folders {
            push(entry.get("folderUri").and_then(|v| v.as_str()));
        }
    }
    if let Some(entries) = root
        .pointer("/openedPathsList/entries")
        .and_then(|v| v.as_array())
    {
        for entry in entries {
            push(entry.get("folderUri").and_then(|v| v.as_str()));
            push(
                entry
                    .pointer("/workspace/configPath")
                    .and_then(|v| v.as_str()),
            );
        }
    }
    out
}

/// Recent project directories out of the recent-projects XML, in file order.
///
/// Only `<entry key="…">` under the recent map is read: `lastProjectLocation` is a *parent* of
/// projects and would make the suggestion one level too shallow.
#[must_use]
pub fn parse_recent_projects_xml(text: &str, home: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find("<entry ") {
        let Some(tail) = rest.get(at + "<entry ".len()..) else {
            break;
        };
        let Some(end) = tail.find('>') else { break };
        let attrs = tail.get(..end).unwrap_or_default();
        if let Some(raw) = attribute(attrs, "key") {
            let expanded = raw.replace("$USER_HOME$", &home.to_string_lossy());
            let path = PathBuf::from(expanded.trim_end_matches(['/', '\\']));
            if path.is_absolute() && !out.contains(&path) {
                out.push(path);
            }
        }
        rest = tail.get(end + 1..).unwrap_or_default();
    }
    out
}

fn attribute<'a>(attrs: &'a str, name: &str) -> Option<&'a str> {
    let needle = format!("{name}=\"");
    let at = attrs.find(&needle)?;
    let tail = attrs.get(at + needle.len()..)?;
    let end = tail.find('"')?;
    tail.get(..end)
}

/// Where the three files live on this machine. Injected rather than read, so a test never
/// touches a real home directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceEnv {
    pub home: PathBuf,
    /// Windows roaming application data, when this is Windows.
    pub app_data: Option<PathBuf>,
    /// `XDG_CONFIG_HOME`, or `~/.config` where that is unset.
    pub xdg_config: Option<PathBuf>,
}

/// Editor product directories under application data or `~/.config`. Names only — the whole
/// point of §10.1a is that this list is short enough to print.
const WORKSPACE_STORE_DIRS: [&str; 3] = ["Code", "Code - Insiders", "VSCodium"];

/// The recent-projects XML lives under a per-product, per-version directory. Only one level of
/// enumeration is performed and only under this parent.
const PROJECT_STORE_PARENT: &str = "JetBrains";

/// Every fixed file or bounded file pattern this subsystem may open, in a fixed order. Nothing
/// outside this list is read.
#[must_use]
pub fn candidate_files(env: &SourceEnv) -> Vec<(SourceKind, PathBuf)> {
    let mut out = Vec::new();
    out.push((SourceKind::GitConfig, env.home.join(".gitconfig")));
    if let Some(config) = &env.xdg_config {
        out.push((SourceKind::GitConfig, config.join("git").join("config")));
    }

    let mut roots: Vec<PathBuf> = Vec::new();
    if let Some(app_data) = &env.app_data {
        roots.push(app_data.clone());
    }
    if let Some(config) = &env.xdg_config {
        roots.push(config.clone());
    }

    for root in &roots {
        for product in WORKSPACE_STORE_DIRS {
            let base = root.join(product).join("User").join("globalStorage");
            out.push((SourceKind::VsCode, base.join("storage.json")));
            out.push((SourceKind::VsCode, base.join("state.vscdb")));
        }
        out.push((
            SourceKind::JetBrains,
            root.join(PROJECT_STORE_PARENT)
                .join("*")
                .join("options")
                .join("recentProjects.xml"),
        ));
    }
    out
}

/// The recent-workspace list out of the editor's key-value store. Opened **read-only**; a store
/// the editor is holding is skipped rather than waited on.
#[must_use]
pub fn read_workspace_store_db(db: &Path) -> Vec<PathBuf> {
    let flags = rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI;
    let Ok(conn) = rusqlite::Connection::open_with_flags(db, flags) else {
        return Vec::new();
    };
    let row: Result<String, _> = conn.query_row(
        "SELECT value FROM ItemTable WHERE key = 'history.recentlyOpenedPathsList'",
        [],
        |r| r.get(0),
    );
    let Ok(json) = row else { return Vec::new() };
    parse_recent_workspaces_json(json.as_bytes())
}

/// Every path the three files name, in source order.
#[must_use]
pub fn collect_hits(env: &SourceEnv) -> Vec<SourceHit> {
    let mut out: Vec<SourceHit> = Vec::new();
    for (kind, path) in candidate_files(env) {
        match kind {
            SourceKind::GitConfig => {
                let Ok(text) = std::fs::read_to_string(&path) else {
                    continue;
                };
                for pattern in parse_gitconfig_gitdirs(&text) {
                    if let Some(dir) = expand_gitdir_pattern(&pattern, &env.home) {
                        out.push(SourceHit {
                            kind,
                            path: dir,
                            is_container: true,
                        });
                    }
                }
            }
            SourceKind::VsCode => {
                let found = if path.extension().is_some_and(|e| e == "vscdb") {
                    read_workspace_store_db(&path)
                } else {
                    std::fs::read(&path)
                        .map(|b| parse_recent_workspaces_json(&b))
                        .unwrap_or_default()
                };
                for project in found {
                    out.push(SourceHit {
                        kind,
                        path: project,
                        is_container: false,
                    });
                }
            }
            SourceKind::JetBrains => {
                for xml in recent_project_files(&path) {
                    let Ok(text) = std::fs::read_to_string(&xml) else {
                        continue;
                    };
                    for project in parse_recent_projects_xml(&text, &env.home) {
                        out.push(SourceHit {
                            kind,
                            path: project,
                            is_container: false,
                        });
                    }
                }
            }
        }
    }
    out
}

/// Resolve the one wildcard level in a recent-project file pattern. Nothing recurses.
fn recent_project_files(pattern: &Path) -> Vec<PathBuf> {
    let Some(parent) = pattern
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
    else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(parent) else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path().join("options").join("recentProjects.xml"))
        .filter(|p| p.is_file())
        .collect();
    out.sort();
    out
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    // §10.1a: an includeIf gitdir directive is the user's own declaration of where
    // repositories live, already split by context. It is the strongest provenance there is.
    #[test]
    fn gitdir_directives_are_read_out_of_a_config() {
        let text = concat!(
            "[user]\n\temail = someone@example.invalid\n",
            "[includeIf \"gitdir:~/work/\"]\n\tpath = ~/work/.gitconfig\n",
            "[includeIf \"gitdir/i:D:/dev/\"]\n\tpath = dev.gitconfig\n",
            "[includeIf \"onbranch:main\"]\n\tpath = main.gitconfig\n",
        );
        assert_eq!(parse_gitconfig_gitdirs(text), vec!["~/work/", "D:/dev/"]);
    }

    #[test]
    fn a_gitdir_pattern_expands_to_a_directory() {
        let home = Path::new("/home/u");
        assert_eq!(
            expand_gitdir_pattern("~/work/", home),
            Some(PathBuf::from("/home/u/work"))
        );
        assert_eq!(
            expand_gitdir_pattern("~/work/**", home),
            Some(PathBuf::from("/home/u/work"))
        );
        assert_eq!(
            expand_gitdir_pattern("D:/dev/", home),
            Some(PathBuf::from("D:/dev"))
        );
        // A bare name is a suffix match against any repository anywhere, not a location.
        assert_eq!(expand_gitdir_pattern("proj/", home), None);
        assert_eq!(expand_gitdir_pattern("", home), None);
    }

    #[test]
    fn a_file_uri_decodes_including_percent_escapes_and_a_drive_letter() {
        assert_eq!(
            decode_file_uri("file:///home/u/a%20b"),
            Some(PathBuf::from("/home/u/a b"))
        );
        assert_eq!(
            decode_file_uri("file:///d%3A/dev/x"),
            Some(PathBuf::from("d:/dev/x"))
        );
        assert_eq!(decode_file_uri("vscode-remote://x/y"), None);
        assert_eq!(decode_file_uri("file://server/share"), None);
    }

    #[test]
    fn recent_workspaces_are_read_from_both_shapes_the_store_has_carried() {
        let json = br#"{
          "backupWorkspaces": { "folders": [ { "folderUri": "file:///home/u/dev/one" } ] },
          "windowsState": {},
          "openedPathsList": { "entries": [
             { "folderUri": "file:///home/u/dev/two" },
             { "fileUri": "file:///home/u/notes.md" },
             { "workspace": { "configPath": "file:///home/u/dev/three/a.code-workspace" } }
          ] }
        }"#;
        assert_eq!(
            parse_recent_workspaces_json(json),
            vec![
                PathBuf::from("/home/u/dev/one"),
                PathBuf::from("/home/u/dev/two"),
                PathBuf::from("/home/u/dev/three/a.code-workspace"),
            ]
        );
    }

    /// An absolute home for the target being tested.
    ///
    /// `/home/u` is **not absolute on Windows** — it has no drive — and the parsers drop a
    /// non-absolute candidate, which is correct behaviour. A Unix-only literal here therefore
    /// makes the whole test vacuous on the target this ships to first.
    fn test_home() -> PathBuf {
        if cfg!(windows) {
            PathBuf::from(r"C:\home\u")
        } else {
            PathBuf::from("/home/u")
        }
    }

    /// An absolute path that is not under the home, for the same reason.
    fn other_absolute() -> PathBuf {
        if cfg!(windows) {
            PathBuf::from(r"C:\srv\beta")
        } else {
            PathBuf::from("/srv/beta")
        }
    }

    #[test]
    fn recent_projects_xml_yields_project_directories_with_the_home_macro_expanded() {
        let home = test_home();
        let other = other_absolute();
        let xml = format!(
            "<application><component name=\"RecentProjectsManager\">\
             <option name=\"additionalInfo\"><map>\
             <entry key=\"$USER_HOME$/dev/alpha\"><value/></entry>\
             <entry key=\"{}\"><value/></entry>\
             </map></option>\
             <option name=\"lastProjectLocation\" value=\"$USER_HOME$/dev\" />\
             </component></application>",
            other.display()
        );
        assert_eq!(
            parse_recent_projects_xml(&xml, &home),
            vec![home.join("dev").join("alpha"), other]
        );
    }

    #[test]
    fn malformed_input_yields_nothing_rather_than_an_error() {
        assert!(parse_recent_workspaces_json(b"not json").is_empty());
        assert!(parse_recent_projects_xml("<unclosed", Path::new("/home/u")).is_empty());
        assert!(parse_gitconfig_gitdirs("").is_empty());
    }

    #[test]
    fn the_candidate_list_is_short_and_is_the_whole_read_surface() {
        let home = test_home();
        let env = SourceEnv {
            home: home.clone(),
            app_data: Some(home.join("AppData").join("Roaming")),
            xdg_config: Some(home.join(".config")),
        };
        let files = candidate_files(&env);
        let names: Vec<String> = files
            .iter()
            .map(|(_, p)| p.to_string_lossy().replace('\\', "/"))
            .collect();

        let home_s = home.to_string_lossy().replace('\\', "/");
        assert!(names.contains(&format!("{home_s}/.gitconfig")), "{names:?}");
        assert!(
            names.contains(&format!("{home_s}/.config/git/config")),
            "{names:?}"
        );
        assert!(names
            .iter()
            .any(|n| n.ends_with("/User/globalStorage/storage.json")));
        assert!(names
            .iter()
            .any(|n| n.ends_with("/User/globalStorage/state.vscdb")));
        assert!(names
            .iter()
            .any(|n| n.ends_with("/options/recentProjects.xml")));

        // The claim on the screen is that a *named set* is read. A directory walk here would
        // make that claim false, so the list is finite and short.
        assert!(files.len() <= 24, "candidate list grew to {}", files.len());
        for (_, path) in &files {
            assert!(path.is_absolute(), "{path:?} is not absolute");
        }
    }

    #[test]
    fn a_missing_home_yields_a_list_and_never_a_panic() {
        let env = SourceEnv {
            home: PathBuf::from("/nonexistent"),
            app_data: None,
            xdg_config: None,
        };
        assert!(!candidate_files(&env).is_empty());
        assert!(collect_hits(&env).is_empty());
    }

    #[test]
    fn a_gitdir_hit_is_a_container_and_an_editor_hit_is_not() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        std::fs::write(
            home.join(".gitconfig"),
            "[includeIf \"gitdir:~/work/\"]\n\tpath = w\n",
        )
        .unwrap();
        let env = SourceEnv {
            home: home.to_path_buf(),
            app_data: None,
            xdg_config: None,
        };
        let hits = collect_hits(&env);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].kind, SourceKind::GitConfig);
        assert!(hits[0].is_container);
        assert_eq!(hits[0].path, home.join("work"));
    }
}
