//! argv, the advisory lock, and the two ways the core learns the shell is gone.

use std::path::{Path, PathBuf};

/// Bad or missing argv. There is no window and no pipe peer yet, so this is an exit code.
pub const EXIT_BAD_ARGS: u8 = 2;
/// Another core holds the advisory lock.
pub const EXIT_LOCK_HELD: u8 = 3;

/// The lock file lives beside the database and is never the database. Only the core ever opens
/// the database itself.
///
/// **Held locked, and deliberately empty.** Windows' `LockFileEx` is *mandatory* where Unix
/// `flock` is advisory, so while the core holds this lock no other process can read it — a
/// shell reading it gets `EBUSY` and would conclude no core is running, which is the inverse of
/// the truth. The identifying body therefore lives in [`CORE_OWNER_FILE`].
pub const CORE_LOCK_FILE: &str = "core.lock";

/// `{"pid":…,"started_at":…}`, beside the lock and never locked, so the shell can read it while
/// the core is alive. This is the file `probeCoreLock` reads.
pub const CORE_OWNER_FILE: &str = "core.owner.json";

/// What the shell passed in argv.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreArgs {
    /// The app data directory the shell decided (§2.1): database, lock and owner file.
    pub data_dir: PathBuf,
    /// The epoch this run belongs to (§2.2), which scopes request ids and event sequences.
    pub epoch: u64,
    /// The shell's process id, watched by `OsParentProbe`.
    pub parent_pid: u32,
    /// §13's in-distro worker, on disk, or `None` where this build has none.
    ///
    /// **Optional, and the two absences are different.** A Linux host has no WSL and never
    /// wants one; a Windows build whose installer staged no worker cannot scan a distro and
    /// must say so rather than pretending the distro was empty. Passed rather than derived
    /// because the shell already computes it (`resolveWorkerBinary`), and a second convention
    /// in Rust is one value spelled twice.
    pub worker: Option<PathBuf>,
}

/// Why argv was refused. Each variant names the flag.
#[derive(Debug, PartialEq, Eq)]
pub enum ArgError {
    /// A required flag was not given.
    Missing(&'static str),
    /// A flag's value did not parse, or was empty where empty is malformed.
    Bad(&'static str),
}

impl std::fmt::Display for ArgError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing(k) => write!(f, "missing required argument {k}"),
            Self::Bad(k) => write!(f, "malformed argument {k}"),
        }
    }
}

impl std::error::Error for ArgError {}

/// `--data-dir=<path> --epoch=<u64> --parent-pid=<u32>`, all three required.
///
/// # Errors
/// `ArgError::Missing` when any of the three is absent; `ArgError::Bad` when `--epoch` or
/// `--parent-pid` does not parse, or when `--worker` is given empty.
pub fn parse_args<I: IntoIterator<Item = String>>(argv: I) -> Result<CoreArgs, ArgError> {
    let mut data_dir: Option<PathBuf> = None;
    let mut epoch: Option<u64> = None;
    let mut parent_pid: Option<u32> = None;
    let mut worker: Option<PathBuf> = None;
    for arg in argv {
        if let Some(v) = arg.strip_prefix("--data-dir=") {
            data_dir = Some(PathBuf::from(v));
        } else if let Some(v) = arg.strip_prefix("--epoch=") {
            epoch = Some(v.parse().map_err(|_| ArgError::Bad("--epoch"))?);
        } else if let Some(v) = arg.strip_prefix("--parent-pid=") {
            parent_pid = Some(v.parse().map_err(|_| ArgError::Bad("--parent-pid"))?);
        } else if let Some(v) = arg.strip_prefix("--worker=") {
            // An empty value is malformed rather than absent: the shell omits the flag when it
            // has no worker, so an empty one means it thought it had a path and did not.
            if v.is_empty() {
                return Err(ArgError::Bad("--worker"));
            }
            worker = Some(PathBuf::from(v));
        }
    }
    Ok(CoreArgs {
        data_dir: data_dir.ok_or(ArgError::Missing("--data-dir"))?,
        epoch: epoch.ok_or(ArgError::Missing("--epoch"))?,
        parent_pid: parent_pid.ok_or(ArgError::Missing("--parent-pid"))?,
        worker,
    })
}

/// Why the advisory lock was not taken.
#[derive(Debug)]
pub enum LockError {
    /// The lock file could not be locked: another core holds it.
    Held,
    /// The data directory, the lock file or the owner file could not be created or written.
    Io(std::io::Error),
}

impl std::fmt::Display for LockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Held => write!(f, "another core holds the advisory lock"),
            Self::Io(e) => write!(f, "lock io: {e}"),
        }
    }
}

impl std::error::Error for LockError {}

/// Held for the whole life of the process. The OS releases it when the process dies.
#[derive(Debug)]
pub struct CoreLock {
    file: std::fs::File,
    owner: PathBuf,
}

impl CoreLock {
    /// Takes the lock in `data_dir`, creating the directory if needed, then writes the owner
    /// file beside it.
    ///
    /// # Errors
    /// `LockError::Held` when the lock call fails, whatever the reason; `LockError::Io` when the
    /// directory, the lock file or the owner file cannot be created or written.
    pub fn acquire(data_dir: &Path) -> Result<Self, LockError> {
        std::fs::create_dir_all(data_dir).map_err(LockError::Io)?;
        let file = std::fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(data_dir.join(CORE_LOCK_FILE))
            .map_err(LockError::Io)?;
        <std::fs::File as fs4::FileExt>::try_lock(&file).map_err(|_| LockError::Held)?;
        let started_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or_default();
        let body = format!(
            "{{\"pid\":{},\"started_at\":{started_at}}}\n",
            std::process::id()
        );
        let owner = data_dir.join(CORE_OWNER_FILE);
        std::fs::write(&owner, body).map_err(LockError::Io)?;
        Ok(Self { file, owner })
    }
}

impl Drop for CoreLock {
    fn drop(&mut self) {
        // The owner file goes first: anything that sees it must be able to trust that the lock
        // behind it is still held.
        let _ = std::fs::remove_file(&self.owner);
        let _ = <std::fs::File as fs4::FileExt>::unlock(&self.file);
    }
}

/// How often the watchdog re-checks. The pipe EOF is the fast path; this is the backstop.
pub const PARENT_POLL: std::time::Duration = std::time::Duration::from_secs(2);

/// The second way the core learns the shell is gone.
#[derive(Debug)]
pub struct OsParentProbe {
    pid: u32,
    token: Option<String>,
}

impl OsParentProbe {
    /// A probe for the parent `pid`, recording its identity now so a later process that reuses
    /// the pid is not mistaken for it.
    #[must_use]
    pub fn new(pid: u32) -> Self {
        Self {
            pid,
            token: Self::identity(pid),
        }
    }

    /// Field 22 of `/proc/<pid>/stat` is the process start time. Reading it after the
    /// executable name — which may itself contain spaces and parentheses — means splitting
    /// at the last `)`, not at the first.
    #[cfg(target_os = "linux")]
    fn identity(pid: u32) -> Option<String> {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        let tail = stat.rsplit_once(')')?.1;
        tail.split_whitespace().nth(19).map(ToOwned::to_owned)
    }

    #[cfg(not(target_os = "linux"))]
    // The Linux twin reads `/proc` and cannot be const; a const here would make `new` const on
    // one platform only.
    #[allow(clippy::missing_const_for_fn)]
    fn identity(_pid: u32) -> Option<String> {
        None
    }

    /// True only when the parent is provably gone, or has been replaced by a new process
    /// reusing its pid. Unknown is never reported as gone.
    #[must_use]
    pub fn parent_gone(&self) -> bool {
        self.token
            .as_ref()
            .is_some_and(|known| Self::identity(self.pid).as_ref() != Some(known))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{parse_args, ArgError, CoreLock, LockError};
    use std::path::PathBuf;

    fn argv(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn argv_requires_the_data_dir_the_shell_decided() {
        let e = parse_args(argv(&["--epoch=1", "--parent-pid=9"])).expect_err("must refuse");
        assert_eq!(e, ArgError::Missing("--data-dir"));
    }

    #[test]
    fn argv_parses_all_three() {
        let a =
            parse_args(argv(&["--data-dir=/x/y", "--epoch=4", "--parent-pid=99"])).expect("parse");
        assert_eq!(a.data_dir, PathBuf::from("/x/y"));
        assert_eq!(a.epoch, 4);
        assert_eq!(a.parent_pid, 99);
    }

    /// §13: `--worker` is optional, and the two absences are different.
    ///
    /// Omitted means this build has no in-distro worker and never wanted one. Empty means the
    /// shell thought it had a path and did not — scanning no distro on that would be the silent
    /// failure the flag exists to prevent.
    #[test]
    fn the_worker_path_is_optional_and_an_empty_one_is_malformed() {
        let none = parse_args(argv(&["--data-dir=/x", "--epoch=1", "--parent-pid=2"])).expect("ok");
        assert_eq!(none.worker, None);

        let given = parse_args(argv(&[
            "--data-dir=/x",
            "--epoch=1",
            "--parent-pid=2",
            "--worker=/opt/app/worker/linux-x64/codotheca-worker",
        ]))
        .expect("ok");
        assert_eq!(
            given.worker,
            Some(PathBuf::from("/opt/app/worker/linux-x64/codotheca-worker"))
        );

        let empty = parse_args(argv(&[
            "--data-dir=/x",
            "--epoch=1",
            "--parent-pid=2",
            "--worker=",
        ]))
        .expect_err("must refuse");
        assert_eq!(empty, ArgError::Bad("--worker"));
    }

    #[test]
    fn a_non_numeric_epoch_is_refused_rather_than_defaulted() {
        let e = parse_args(argv(&["--data-dir=/x", "--epoch=soon", "--parent-pid=1"]))
            .expect_err("must refuse");
        assert_eq!(e, ArgError::Bad("--epoch"));
    }

    #[test]
    fn a_second_core_cannot_take_the_lock_and_a_released_one_can() {
        let dir = std::env::temp_dir().join(format!("codotheca-lock-{}", std::process::id()));
        let first = CoreLock::acquire(&dir).expect("first");
        assert!(matches!(CoreLock::acquire(&dir), Err(LockError::Held)));
        drop(first);
        let third = CoreLock::acquire(&dir).expect("after release");
        drop(third);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The shell reads this while the core is alive. Reading the *locked* file instead fails on
    /// Windows with OS error 33, and the probe would then report that no core is running — the
    /// inverse of the truth. This test is what keeps the body out of the locked file.
    #[test]
    fn the_owner_file_names_the_process_and_is_readable_while_the_lock_is_held() {
        let dir = std::env::temp_dir().join(format!("codotheca-lockpid-{}", std::process::id()));
        let held = CoreLock::acquire(&dir).expect("lock");
        let body = std::fs::read_to_string(dir.join(super::CORE_OWNER_FILE)).expect("read");
        let v: serde_json::Value = serde_json::from_str(&body).expect("json");
        assert_eq!(
            v.get("pid").and_then(serde_json::Value::as_u64),
            Some(u64::from(std::process::id()))
        );
        drop(held);
        assert!(
            !dir.join(super::CORE_OWNER_FILE).exists(),
            "a released lock must not leave an owner file naming a dead process"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn our_own_process_is_never_reported_gone() {
        let probe = super::OsParentProbe::new(std::process::id());
        assert!(!probe.parent_gone());
    }

    #[test]
    fn an_unresolvable_parent_is_unknown_not_gone() {
        // pid 0 is not a pollable process on either target.
        let probe = super::OsParentProbe::new(0);
        assert!(
            !probe.parent_gone(),
            "unknown must never be reported as gone"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_parent_that_exits_is_reported_gone() {
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("spawn a stand-in parent");
        let probe = super::OsParentProbe::new(child.id());
        assert!(!probe.parent_gone());
        child.kill().expect("kill");
        child.wait().expect("reap");
        assert!(probe.parent_gone());
    }
}
