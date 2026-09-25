//! Linux launch-target discovery.
//!
//! Desktop entries are never filtered on `Categories=Development`: that finds editors but
//! excludes file managers and terminals. Discovered applications are instead filtered through
//! the catalogue, which is the same list used to draw the target dropdown.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::launch::probe::{ProbeFacts, ProbeSource, ProbedApp, TargetProbe};

#[derive(Debug)]
pub struct LinuxProbe {
    pub xdg_data_dirs: Vec<PathBuf>,
    pub path_dirs: Vec<PathBuf>,
    pub env: BTreeMap<String, String>,
    /// Injected so `flatpak` and `snap` are a seam, not a hard dependency on the host.
    pub run: fn(&str, &[&str]) -> Option<String>,
}

fn run_capture(program: &str, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8(out.stdout).ok()
}

impl LinuxProbe {
    #[must_use]
    pub fn new() -> Self {
        let env: BTreeMap<String, String> = std::env::vars().collect();
        let home = env.get("HOME").cloned().unwrap_or_default();
        let mut data = vec![PathBuf::from("/usr/share/applications")];
        if !home.is_empty() {
            data.push(PathBuf::from(&home).join(".local/share/applications"));
        }
        let path_dirs = env.get("PATH").map_or_else(Vec::new, |p| {
            p.split(':')
                .filter(|s| !s.is_empty())
                .map(PathBuf::from)
                .collect()
        });
        Self {
            xdg_data_dirs: data,
            path_dirs,
            env,
            run: run_capture,
        }
    }

    fn scan_desktop(&self, out: &mut Vec<ProbedApp>) {
        for dir in &self.xdg_data_dirs {
            let Ok(entries) = std::fs::read_dir(dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("desktop") {
                    continue;
                }
                let Ok(bytes) = std::fs::read(&path) else {
                    continue;
                };
                let Some((exec, _)) = parse_desktop_exec(&bytes) else {
                    continue;
                };
                push(out, Path::new(&exec), ProbeSource::Desktop, None);
            }
        }
    }

    fn scan_path(&self, out: &mut Vec<ProbedApp>) {
        for dir in &self.path_dirs {
            let Ok(entries) = std::fs::read_dir(dir) else {
                continue;
            };
            for entry in entries.flatten() {
                push(out, &entry.path(), ProbeSource::Path, None);
            }
        }
    }
}

impl Default for LinuxProbe {
    fn default() -> Self {
        Self::new()
    }
}

fn mtime_secs(path: &Path) -> Option<i64> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    let dur = modified.duration_since(std::time::UNIX_EPOCH).ok()?;
    i64::try_from(dur.as_secs()).ok()
}

fn push(out: &mut Vec<ProbedApp>, exec: &Path, source: ProbeSource, distro: Option<String>) {
    let Some(stem) = exec.file_stem().map(|s| s.to_string_lossy().into_owned()) else {
        return;
    };
    if crate::launch::catalogue::lookup(&stem).is_none() {
        return;
    }
    out.push(ProbedApp {
        exec: exec.to_path_buf(),
        stem,
        source,
        distro,
        installed_at: mtime_secs(exec),
    });
}

impl TargetProbe for LinuxProbe {
    fn probe(&self) -> ProbeFacts {
        let mut apps = Vec::new();
        self.scan_desktop(&mut apps);
        self.scan_path(&mut apps);
        for id in (self.run)("flatpak", &["list", "--columns=application"])
            .as_deref()
            .map_or_else(Vec::new, parse_flatpak_list)
        {
            // A flatpak is launched through `flatpak run <id>`; the id's tail is its stem.
            let stem = id.rsplit('.').next().unwrap_or(&id).to_ascii_lowercase();
            if crate::launch::catalogue::lookup(&stem).is_some() {
                apps.push(ProbedApp {
                    exec: PathBuf::from("/usr/bin/flatpak"),
                    stem,
                    source: ProbeSource::Flatpak,
                    distro: None,
                    installed_at: None,
                });
            }
        }
        for name in (self.run)("snap", &["list"])
            .as_deref()
            .map_or_else(Vec::new, parse_snap_list)
        {
            push(
                &mut apps,
                &PathBuf::from("/snap/bin").join(&name),
                ProbeSource::Snap,
                None,
            );
        }
        let editor_env = self
            .env
            .get("VISUAL")
            .or_else(|| self.env.get("EDITOR"))
            .and_then(|v| v.split_whitespace().next())
            .and_then(|v| {
                Path::new(v)
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
            });
        let folder_handler_stem = (self.run)("xdg-mime", &["query", "default", "inode/directory"])
            .and_then(|s| {
                let id = s.trim().trim_end_matches(".desktop").to_owned();
                if id.is_empty() {
                    None
                } else {
                    Some(id.to_ascii_lowercase())
                }
            });
        ProbeFacts {
            apps: crate::launch::probe::dedupe(apps),
            editor_env,
            folder_handler_stem,
        }
    }
}

/// The `Exec=` key of the `[Desktop Entry]` group, with `%f %F %u %U %i %c %k` removed.
#[must_use]
pub fn parse_desktop_exec(bytes: &[u8]) -> Option<(String, Vec<String>)> {
    let text = String::from_utf8_lossy(bytes);
    let mut in_entry = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_entry {
            continue;
        }
        let Some(value) = line.strip_prefix("Exec=") else {
            continue;
        };
        let mut parts = split_exec(value);
        if parts.is_empty() {
            return None;
        }
        let exec = parts.remove(0);
        parts.retain(|p| !(p.len() == 2 && p.starts_with('%')));
        return Some((exec, parts));
    }
    None
}

fn split_exec(value: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    for ch in value.chars() {
        match ch {
            '"' => quoted = !quoted,
            c if c.is_whitespace() && !quoted => {
                if !current.is_empty() {
                    out.push(std::mem::take(&mut current));
                }
            }
            c => current.push(c),
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

#[must_use]
pub fn parse_flatpak_list(text: &str) -> Vec<String> {
    text.lines()
        .skip_while(|l| l.contains("Application ID"))
        .filter_map(|l| l.split('\t').find(|f| f.contains('.')).map(str::to_owned))
        .collect()
}

#[must_use]
pub fn parse_snap_list(text: &str) -> Vec<String> {
    text.lines()
        .skip(1)
        .filter_map(|l| l.split_whitespace().next().map(str::to_owned))
        .collect()
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
    fn a_desktop_exec_line_loses_its_field_codes_and_keeps_its_arguments() {
        let d =
            b"[Desktop Entry]\nName=Editor\nExec=/usr/bin/code --new-window %F\nType=Application\n";
        let (exec, args) = parse_desktop_exec(d).unwrap();
        assert_eq!(exec, "/usr/bin/code");
        assert_eq!(args, vec!["--new-window".to_owned()]);
    }

    #[test]
    fn a_quoted_exec_with_a_space_survives() {
        let d = b"[Desktop Entry]\nExec=\"/opt/My App/bin/kitty\" --single-instance %U\n";
        let (exec, args) = parse_desktop_exec(d).unwrap();
        assert_eq!(exec, "/opt/My App/bin/kitty");
        assert_eq!(args, vec!["--single-instance".to_owned()]);
    }

    #[test]
    fn an_exec_line_in_an_action_group_is_not_the_entrys_exec() {
        let d = b"[Desktop Entry]\nExec=/usr/bin/foot\n[Desktop Action new]\nExec=/usr/bin/other\n";
        assert_eq!(parse_desktop_exec(d).unwrap().0, "/usr/bin/foot");
    }

    #[test]
    fn a_desktop_file_with_no_exec_yields_nothing_rather_than_a_default() {
        assert_eq!(parse_desktop_exec(b"[Desktop Entry]\nName=Nothing\n"), None);
    }

    #[test]
    fn flatpak_and_snap_listings_are_reduced_to_their_command_names() {
        let flat = "Name\tApplication ID\tVersion\nEditor\torg.example.Code\t1.0\n";
        assert_eq!(
            parse_flatpak_list(flat),
            vec!["org.example.Code".to_owned()]
        );
        let snap = "Name       Version  Rev  Tracking  Publisher  Notes\ncode  1.0  1  latest  x  classic\n";
        assert_eq!(parse_snap_list(snap), vec!["code".to_owned()]);
    }

    #[test]
    fn the_probe_reports_path_hits_and_the_editor_environment() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("kitty");
        std::fs::write(&bin, b"#!/bin/sh\n").unwrap();
        let mut env = BTreeMap::new();
        env.insert("VISUAL".to_owned(), "/usr/bin/nvim".to_owned());
        let probe = LinuxProbe {
            xdg_data_dirs: vec![],
            path_dirs: vec![dir.path().to_path_buf()],
            env,
            run: |_, _| None,
        };
        let facts = probe.probe();
        assert!(facts.apps.iter().any(|a| a.stem == "kitty"));
        assert_eq!(
            facts.editor_env.as_deref(),
            Some("nvim"),
            "reduced to a stem"
        );
    }
}
