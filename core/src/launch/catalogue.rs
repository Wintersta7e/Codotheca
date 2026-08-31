//! The applications this build knows how to launch: kind, display name, argv, wait mode, cwd
//! mode and where each records the projects it has opened.

/// R31: `TargetKind` and `CwdMode` are declared in `protocol/schema/protocol.json` and
/// generated into `crate::protocol`. Re-exported here so `launch::catalogue::*` and
/// `launch::{CwdMode, TargetKind}` still name them, and hand-written nowhere — a second copy
/// compiles, because the modules differ, and then drifts from the wire form silently.
///
/// `as_str` and `parse` stay as **inherent impls on the generated types**, which is legal
/// because both modules are in this crate. The generated enums carry serde attributes and no
/// methods, and these are how a kind or a cwd mode reaches and returns from SQLite.
pub use crate::protocol::{CwdMode, TargetKind};

impl TargetKind {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            TargetKind::Editor => "editor",
            TargetKind::Terminal => "terminal",
            TargetKind::FileManager => "file_manager",
            TargetKind::GitClient => "git_client",
        }
    }
    #[must_use]
    pub fn parse(s: &str) -> Option<TargetKind> {
        match s {
            "editor" => Some(TargetKind::Editor),
            "terminal" => Some(TargetKind::Terminal),
            "file_manager" => Some(TargetKind::FileManager),
            "git_client" => Some(TargetKind::GitClient),
            _ => None,
        }
    }
}

impl CwdMode {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            CwdMode::Location => "location",
            CwdMode::None => "none",
        }
    }
    #[must_use]
    pub fn parse(s: &str) -> Option<CwdMode> {
        match s {
            "location" => Some(CwdMode::Location),
            "none" => Some(CwdMode::None),
            _ => None,
        }
    }
}

/// How, if at all, this tool's process exit marks the end of a session (§9, mechanism 1).
///
/// It lives here rather than in a `launch_target` column because waiting is a property of the
/// tool and of the argv it was given, not of the user's choice: deriving it from the
/// executable's stem and from whether the row's argv still carries the flag needs no migration
/// and self-corrects when the arguments are edited.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitMode {
    Never,
    Always,
    Flag(&'static str),
}

/// Where this application records the projects it has opened (§4bis.2, step 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecentsSource {
    None,
    SqliteState {
        rel: &'static str,
        key: &'static str,
    },
    JetBrainsXml {
        rel: &'static str,
    },
    JsonList {
        rel: &'static str,
        pointer: &'static str,
    },
}

#[derive(Debug, Clone, Copy)]
pub struct CatalogueEntry {
    pub binaries: &'static [&'static str],
    pub name: &'static str,
    pub kind: TargetKind,
    pub args: &'static [&'static str],
    pub wait: WaitMode,
    pub cwd: CwdMode,
    pub recents: RecentsSource,
    /// Relative to the platform's per-user config or data root; probed for mtime (step 2).
    pub config_dirs: &'static [&'static str],
    /// §4bis.4: the argv used when the location lives inside a distro. `{distro}` and
    /// `{path}` are the two substitutions the argv builder performs.
    pub wsl_args: Option<&'static [&'static str]>,
}

pub const CATALOGUE: &[CatalogueEntry] = &[
    CatalogueEntry {
        binaries: &["code", "code-insiders", "codium"],
        name: "Code",
        kind: TargetKind::Editor,
        args: &["--wait", "{path}"],
        wait: WaitMode::Flag("--wait"),
        cwd: CwdMode::Location,
        recents: RecentsSource::SqliteState {
            rel: "User/globalStorage/state.vscdb",
            key: "history.recentlyOpenedPathsList",
        },
        config_dirs: &["Code/User", "VSCodium/User"],
        wsl_args: Some(&["--remote", "wsl+{distro}", "{path}"]),
    },
    CatalogueEntry {
        binaries: &["nvim", "vim", "hx", "helix"],
        name: "Terminal editor",
        kind: TargetKind::Editor,
        args: &["{path}"],
        wait: WaitMode::Always,
        cwd: CwdMode::Location,
        recents: RecentsSource::None,
        config_dirs: &["nvim", "helix"],
        wsl_args: None,
    },
    CatalogueEntry {
        binaries: &["subl", "sublime_text"],
        name: "Sublime Text",
        kind: TargetKind::Editor,
        args: &["{path}"],
        wait: WaitMode::Flag("--wait"),
        cwd: CwdMode::Location,
        recents: RecentsSource::JsonList {
            rel: "Local/Session.sublime_session",
            pointer: "/folder_history",
        },
        config_dirs: &["sublime-text"],
        wsl_args: None,
    },
    CatalogueEntry {
        binaries: &[
            "idea",
            "clion",
            "pycharm",
            "rustrover",
            "rider",
            "webstorm",
            "goland",
        ],
        name: "JetBrains IDE",
        kind: TargetKind::Editor,
        args: &["{path}"],
        wait: WaitMode::Flag("--wait"),
        cwd: CwdMode::Location,
        recents: RecentsSource::JetBrainsXml {
            rel: "options/recentProjects.xml",
        },
        config_dirs: &["JetBrains"],
        wsl_args: None,
    },
    CatalogueEntry {
        binaries: &["zed"],
        name: "Zed",
        kind: TargetKind::Editor,
        args: &["{path}"],
        wait: WaitMode::Flag("--wait"),
        cwd: CwdMode::Location,
        recents: RecentsSource::None,
        config_dirs: &["zed"],
        wsl_args: None,
    },
    CatalogueEntry {
        binaries: &["wezterm"],
        name: "WezTerm",
        kind: TargetKind::Terminal,
        args: &["start", "--cwd", "{path}"],
        wait: WaitMode::Never,
        cwd: CwdMode::None,
        recents: RecentsSource::None,
        config_dirs: &["wezterm"],
        wsl_args: None,
    },
    CatalogueEntry {
        binaries: &["alacritty"],
        name: "Alacritty",
        kind: TargetKind::Terminal,
        args: &["--working-directory", "{path}"],
        wait: WaitMode::Never,
        cwd: CwdMode::None,
        recents: RecentsSource::None,
        config_dirs: &["alacritty"],
        wsl_args: None,
    },
    CatalogueEntry {
        binaries: &["kitty"],
        name: "kitty",
        kind: TargetKind::Terminal,
        args: &["--directory", "{path}"],
        wait: WaitMode::Never,
        cwd: CwdMode::None,
        recents: RecentsSource::None,
        config_dirs: &["kitty"],
        wsl_args: None,
    },
    CatalogueEntry {
        binaries: &["foot"],
        name: "foot",
        kind: TargetKind::Terminal,
        args: &["-D", "{path}"],
        wait: WaitMode::Never,
        cwd: CwdMode::None,
        recents: RecentsSource::None,
        config_dirs: &["foot"],
        wsl_args: None,
    },
    CatalogueEntry {
        binaries: &["konsole"],
        name: "Konsole",
        kind: TargetKind::Terminal,
        args: &["--workdir", "{path}"],
        wait: WaitMode::Never,
        cwd: CwdMode::None,
        recents: RecentsSource::None,
        config_dirs: &["konsolerc"],
        wsl_args: None,
    },
    CatalogueEntry {
        binaries: &["gnome-terminal"],
        name: "Terminal",
        kind: TargetKind::Terminal,
        args: &["--working-directory", "{path}"],
        wait: WaitMode::Never,
        cwd: CwdMode::None,
        recents: RecentsSource::None,
        config_dirs: &[],
        wsl_args: None,
    },
    CatalogueEntry {
        binaries: &["wt"],
        name: "Windows Terminal",
        kind: TargetKind::Terminal,
        args: &["-d", "{path}"],
        wait: WaitMode::Never,
        cwd: CwdMode::None,
        recents: RecentsSource::None,
        config_dirs: &[],
        wsl_args: Some(&["wsl", "-d", "{distro}", "--cd", "{path}"]),
    },
    CatalogueEntry {
        binaries: &["explorer"],
        name: "Explorer",
        kind: TargetKind::FileManager,
        args: &["{path}"],
        wait: WaitMode::Never,
        cwd: CwdMode::None,
        recents: RecentsSource::None,
        config_dirs: &[],
        wsl_args: None,
    },
    CatalogueEntry {
        binaries: &["nautilus", "dolphin", "thunar", "nemo", "pcmanfm"],
        name: "Files",
        kind: TargetKind::FileManager,
        args: &["{path}"],
        wait: WaitMode::Never,
        cwd: CwdMode::None,
        recents: RecentsSource::None,
        config_dirs: &[],
        wsl_args: None,
    },
    CatalogueEntry {
        binaries: &["xdg-open", "open"],
        name: "System handler",
        kind: TargetKind::FileManager,
        args: &["{path}"],
        wait: WaitMode::Never,
        cwd: CwdMode::None,
        recents: RecentsSource::None,
        config_dirs: &[],
        wsl_args: None,
    },
    CatalogueEntry {
        binaries: &[
            "lazygit",
            "tig",
            "gitg",
            "git-cola",
            "gitkraken",
            "sourcetree",
            "smartgit",
            "fork",
        ],
        name: "Git client",
        kind: TargetKind::GitClient,
        args: &["{path}"],
        wait: WaitMode::Never,
        cwd: CwdMode::Location,
        recents: RecentsSource::None,
        config_dirs: &[],
        wsl_args: None,
    },
];

/// `stem` is a file stem — `code`, never `code.exe` and never a full path.
#[must_use]
pub fn lookup(stem: &str) -> Option<&'static CatalogueEntry> {
    CATALOGUE
        .iter()
        .find(|e| e.binaries.iter().any(|b| b.eq_ignore_ascii_case(stem)))
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

    #[test]
    fn every_terminal_named_in_the_spec_is_present_with_its_directory_flag() {
        for (bin, flag) in [
            ("wezterm", "--cwd"),
            ("alacritty", "--working-directory"),
            ("kitty", "--directory"),
            ("foot", "-D"),
            ("konsole", "--workdir"),
            ("gnome-terminal", "--working-directory"),
            ("wt", "-d"),
        ] {
            let e = lookup(bin).unwrap_or_else(|| panic!("{bin} missing from the catalogue"));
            assert_eq!(e.kind, TargetKind::Terminal, "{bin}");
            assert!(e.args.contains(&flag), "{bin} does not carry {flag}");
        }
    }

    #[test]
    fn a_flag_waiter_and_an_always_waiter_are_distinguished() {
        assert_eq!(lookup("code").unwrap().wait, WaitMode::Flag("--wait"));
        assert!(lookup("code").unwrap().args.contains(&"--wait"));
        assert_eq!(lookup("nvim").unwrap().wait, WaitMode::Always);
        assert_eq!(lookup("wezterm").unwrap().wait, WaitMode::Never);
    }

    #[test]
    fn lookup_is_case_insensitive_and_extension_free() {
        assert!(lookup("Code").is_some());
        assert!(
            lookup("code.exe").is_none(),
            "lookup takes a stem, not a file name"
        );
    }

    #[test]
    fn no_entry_names_a_language() {
        // §4bis.2a: phase 1 ships no built-in language-to-editor table.
        let text = format!("{CATALOGUE:?}");
        for lang in ["Rust", "TypeScript", "Python", "C++", "C#"] {
            assert!(
                !text.contains(lang),
                "the catalogue names the language {lang}"
            );
        }
    }

    #[test]
    fn every_kind_the_dropdown_offers_has_at_least_one_entry() {
        for kind in [
            TargetKind::Editor,
            TargetKind::Terminal,
            TargetKind::FileManager,
            TargetKind::GitClient,
        ] {
            assert!(
                CATALOGUE.iter().any(|e| e.kind == kind),
                "{kind:?} has no entry"
            );
        }
    }

    #[test]
    fn kind_and_cwd_mode_round_trip_through_their_column_strings() {
        for kind in [TargetKind::Editor, TargetKind::FileManager] {
            assert_eq!(TargetKind::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(CwdMode::parse("location"), Some(CwdMode::Location));
        assert_eq!(CwdMode::parse("elsewhere"), None);
    }
}
