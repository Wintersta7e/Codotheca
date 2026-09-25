//! Spawning git (§3.4).
//!
//! Two structural guarantees:
//!
//! * every child goes into a process group (Linux) or Job Object (Windows) via
//!   `group_spawn`, so [`GitExec::run_piped`] tears down the whole tree and leaves no orphan
//!   holding a file lock;
//! * [`GitExec::run_piped`] takes a writer closure and a reader closure and runs each on its
//!   own thread, closing stdin when the writer returns. Writing everything and only then
//!   reading deadlocks once git's stdout fills the pipe buffer, and this API gives a caller no
//!   way to express that shape.

use std::ffi::OsStr;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use command_group::CommandGroup;

use crate::cancel::CancelToken;

use super::error::{classify, classify_spawn, GitError, GitResult};
use super::invocation::{base_args, neutralise_env};
use super::repo::RepoHandle;

/// The per-invocation budget. `None` is J4's no-deadline case (§4.1).
#[derive(Debug, Clone, Copy)]
pub struct RunLimits {
    /// Wall-clock ceiling before the process tree is killed.
    pub deadline: Option<Duration>,
    /// The one non-zero exit code that is an *answer* rather than a failure, if any.
    ///
    /// Every subcommand the core runs succeeds with `0` except `check-ignore`, which exits `1`
    /// to mean "none of these paths is ignored" — the usual answer in a repository with no
    /// ignore rules at all. Raising that as an error would force the caller to guess, and both
    /// guesses are wrong: treating it as "all ignored" credits nothing in such a repository, and
    /// treating it as "none ignored" credits the dev-server writes §9 exists to exclude.
    pub tolerated_exit: Option<i32>,
}

impl RunLimits {
    /// No deadline.
    #[must_use]
    pub const fn none() -> Self {
        Self {
            deadline: None,
            tolerated_exit: None,
        }
    }

    /// Kill the tree after `d`.
    #[must_use]
    pub const fn after(d: Duration) -> Self {
        Self {
            deadline: Some(d),
            tolerated_exit: None,
        }
    }

    /// Accept `code` as an answer, keeping the stdout that came with it.
    #[must_use]
    pub const fn tolerating(self, code: i32) -> Self {
        Self {
            tolerated_exit: Some(code),
            ..self
        }
    }
}

/// A completed invocation's captured streams.
#[derive(Debug, Clone)]
pub struct GitOutput {
    /// Everything git wrote to stdout.
    pub stdout: Vec<u8>,
    /// Everything git wrote to stderr.
    pub stderr: Vec<u8>,
}

/// The git binary plus the empty hooks directory every invocation points at.
#[derive(Debug, Clone)]
pub struct GitExec {
    git: PathBuf,
    hooks_dir: PathBuf,
}

/// Why the wait loop stopped watching the child.
enum Stop {
    Exited(ExitStatus),
    Cancelled,
    Deadline(u64),
}

impl GitExec {
    /// Use an explicit binary — the WSL worker (§13) passes its in-distro path.
    #[must_use]
    pub const fn new(git: PathBuf, hooks_dir: PathBuf) -> Self {
        Self { git, hooks_dir }
    }

    /// Use `git` from `PATH`.
    #[must_use]
    pub fn system(hooks_dir: PathBuf) -> Self {
        Self::new(PathBuf::from("git"), hooks_dir)
    }

    /// The empty hooks directory every invocation points `core.hooksPath` at. `git --version`
    /// borrows it as a working directory, because `--version` needs no repository.
    #[must_use]
    pub fn hooks_dir(&self) -> &std::path::Path {
        &self.hooks_dir
    }

    /// Run a subcommand that needs no stdin and buffer all of its stdout.
    ///
    /// # Errors
    ///
    /// As [`Self::run_piped`].
    pub fn run(
        &self,
        repo: &RepoHandle,
        args: &[&OsStr],
        limits: RunLimits,
        cancel: &CancelToken,
    ) -> GitResult<GitOutput> {
        let stdout = self.run_piped(
            repo,
            args,
            limits,
            cancel,
            |_stdin| Ok(()),
            |out| {
                let mut buf = Vec::new();
                out.read_to_end(&mut buf)?;
                Ok(buf)
            },
        )?;
        Ok(GitOutput {
            stdout,
            stderr: Vec::new(),
        })
    }

    /// Run a subcommand, writing stdin on one thread and draining stdout on another.
    ///
    /// `write_stdin` receives a writer; when it returns, the handle is dropped so git sees
    /// EOF. `read_stdout` receives a buffered reader and returns whatever the caller wants to
    /// keep — it should stream rather than collect when the output is large.
    ///
    /// # Errors
    ///
    /// `GitError::Cancelled` when `cancel` fires before or during the run; `GitError::Budget`
    /// when the deadline is zero or elapses; `GitError::Missing`, `PermissionDenied` or `Internal`
    /// when the spawn fails; the [`classify`]d failure when git exits non-zero with a code
    /// `limits` does not tolerate; `GitError::Internal` when writing stdin or reading stdout fails.
    pub fn run_piped<W, R, T>(
        &self,
        repo: &RepoHandle,
        args: &[&OsStr],
        limits: RunLimits,
        cancel: &CancelToken,
        write_stdin: W,
        read_stdout: R,
    ) -> GitResult<T>
    where
        W: FnOnce(&mut dyn Write) -> std::io::Result<()> + Send + 'static,
        R: FnOnce(&mut dyn BufRead) -> std::io::Result<T> + Send + 'static,
        T: Send + 'static,
    {
        cancel.check()?;
        // A budget of zero has nothing left to spend, and the poll loop below cannot enforce it:
        // it reads `try_wait` first, so a subcommand that exits inside the first 250 µs poll is an
        // `Exited` result and the deadline never fires at all. `git status` on a one-file
        // repository does exactly that on a fast machine, which is how
        // `a_deadline_of_zero_is_a_budget_failure_not_a_missing_repository` passed here and in the
        // `core` job and failed in `acceptance` on the same commit. Refusing here is also the
        // honest behaviour — a caller holding no budget must not start a process — and it is the
        // shape of the cancellation check above.
        if limits.deadline == Some(Duration::ZERO) {
            return Err(GitError::Budget { after_ms: 0 });
        }

        let mut cmd = Command::new(&self.git);
        cmd.args(base_args(repo, &self.hooks_dir));
        cmd.args(args);
        neutralise_env(&mut cmd);
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = cmd.group_spawn().map_err(|e| classify_spawn(&e))?;
        let missing = || GitError::Internal {
            detail: "child pipe was not created".to_owned(),
        };
        let stdin = child.inner().stdin.take().ok_or_else(missing)?;
        let stdout = child.inner().stdout.take().ok_or_else(missing)?;
        let stderr = child.inner().stderr.take().ok_or_else(missing)?;

        let writer = thread::spawn(move || {
            let mut handle = stdin;
            let outcome = write_stdin(&mut handle);
            drop(handle); // git sees EOF here, and only here.
            outcome
        });
        let reader = thread::spawn(move || {
            let mut buffered = BufReader::new(stdout);
            read_stdout(&mut buffered)
        });
        let errors = thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = BufReader::new(stderr).read_to_end(&mut buf);
            buf
        });

        let started = Instant::now();
        let mut poll = Duration::from_micros(250);
        let stop = loop {
            match child.try_wait() {
                Ok(Some(status)) => break Stop::Exited(status),
                Ok(None) => {}
                Err(e) => return Err(classify_spawn(&e)),
            }
            if cancel.is_cancelled() {
                let _ = child.kill();
                let _ = child.wait();
                break Stop::Cancelled;
            }
            if let Some(d) = limits.deadline {
                let elapsed = started.elapsed();
                if elapsed >= d {
                    let _ = child.kill();
                    let _ = child.wait();
                    break Stop::Deadline(u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX));
                }
            }
            thread::sleep(poll);
            poll = (poll * 2).min(Duration::from_millis(10));
        };

        let stderr_bytes = errors.join().unwrap_or_default();
        let wrote = writer.join().unwrap_or(Ok(()));
        let read = reader
            .join()
            .unwrap_or_else(|_| Err(std::io::Error::other("stdout reader panicked")));

        match stop {
            Stop::Cancelled => return Err(GitError::Cancelled),
            Stop::Deadline(ms) => return Err(GitError::Budget { after_ms: ms }),
            Stop::Exited(status) => {
                if !status.success() {
                    let code = status.code().unwrap_or(-1);
                    if limits.tolerated_exit != Some(code) {
                        return Err(classify(code, &stderr_bytes));
                    }
                }
            }
        }

        if let Err(e) = wrote {
            // A subcommand that ignores stdin closes it early; that is not a failure.
            if e.kind() != std::io::ErrorKind::BrokenPipe {
                return Err(GitError::Internal {
                    detail: e.to_string(),
                });
            }
        }
        read.map_err(|e| GitError::Internal {
            detail: e.to_string(),
        })
    }
}
