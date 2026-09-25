//! §4bis.1's Windows sources. The two parsers are platform-free and are tested on every
//! platform; only the registry and Start Menu walk is `#[cfg(windows)]`.

use std::path::{Path, PathBuf};

use crate::launch::probe::{ProbeFacts, TargetProbe};

const HEADER_SIZE: usize = 0x4c;
const FLAG_HAS_LINK_INFO: u32 = 0x0000_0002;
const LINK_INFO_HAS_LOCAL_BASE_PATH: u32 = 0x0000_0001;

fn le_u32(bytes: &[u8], at: usize) -> Option<u32> {
    let slice = bytes.get(at..at.checked_add(4)?)?;
    let array: [u8; 4] = slice.try_into().ok()?;
    Some(u32::from_le_bytes(array))
}

/// The `LocalBasePath` of a Shell Link's `LinkInfo` block. `None` for anything else — a
/// shortcut we cannot read exactly is not a target, and the `LinkTargetIDList` is not parsed.
#[must_use]
pub fn parse_shell_link_target(bytes: &[u8]) -> Option<PathBuf> {
    if bytes.len() < HEADER_SIZE || le_u32(bytes, 0)? != u32::try_from(HEADER_SIZE).ok()? {
        return None;
    }
    if le_u32(bytes, 20)? & FLAG_HAS_LINK_INFO == 0 {
        return None;
    }
    let info = bytes.get(HEADER_SIZE..)?;
    if le_u32(info, 8)? & LINK_INFO_HAS_LOCAL_BASE_PATH == 0 {
        return None;
    }
    let offset = usize::try_from(le_u32(info, 16)?).ok()?;
    let tail = info.get(offset..)?;
    let end = tail.iter().position(|b| *b == 0).unwrap_or(tail.len());
    let text = std::str::from_utf8(tail.get(..end)?).ok()?;
    if text.is_empty() {
        None
    } else {
        Some(PathBuf::from(text))
    }
}

/// `"C:\x\y.exe" "%1"` and `C:\x\y.exe %1` both reduce to the executable.
#[must_use]
pub fn parse_command_line_exec(command: &str) -> Option<String> {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return None;
    }
    let exec = trimmed.strip_prefix('"').map_or_else(
        || {
            trimmed
                .split_whitespace()
                .next()
                .unwrap_or_default()
                .to_owned()
        },
        |rest| rest.split('"').next().unwrap_or_default().to_owned(),
    );
    if exec.is_empty() {
        None
    } else {
        Some(exec)
    }
}

/// Two common Windows package managers both place a real `.exe` beside their metadata, so the
/// executable is taken directly and the metadata file is ignored.
#[must_use]
pub fn shim_targets(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("exe"))
        })
        .collect()
}

/// The Windows [`TargetProbe`]: the registry and Start Menu walk on Windows. Built for any other
/// host it finds nothing.
#[derive(Debug, Default)]
pub struct WindowsProbe;

impl WindowsProbe {
    /// A probe of this host; it carries no state of its own.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl TargetProbe for WindowsProbe {
    #[cfg(not(windows))]
    fn probe(&self) -> ProbeFacts {
        ProbeFacts::default()
    }

    // 138 lines of registry walking: three hives, each with its own key layout and its own
    // failure mode. Splitting it would move the hive-specific handling away from the hive it
    // belongs to for no gain in testability, since none of it is reachable off Windows.
    // Same allow, same reason, as `core/src/main.rs:105`.
    #[cfg(windows)]
    #[allow(clippy::too_many_lines)]
    fn probe(&self) -> ProbeFacts {
        use crate::launch::probe::{ProbeSource, ProbedApp};
        use winreg::enums::{HKEY_CLASSES_ROOT, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
        use winreg::RegKey;

        let mut apps: Vec<ProbedApp> = Vec::new();
        let mut push = |exec: PathBuf, source: ProbeSource| {
            let Some(stem) = exec.file_stem().map(|s| s.to_string_lossy().into_owned()) else {
                return;
            };
            if crate::launch::catalogue::lookup(&stem).is_none() {
                return;
            }
            let installed_at = std::fs::metadata(&exec)
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .and_then(|d| i64::try_from(d.as_secs()).ok());
            apps.push(ProbedApp {
                exec,
                stem,
                source,
                distro: None,
                installed_at,
            });
        };

        // App Paths, both hives.
        for hive in [HKEY_LOCAL_MACHINE, HKEY_CURRENT_USER] {
            let Ok(root) = RegKey::predef(hive)
                .open_subkey(r"SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths")
            else {
                continue;
            };
            for name in root.enum_keys().flatten() {
                if let Ok(key) = root.open_subkey(&name) {
                    if let Ok(value) = key.get_value::<String, _>("") {
                        push(
                            PathBuf::from(value.trim_matches('"')),
                            ProbeSource::Registry,
                        );
                    }
                }
            }
        }

        // $PATH, the Start Menu, the Toolbox scripts directory and the two shim directories.
        let env: std::collections::BTreeMap<String, String> = std::env::vars().collect();
        for dir in env
            .get("PATH")
            .map(|p| p.split(';').map(PathBuf::from).collect::<Vec<_>>())
            .unwrap_or_default()
        {
            for exec in shim_targets(&dir) {
                push(exec, ProbeSource::Path);
            }
        }
        let local = env.get("LOCALAPPDATA").cloned().unwrap_or_default();
        let appdata = env.get("APPDATA").cloned().unwrap_or_default();
        for (dir, source) in [
            (
                PathBuf::from(&local).join(r"JetBrains\Toolbox\scripts"),
                ProbeSource::Toolbox,
            ),
            (
                PathBuf::from(&env.get("USERPROFILE").cloned().unwrap_or_default())
                    .join(r"scoop\shims"),
                ProbeSource::Shim,
            ),
            (
                PathBuf::from(r"C:\ProgramData\chocolatey\bin"),
                ProbeSource::Shim,
            ),
        ] {
            for exec in shim_targets(&dir) {
                push(exec, source);
            }
        }
        for root in [PathBuf::from(&appdata).join(r"Microsoft\Windows\Start Menu\Programs")] {
            let mut stack = vec![root];
            while let Some(dir) = stack.pop() {
                let Ok(entries) = std::fs::read_dir(&dir) else {
                    continue;
                };
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        stack.push(path);
                        continue;
                    }
                    if path.extension().and_then(|e| e.to_str()) != Some("lnk") {
                        continue;
                    }
                    if let Some(target) = std::fs::read(&path)
                        .ok()
                        .as_deref()
                        .and_then(parse_shell_link_target)
                    {
                        push(target, ProbeSource::StartMenu);
                    }
                }
            }
        }

        // The OS default handler for a folder.
        let folder_handler_stem = RegKey::predef(HKEY_CLASSES_ROOT)
            .open_subkey(r"Directory\shell")
            .ok()
            .and_then(|shell| shell.get_value::<String, _>("").ok())
            .and_then(|verb| {
                RegKey::predef(HKEY_CLASSES_ROOT)
                    .open_subkey(format!(r"Directory\shell\{verb}\command"))
                    .ok()?
                    .get_value::<String, _>("")
                    .ok()
            })
            .and_then(|command| parse_command_line_exec(&command))
            .and_then(|exec| {
                Path::new(&exec)
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
            });

        let editor_env = env
            .get("VISUAL")
            .or_else(|| env.get("EDITOR"))
            .and_then(|v| {
                Path::new(v)
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
            });

        ProbeFacts {
            apps: crate::launch::probe::dedupe(apps),
            editor_env,
            folder_handler_stem,
        }
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

    /// Builds the smallest Shell Link with a `LinkInfo` block naming `target`.
    fn shell_link(target: &str) -> Vec<u8> {
        let mut header = vec![0u8; 0x4c];
        header[0] = 0x4c; // HeaderSize
        header[4..20].copy_from_slice(&[
            0x01, 0x14, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0xC0, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x46,
        ]); // LinkCLSID
        header[20..24].copy_from_slice(&0x0000_0002u32.to_le_bytes()); // HasLinkInfo only
        let base = target.as_bytes();
        let base_len = u32::try_from(base.len()).unwrap();
        let header_len = 28u32;
        let mut info = Vec::new();
        info.extend_from_slice(&(header_len + base_len + 1).to_le_bytes()); // LinkInfoSize
        info.extend_from_slice(&header_len.to_le_bytes()); // LinkInfoHeaderSize
        info.extend_from_slice(&0x0000_0001u32.to_le_bytes()); // VolumeIDAndLocalBasePath
        info.extend_from_slice(&0u32.to_le_bytes()); // VolumeIDOffset
        info.extend_from_slice(&header_len.to_le_bytes()); // LocalBasePathOffset
        info.extend_from_slice(&0u32.to_le_bytes()); // CommonNetworkRelativeLinkOffset
        info.extend_from_slice(&0u32.to_le_bytes()); // CommonPathSuffixOffset
        info.extend_from_slice(base);
        info.push(0);
        header.extend_from_slice(&info);
        header
    }

    #[test]
    fn a_shortcut_yields_the_executable_it_points_at() {
        let bytes = shell_link(r"C:\Apps\code.exe");
        assert_eq!(
            parse_shell_link_target(&bytes),
            Some(PathBuf::from(r"C:\Apps\code.exe"))
        );
    }

    #[test]
    fn a_shortcut_without_a_link_info_block_is_skipped_not_guessed() {
        let mut bytes = shell_link(r"C:\Apps\code.exe");
        bytes[20..24].copy_from_slice(&0u32.to_le_bytes()); // clear HasLinkInfo
        assert_eq!(parse_shell_link_target(&bytes), None);
    }

    #[test]
    fn a_truncated_shortcut_returns_none_rather_than_panicking() {
        assert_eq!(parse_shell_link_target(&[0x4c, 0, 0]), None);
        assert_eq!(parse_shell_link_target(&[]), None);
    }

    #[test]
    fn a_shell_verb_command_yields_its_executable_without_its_placeholders() {
        assert_eq!(
            parse_command_line_exec(r#""C:\Apps\code.exe" "%1""#).as_deref(),
            Some(r"C:\Apps\code.exe")
        );
        assert_eq!(
            parse_command_line_exec(r"C:\Windows\explorer.exe %1").as_deref(),
            Some(r"C:\Windows\explorer.exe")
        );
        assert_eq!(parse_command_line_exec("   "), None);
    }

    #[test]
    fn shim_directories_contribute_their_executables_only() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("code.exe"), b"x").unwrap();
        std::fs::write(dir.path().join("code.shim"), b"path = x").unwrap();
        let found = shim_targets(dir.path());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].file_name().unwrap(), "code.exe");
    }
}
