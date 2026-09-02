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

/// Variables git must not inherit, and the three it must have.
///
/// `LC_ALL=C` is load-bearing rather than cosmetic: `error::classify` matches git's English
/// message text, so a localised build would classify a sharing violation as an unknown failure.
pub fn neutralise_env(cmd: &mut Command) {
    cmd.env("GIT_TERMINAL_PROMPT", "0");
    cmd.env("GIT_CONFIG_NOSYSTEM", "1");
    cmd.env("LC_ALL", "C");
    for k in [
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
    ] {
        cmd.env_remove(k);
    }
}

/// The leaf name of the empty hooks directory. Exported because §13's worker cleanup has to
/// remove the very directory this creates, and `rmdir` refuses one that still holds anything —
/// spelling the name a second time over there would make that cleanup fail for every stale
/// build, silently and forever.
pub const EMPTY_HOOKS_DIR_NAME: &str = "git-hooks-empty";

/// Create the empty directory `core.hooksPath` points at, under the app data directory the
/// shell passed in argv (§2.1).
pub fn ensure_empty_hooks_dir(app_data_dir: &Path) -> std::io::Result<PathBuf> {
    let dir = app_data_dir.join(EMPTY_HOOKS_DIR_NAME);
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}
