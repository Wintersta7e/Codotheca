//! Assembling the spawn argv from a stored row and the copy being opened.
//!
//! Three rules:
//!
//! - **Nothing is ever joined into a string and re-split.** Substitution is per argument and the
//!   result is an `OsString`, so a path with a space or a quote in it cannot become two
//!   arguments.
//! - **§4bis.4's WSL forms** come from the catalogue's `wsl_args` when the site is a distro: an
//!   editor becomes `--remote wsl+<distro> <linux-path>`, a terminal becomes
//!   `wsl -d <distro> --cd <linux-path>`. A target with no `wsl_args` and a distro site yields
//!   `None` — we do not invent an invocation.
//! - **The `\\wsl.localhost\…` form is display only.** `build` never emits it; it emits the
//!   Linux-shaped path the distro understands, and a test asserts the prefix appears in no
//!   argument.
//!
//! **Two substitution rules, on purpose.** A *stored row* argument is replaced only when the
//! whole argument is the placeholder, because those arguments are the user's and `--x={path}`
//! must survive byte-for-byte. A *catalogue* `wsl_args` pattern interpolates, because those
//! patterns are ours and `wsl+{distro}` is one argument by construction.

use std::ffi::OsString;
use std::path::PathBuf;

// R21: `LocationKind` is plan 09's, in `core/src/derive`. Do not redeclare it here.
use crate::derive::LocationKind;
use crate::launch::catalogue::{lookup, CwdMode, WaitMode};
use crate::launch::resolve::StoredTarget;
use crate::launch::wslpath::{windows_to_wsl, DEFAULT_DRVFS_ROOT};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchSite {
    pub kind: LocationKind,
    pub distro: String,
    pub path_bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invocation {
    pub program: PathBuf,
    pub argv: Vec<OsString>,
    pub cwd: Option<PathBuf>,
    pub env: Vec<(String, String)>,
    pub wait: bool,
}

/// The executable's stem, separator-agnostic.
///
/// `Path::file_stem` knows only the host's separator, and a stored `exec_bytes` may use the
/// other one — a Windows executable path read on a host that treats `\` as an ordinary
/// character yields the whole path as one "file name", and the catalogue lookup then misses.
fn stem_of(target: &StoredTarget) -> String {
    let display = crate::paths::path_display(&crate::paths::path_from_bytes(&target.exec_bytes));
    let file = display.rsplit(['/', '\\']).next().unwrap_or(&display);
    file.rsplit_once('.')
        .map_or(file, |(stem, _)| stem)
        .to_owned()
}

/// §9, mechanism 1. Waiting is a property of the tool and of the argv it was given, so editing
/// the arguments changes the wait behaviour with them — which is the honest outcome.
#[must_use]
pub fn wants_wait(target: &StoredTarget) -> bool {
    match lookup(&stem_of(target)).map(|e| e.wait) {
        Some(WaitMode::Always) => true,
        Some(WaitMode::Flag(flag)) => target.args.iter().any(|a| a == flag),
        Some(WaitMode::Never) | None => false,
    }
}

/// A stored row's argument: replaced only when the whole argument is the placeholder.
fn substitute(pattern: &str, path: &str, distro: &str) -> OsString {
    match pattern {
        "{path}" => OsString::from(path),
        "{distro}" => OsString::from(distro),
        other => OsString::from(other),
    }
}

/// A catalogue `wsl_args` pattern: interpolated, because `wsl+{distro}` is one argument.
fn interpolate(pattern: &str, path: &str, distro: &str) -> OsString {
    OsString::from(pattern.replace("{distro}", distro).replace("{path}", path))
}

#[must_use]
pub fn build(target: &StoredTarget, site: &LaunchSite) -> Option<Invocation> {
    let program = crate::paths::path_from_bytes(&target.exec_bytes);
    let stored_path = crate::paths::path_from_bytes(&site.path_bytes);
    let display = crate::paths::path_display(&stored_path);

    if site.kind == LocationKind::Wsl {
        // §4bis.4: the stored path is the display form; the distro understands only the
        // Linux one, so translate before anything else and never emit the UNC form.
        let linux = match windows_to_wsl(&display, &site.distro, DEFAULT_DRVFS_ROOT) {
            Some(linux) => linux,
            None if display.starts_with('/') => display.clone(),
            None => return None,
        };
        let pattern = lookup(&stem_of(target)).and_then(|e| e.wsl_args)?;
        let argv = pattern
            .iter()
            .map(|p| interpolate(p, &linux, &site.distro))
            .collect();
        return Some(Invocation {
            program,
            argv,
            cwd: None,
            env: target.env.clone(),
            wait: wants_wait(target),
        });
    }

    let argv = target
        .args
        .iter()
        .map(|a| substitute(a, &display, &site.distro))
        .collect();
    let cwd = match target.cwd_mode {
        CwdMode::Location => Some(stored_path),
        CwdMode::None => None,
    };
    Some(Invocation {
        program,
        argv,
        cwd,
        env: target.env.clone(),
        wait: wants_wait(target),
    })
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
    use crate::launch::catalogue::{CwdMode, TargetKind};
    use crate::launch::resolve::StoredTarget;

    fn target(exec: &str, args: &[&str], kind: TargetKind, cwd: CwdMode) -> StoredTarget {
        StoredTarget {
            id: 1,
            kind,
            name: "t".to_owned(),
            project_id: None,
            location_id: None,
            language: None,
            sort_index: 0,
            detected: true,
            verify_state: "ok".to_owned(),
            verified_at: None,
            exec_bytes: crate::paths::path_bytes(std::path::Path::new(exec)),
            args: args.iter().map(|s| (*s).to_owned()).collect(),
            cwd_mode: cwd,
            env: vec![],
        }
    }

    fn site(kind: LocationKind, distro: &str, path: &str) -> LaunchSite {
        LaunchSite {
            kind,
            distro: distro.to_owned(),
            path_bytes: crate::paths::path_bytes(std::path::Path::new(path)),
        }
    }

    #[test]
    fn a_native_launch_substitutes_the_path_and_sets_the_working_directory() {
        let t = target(
            "/usr/bin/code",
            &["--wait", "{path}"],
            TargetKind::Editor,
            CwdMode::Location,
        );
        let inv = build(&t, &site(LocationKind::Linux, "", "/code/one")).unwrap();
        assert_eq!(inv.program, std::path::PathBuf::from("/usr/bin/code"));
        assert_eq!(
            inv.argv,
            vec![
                std::ffi::OsString::from("--wait"),
                std::ffi::OsString::from("/code/one")
            ]
        );
        assert_eq!(inv.cwd.as_deref(), Some(std::path::Path::new("/code/one")));
        assert!(inv.wait);
    }

    #[test]
    fn cwd_mode_none_leaves_the_working_directory_alone() {
        let t = target(
            "/usr/bin/kitty",
            &["--directory", "{path}"],
            TargetKind::Terminal,
            CwdMode::None,
        );
        let inv = build(&t, &site(LocationKind::Linux, "", "/code/one")).unwrap();
        assert_eq!(inv.cwd, None);
        assert!(!inv.wait);
    }

    #[test]
    fn an_editor_at_a_distro_site_takes_the_remote_form_with_a_linux_path() {
        let t = target(
            r"C:\Apps\code.exe",
            &["--wait", "{path}"],
            TargetKind::Editor,
            CwdMode::Location,
        );
        let inv = build(
            &t,
            &site(
                LocationKind::Wsl,
                "distro-a",
                r"\\wsl.localhost\distro-a\home\u\w",
            ),
        )
        .unwrap();
        let rendered: Vec<String> = inv
            .argv
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(rendered, vec!["--remote", "wsl+distro-a", "/home/u/w"]);
        assert!(
            rendered.iter().all(|a| !a.contains("wsl.localhost")),
            "the UNC form is display only"
        );
        assert_eq!(
            inv.cwd, None,
            "a distro path is not a working directory for a Windows process"
        );
    }

    #[test]
    fn a_terminal_at_a_distro_site_takes_the_wsl_launcher_form() {
        let t = target(
            r"C:\Apps\wt.exe",
            &["-d", "{path}"],
            TargetKind::Terminal,
            CwdMode::None,
        );
        let inv = build(
            &t,
            &site(LocationKind::Wsl, "distro-b", r"\\wsl$\distro-b\srv\x"),
        )
        .unwrap();
        let rendered: Vec<String> = inv
            .argv
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(rendered, vec!["wsl", "-d", "distro-b", "--cd", "/srv/x"]);
    }

    #[test]
    fn a_target_with_no_wsl_form_refuses_a_distro_site_rather_than_guessing() {
        let t = target(
            "/usr/bin/foot",
            &["-D", "{path}"],
            TargetKind::Terminal,
            CwdMode::None,
        );
        assert!(build(
            &t,
            &site(LocationKind::Wsl, "distro-a", r"\\wsl$\distro-a\srv\x")
        )
        .is_none());
    }

    #[test]
    fn a_wait_flag_removed_from_the_row_removes_the_wait() {
        let with = target(
            "/usr/bin/code",
            &["--wait", "{path}"],
            TargetKind::Editor,
            CwdMode::Location,
        );
        let without = target(
            "/usr/bin/code",
            &["{path}"],
            TargetKind::Editor,
            CwdMode::Location,
        );
        assert!(wants_wait(&with));
        assert!(!wants_wait(&without));
        // A terminal editor waits unconditionally: it *is* the session.
        let always = target(
            "/usr/bin/nvim",
            &["{path}"],
            TargetKind::Editor,
            CwdMode::Location,
        );
        assert!(wants_wait(&always));
    }

    #[test]
    fn an_argument_that_is_not_a_placeholder_is_passed_through_unchanged() {
        let t = target(
            "/usr/bin/code",
            &["--new-window", "{path}", "--x={path}"],
            TargetKind::Editor,
            CwdMode::Location,
        );
        let inv = build(&t, &site(LocationKind::Linux, "", "/a b")).unwrap();
        let rendered: Vec<String> = inv
            .argv
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            rendered,
            vec!["--new-window", "/a b", "--x={path}"],
            "substitution is whole-argument, never textual interpolation"
        );
    }
}
