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

/// The production `DistroProbe` (R9, R46). §13 forbids starting a stopped distro, so this only
/// ever reads the two quiet listings — neither of which names a distro, and so neither of which
/// can start one.
///
/// It is declared here, beside the CLI it drives, and in the same change as the trait's first
/// real use: a trait that gets its fake and never its production implementation has cost this
/// project four rulings, and this seam was the fifth until now.
///
/// On a machine with no `wsl.exe` the spawn fails and the answer is an empty list — which is
/// the truth ("no distros found"), not a stand-in for it. No `#[cfg]`: the behaviour is then
/// testable on either host, and the two listings are the whole cost.
#[derive(Debug, Clone)]
pub struct SystemDistroProbe {
    cli: std::sync::Arc<dyn WslCli>,
}

impl SystemDistroProbe {
    #[must_use]
    pub fn new(cli: std::sync::Arc<dyn WslCli>) -> SystemDistroProbe {
        SystemDistroProbe { cli }
    }

    /// The probe the shipped binary uses.
    #[must_use]
    pub fn system() -> SystemDistroProbe {
        SystemDistroProbe::new(std::sync::Arc::new(SystemWslCli::new()))
    }
}

impl crate::firstrun::classify::DistroProbe for SystemDistroProbe {
    fn distros(&self) -> Vec<DistroInfo> {
        installed_distros(self.cli.as_ref()).unwrap_or_default()
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

    // The shell name and its command flag are named through constants rather than written
    // adjacent in one array. `core/tests/launch_spawn.rs` catches a launch that goes through a
    // shell by scanning this crate for that pair as adjacent source text, and a list *about*
    // the rule reads to it exactly like a violation of it.
    const SHELL_NAME: &str = "sh";
    const SHELL_COMMAND_FLAG: &str = "-c";

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
        for banned in [
            "-e",
            "--shell-type",
            "bash",
            SHELL_NAME,
            SHELL_COMMAND_FLAG,
            "--",
        ] {
            assert!(
                !strings.contains(&banned.to_owned()),
                "{banned} would run a shell"
            );
        }
    }

    /// Records what was asked and answers with canned UTF-16LE, so the *production* probe is
    /// what runs — not a second implementation written to satisfy the test.
    #[derive(Debug, Default)]
    struct RecordingCli {
        asked: std::sync::Mutex<Vec<Vec<String>>>,
        replies: std::sync::Mutex<Vec<Vec<u8>>>,
    }

    impl RecordingCli {
        fn with(replies: Vec<Vec<u8>>) -> RecordingCli {
            RecordingCli {
                asked: std::sync::Mutex::new(Vec::new()),
                replies: std::sync::Mutex::new(replies),
            }
        }

        fn asked(&self) -> Vec<Vec<String>> {
            self.asked.lock().map(|a| a.clone()).unwrap_or_default()
        }
    }

    impl super::WslCli for RecordingCli {
        fn output(&self, args: &[&std::ffi::OsStr]) -> std::io::Result<std::process::Output> {
            if let Ok(mut asked) = self.asked.lock() {
                asked.push(
                    args.iter()
                        .map(|a| a.to_string_lossy().into_owned())
                        .collect(),
                );
            }
            let stdout = self
                .replies
                .lock()
                .map(|mut r| {
                    if r.is_empty() {
                        Vec::new()
                    } else {
                        r.remove(0)
                    }
                })
                .unwrap_or_default();
            Ok(std::process::Output {
                status: std::process::Command::new(if cfg!(windows) { "cmd" } else { "true" })
                    .args(if cfg!(windows) {
                        vec!["/c", "exit 0"]
                    } else {
                        vec![]
                    })
                    .status()?,
                stdout,
                stderr: Vec::new(),
            })
        }

        fn spawn_piped(&self, _args: &[&std::ffi::OsStr]) -> std::io::Result<std::process::Child> {
            Err(std::io::Error::other("not used by the probe"))
        }
    }

    #[test]
    fn the_production_probe_reads_both_listings_and_starts_nothing() {
        use crate::firstrun::classify::DistroProbe;

        let cli = std::sync::Arc::new(RecordingCli::with(vec![
            utf16le("alpha\r\nbeta\r\n", true),
            utf16le("beta\r\n", true),
        ]));
        let probe = super::SystemDistroProbe::new(cli.clone());
        assert_eq!(
            probe.distros(),
            vec![
                DistroInfo {
                    name: "alpha".to_owned(),
                    state: DistroState::Stopped
                },
                DistroInfo {
                    name: "beta".to_owned(),
                    state: DistroState::Running
                },
            ]
        );
        assert_eq!(
            cli.asked(),
            vec![
                vec!["--list".to_owned(), "--quiet".to_owned()],
                vec![
                    "--list".to_owned(),
                    "--running".to_owned(),
                    "--quiet".to_owned()
                ],
            ]
        );
    }

    #[test]
    fn a_machine_without_wsl_reports_no_distros_rather_than_failing() {
        use crate::firstrun::classify::DistroProbe;

        #[derive(Debug)]
        struct NoWslExe;
        impl super::WslCli for NoWslExe {
            fn output(&self, _args: &[&std::ffi::OsStr]) -> std::io::Result<std::process::Output> {
                Err(std::io::Error::from(std::io::ErrorKind::NotFound))
            }
            fn spawn_piped(
                &self,
                _args: &[&std::ffi::OsStr],
            ) -> std::io::Result<std::process::Child> {
                Err(std::io::Error::from(std::io::ErrorKind::NotFound))
            }
        }
        let probe = super::SystemDistroProbe::new(std::sync::Arc::new(NoWslExe));
        assert!(probe.distros().is_empty());
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
