//! §13 — which distros exist, which are already running, and how to start the worker in one.
//!
//! `wsl.exe --list --verbose` is not used: its header and its state column are localised, so a
//! parser keyed on `Running` reports every distro stopped on a non-English Windows, and the app
//! then asks for consent it does not need. `--list --quiet` and `--list --running --quiet` emit
//! names only, in every locale, and neither starts anything.

use std::ffi::{OsStr, OsString};

pub const WSL_EXE: &str = "wsl.exe";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DistroState {
    Running,
    Stopped,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DistroInfo {
    pub name: String,
    pub state: DistroState,
}

/// `wsl.exe` writes UTF-16LE with a byte-order mark. A truncated final unit is dropped rather
/// than failing the whole listing.
#[must_use]
pub fn decode_utf16le(bytes: &[u8]) -> String {
    let body = if bytes.starts_with(&[0xFF, 0xFE]) {
        bytes.get(2..).unwrap_or_default()
    } else {
        bytes
    };
    let units: Vec<u16> = body
        .chunks_exact(2)
        .filter_map(|pair| Some(u16::from_le_bytes([*pair.first()?, *pair.get(1)?])))
        .collect();
    char::decode_utf16(units)
        .map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER))
        .collect()
}

#[must_use]
pub fn parse_list_quiet(bytes: &[u8]) -> Vec<String> {
    decode_utf16le(bytes)
        .lines()
        .map(|line| {
            line.trim_matches(|c: char| c.is_whitespace() || c == '\u{0}')
                .to_owned()
        })
        .filter(|line| !line.is_empty())
        .collect()
}

/// Installed minus running is stopped. Order follows the installed list.
#[must_use]
pub fn merge_states(installed: &[String], running: &[String]) -> Vec<DistroInfo> {
    installed
        .iter()
        .map(|name| DistroInfo {
            name: name.clone(),
            state: if running.iter().any(|r| r == name) {
                DistroState::Running
            } else {
                DistroState::Stopped
            },
        })
        .collect()
}

/// Neither listing names a distro, so neither can start one (§13).
#[must_use]
pub fn list_argv(running_only: bool) -> Vec<&'static str> {
    if running_only {
        vec!["--list", "--running", "--quiet"]
    } else {
        vec!["--list", "--quiet"]
    }
}

/// §13's launch: `-d <distro> [-u <user>] --exec <path>`. `--exec` runs the executable directly,
/// so no login shell runs and no profile can write into the frame stream.
#[must_use]
pub fn launch_argv(distro: &str, user: Option<&str>, exec: &str, args: &[&str]) -> Vec<OsString> {
    let mut command_line = vec![OsString::from("-d"), OsString::from(distro)];
    if let Some(user) = user {
        command_line.push(OsString::from("-u"));
        command_line.push(OsString::from(user));
    }
    command_line.push(OsString::from("--exec"));
    command_line.push(OsString::from(exec));
    command_line.extend(args.iter().map(OsString::from));
    command_line
}

/// The seam. Tests substitute a recorder; nothing else reaches `wsl.exe` directly.
pub trait WslCli: Send + Sync + std::fmt::Debug {
    fn output(&self, args: &[&OsStr]) -> std::io::Result<std::process::Output>;
    fn spawn_piped(&self, args: &[&OsStr]) -> std::io::Result<std::process::Child>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SystemWslCli;

impl SystemWslCli {
    #[must_use]
    pub fn new() -> SystemWslCli {
        SystemWslCli
    }

    fn command(args: &[&OsStr]) -> std::process::Command {
        let mut cmd = std::process::Command::new(WSL_EXE);
        cmd.args(args);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            // CREATE_NO_WINDOW: a console flashing on every scan is a visible defect.
            cmd.creation_flags(0x0800_0000);
        }
        cmd
    }
}

impl WslCli for SystemWslCli {
    fn output(&self, args: &[&OsStr]) -> std::io::Result<std::process::Output> {
        SystemWslCli::command(args)
            .stdin(std::process::Stdio::null())
            .output()
    }

    fn spawn_piped(&self, args: &[&OsStr]) -> std::io::Result<std::process::Child> {
        SystemWslCli::command(args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
    }
}

/// Every installed distro with its state, read without starting any of them.
pub fn installed_distros(cli: &dyn WslCli) -> std::io::Result<Vec<DistroInfo>> {
    let all = list_argv(false);
    let all: Vec<&OsStr> = all.iter().map(OsStr::new).collect();
    let running = list_argv(true);
    let running: Vec<&OsStr> = running.iter().map(OsStr::new).collect();
    let installed = parse_list_quiet(&cli.output(&all)?.stdout);
    let running = parse_list_quiet(&cli.output(&running)?.stdout);
    Ok(merge_states(&installed, &running))
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
mod tests {
    use super::{
        decode_utf16le, launch_argv, list_argv, merge_states, parse_list_quiet, DistroInfo,
        DistroState,
    };
    use std::ffi::OsString;

    fn utf16le(text: &str, bom: bool) -> Vec<u8> {
        let mut out = Vec::new();
        if bom {
            out.extend_from_slice(&[0xFF, 0xFE]);
        }
        for unit in text.encode_utf16() {
            out.extend_from_slice(&unit.to_le_bytes());
        }
        out
    }

    #[test]
    fn the_output_is_utf16le_and_the_bom_is_not_content() {
        assert_eq!(decode_utf16le(&utf16le("alpha", true)), "alpha");
        assert_eq!(decode_utf16le(&utf16le("alpha", false)), "alpha");
        // An odd trailing byte is a truncated stream, not a reason to lose the whole list.
        let mut truncated = utf16le("ab", true);
        truncated.push(0x00);
        assert_eq!(decode_utf16le(&truncated), "ab");
    }

    #[test]
    fn the_quiet_list_is_names_and_nothing_else() {
        let bytes = utf16le("alpha\r\nbeta\r\n\r\n", true);
        assert_eq!(
            parse_list_quiet(&bytes),
            vec!["alpha".to_owned(), "beta".to_owned()]
        );
    }

    #[test]
    fn state_comes_from_two_unlocalised_lists_not_from_a_localised_column() {
        let installed = ["alpha".to_owned(), "beta".to_owned(), "gamma".to_owned()];
        let running = ["beta".to_owned()];
        assert_eq!(
            merge_states(&installed, &running),
            vec![
                DistroInfo {
                    name: "alpha".to_owned(),
                    state: DistroState::Stopped
                },
                DistroInfo {
                    name: "beta".to_owned(),
                    state: DistroState::Running
                },
                DistroInfo {
                    name: "gamma".to_owned(),
                    state: DistroState::Stopped
                },
            ]
        );
    }

    #[test]
    fn neither_list_names_a_distro_and_so_neither_starts_one() {
        assert_eq!(list_argv(false), vec!["--list", "--quiet"]);
        assert_eq!(list_argv(true), vec!["--list", "--running", "--quiet"]);
        for argv in [list_argv(false), list_argv(true)] {
            assert!(!argv.contains(&"-d"), "a listing must never name a distro");
        }
    }

    #[test]
    fn the_launch_is_non_interactive_and_runs_no_shell() {
        // §13: `--exec` so shell profiles cannot write into the frame stream.
        let argv = launch_argv(
            "alpha",
            Some("u"),
            "/home/u/.cache/codotheca/worker/ab/w",
            &[],
        );
        assert_eq!(
            argv,
            vec![
                OsString::from("-d"),
                OsString::from("alpha"),
                OsString::from("-u"),
                OsString::from("u"),
                OsString::from("--exec"),
                OsString::from("/home/u/.cache/codotheca/worker/ab/w"),
            ]
        );
        let strings: Vec<String> = argv
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        for banned in ["-e", "--shell-type", "bash", "sh", "-c", "--"] {
            assert!(
                !strings.contains(&banned.to_owned()),
                "{banned} would run a shell"
            );
        }
    }

    #[test]
    fn the_user_flag_is_omitted_rather_than_guessed() {
        let argv = launch_argv("alpha", None, "/w", &["--version"]);
        let strings: Vec<String> = argv
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert!(!strings.contains(&"-u".to_owned()));
        assert_eq!(strings, vec!["-d", "alpha", "--exec", "/w", "--version"]);
    }
}
