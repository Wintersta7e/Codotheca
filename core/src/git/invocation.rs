//! §3.2: every invocation is config-neutralised, argv only, never a shell.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::repo::RepoHandle;

/// The options that precede every subcommand.
///
/// `-C <work_dir>` is how the repository is selected; nothing here ever interpolates a path
/// into a string a shell could see, because no shell is involved at any point.
#[must_use]
pub fn base_args(repo: &RepoHandle, hooks_dir: &Path) -> Vec<OsString> {
    let mut v: Vec<OsString> = Vec::with_capacity(18);
    v.push(OsString::from("-C"));
    v.push(repo.work_dir.clone().into_os_string());
    v.push(OsString::from("--no-optional-locks"));

    let mut push_cfg = |value: OsString| {
        v.push(OsString::from("-c"));
        v.push(value);
    };
    push_cfg(OsString::from("core.fsmonitor=false"));
    let mut hooks = OsString::from("core.hooksPath=");
    hooks.push(hooks_dir);
    push_cfg(hooks);
    push_cfg(OsString::from("protocol.ext.allow=never"));
    push_cfg(OsString::from("diff.external="));
    push_cfg(OsString::from("core.askPass="));
    push_cfg(OsString::from("credential.helper="));

    if repo.trusted {
        let mut safe = OsString::from("safe.directory=");
        safe.push(&repo.work_dir);
        v.push(OsString::from("-c"));
        v.push(safe);
    }
    v
}

/// Variables no git child inherits, by exact name.
///
/// The first fourteen redirect the repository, its index or objects, or hand git a program to
/// run. The rest are §47.3's: config by environment (`GIT_CONFIG_COUNT`, `GIT_CONFIG_PARAMETERS`)
/// deleted a branch and a tag under the audited fetch (§47 M2); a user's `GIT_ALLOW_PROTOCOL`
/// overrides every `protocol.*.allow` pin, and the write path sets its own; `GIT_EXEC_PATH`
/// substitutes the `git-remote-*` helpers, which are programs; and the graft, replace-base and
/// shallow files each rewrite the graph a read walks.
const SCRUBBED: [&str; 22] = [
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_COMMON_DIR",
    "GIT_CEILING_DIRECTORIES",
    "GIT_NAMESPACE",
    "GIT_EXTERNAL_DIFF",
    "GIT_PAGER",
    "GIT_EDITOR",
    "GIT_ASKPASS",
    "SSH_ASKPASS",
    "GIT_CONFIG",
    "GIT_CONFIG_COUNT",
    "GIT_CONFIG_PARAMETERS",
    "GIT_ALLOW_PROTOCOL",
    "GIT_PROTOCOL_FROM_USER",
    "GIT_EXEC_PATH",
    "GIT_GRAFT_FILE",
    "GIT_REPLACE_REF_BASE",
    "GIT_SHALLOW_FILE",
];

/// Variable families no git child inherits, by prefix.
///
/// **`GIT_CONFIG_KEY_` and `GIT_CONFIG_VALUE_`, never `GIT_CONFIG_`**: that shorter prefix would
/// take `GIT_CONFIG_GLOBAL`, the user's own config file, which §47.3 keeps. `GIT_TRACE` covers
/// `GIT_TRACE` itself and every `GIT_TRACE*` sibling — a trace file receives what crosses a
/// transport helper.
const SCRUBBED_PREFIXES: [&str; 3] = ["GIT_CONFIG_KEY_", "GIT_CONFIG_VALUE_", "GIT_TRACE"];

/// Whether an inherited variable belongs to the scrub.
///
/// Compared ASCII-case-insensitively on every platform. Windows names are case-insensitive, so
/// `git_trace` there is `GIT_TRACE`; on Unix git reads only the upper-case spelling, and removing
/// a lower-case one it would never read changes nothing.
fn is_scrubbed(name: &std::ffi::OsStr) -> bool {
    let Some(name) = name.to_str() else {
        return false;
    };
    let upper = name.to_ascii_uppercase();
    SCRUBBED.contains(&upper.as_str())
        || SCRUBBED_PREFIXES
            .iter()
            .any(|prefix| upper.starts_with(prefix))
}

/// Variables git must not inherit, and the three it must have.
///
/// `LC_ALL=C` is load-bearing rather than cosmetic: `error::classify` matches git's English
/// message text, so a localised build would classify a sharing violation as an unknown failure.
///
/// **Prefixed families are found by enumerating this process's environment**, never by index
/// `0..GIT_CONFIG_COUNT`: git reads `GIT_CONFIG_KEY_<n>` for every `n` below the count, and a
/// parent can set the count and the keys independently. Every read and every write child calls
/// this (§47.3). Kept on purpose: the user's transport (`SSH_AUTH_SOCK`, `GIT_SSH`,
/// `GIT_SSH_COMMAND`, the proxies) and `GIT_CONFIG_GLOBAL`.
pub fn neutralise_env(cmd: &mut Command) {
    cmd.env("GIT_TERMINAL_PROMPT", "0");
    cmd.env("GIT_CONFIG_NOSYSTEM", "1");
    cmd.env("LC_ALL", "C");
    for name in SCRUBBED {
        cmd.env_remove(name);
    }
    for (name, _) in std::env::vars_os() {
        if is_scrubbed(&name) {
            cmd.env_remove(name);
        }
    }
}

/// The leaf name, inside the empty hooks directory, that `GIT_GRAFT_FILE` is pointed at.
///
/// `--no-replace-objects` does not disable grafts (measured on git 2.43, §45.3(a)); only a graft
/// file path that does not exist does. The hooks directory is kept empty, so this path never
/// exists. **One owner**, beside [`EMPTY_HOOKS_DIR_NAME`], shared by the verifying read and the
/// analyser's walk.
pub const ABSENT_GRAFT_FILE: &str = "absent-graft-file";

/// The absent graft file's full path inside `hooks_dir`.
#[must_use]
pub fn absent_graft_path(hooks_dir: &Path) -> PathBuf {
    hooks_dir.join(ABSENT_GRAFT_FILE)
}

/// The leaf name of the empty hooks directory.
///
/// Exported because §13's worker cleanup has to remove the very directory this creates, and
/// `rmdir` refuses one that still holds anything — spelling the name a second time over there
/// would make that cleanup fail for every stale build, silently and forever.
pub const EMPTY_HOOKS_DIR_NAME: &str = "git-hooks-empty";

/// Create the empty directory `core.hooksPath` points at, under the app data directory the
/// shell passed in argv (§2.1).
///
/// # Errors
///
/// The I/O error when the directory cannot be created, e.g. the app data directory is not
/// writable.
pub fn ensure_empty_hooks_dir(app_data_dir: &Path) -> std::io::Result<PathBuf> {
    let dir = app_data_dir.join(EMPTY_HOOKS_DIR_NAME);
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}
