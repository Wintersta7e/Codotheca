//! §24.1b: the write invocation, and the second and last git spawn site.
//!
//! This is a **second builder, not a widened first one**. `crate::git::base_args` is asserted
//! byte-identical by criterion 63 and is shared by every read call site in the product; widening
//! it to carry a credential would change the invocation for all of them. What the two share is
//! [`crate::git::neutralise_env`], reused here **verbatim** — every variable it sets and every one
//! it removes is required on the write path too, `GIT_ASKPASS` and `SSH_ASKPASS` included. The
//! *"minus the credential neutralisation"* the plan describes is the **argv** half, `base_args`'s
//! `-c credential.helper=`, not the environment half.

use std::ffi::OsString;
use std::io::{BufRead, BufReader, Read};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use command_group::CommandGroup;

use crate::cancel::CancelToken;
use crate::git::{classify_spawn, neutralise_env, GitError, GitResult};
use crate::gitw::credential::CredentialChannel;
use crate::gitw::intent::Intent;

/// The filter drivers configured in the effective config, **and the proof that they were read**.
///
/// §24.1b requires `Intent::Clone` to neutralise every configured filter driver, because a clone
/// checks out and `.gitattributes` from a remote repository would otherwise run them. That
/// enumeration is the one git invocation in this module that **neither audit can see**: it is not
/// in `core/src/git/`, so the read audit's directory scan misses it, and it is not an `Intent`
/// variant, so the write audit's enumeration of `Intent::ALL` misses it too. If it silently
/// returned nothing, every later argv would render without the neutralisation and **both audits
/// would stay green** — R88's shape aimed at a precondition instead of a surface.
///
/// So the type closes it rather than a check: [`FilterDrivers::enumerated`] is the **only**
/// constructor, there is deliberately no `Default` and no empty constructor, and a `WriteEnv` can
/// therefore only hold drivers that came from a read which completed. *Enumerated and found none*
/// and *never enumerated* are different facts, and a type that cannot express the second cannot
/// let it pass for the first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilterDrivers {
    names: Vec<String>,
}

impl FilterDrivers {
    /// The only constructor, named for what it proves. An empty list here is a real answer: the
    /// effective config was read and declares no filter driver.
    #[must_use]
    pub fn enumerated(names: Vec<String>) -> FilterDrivers {
        FilterDrivers { names }
    }

    /// The driver names, in the order the enumeration returned them.
    #[must_use]
    pub fn names(&self) -> &[String] {
        &self.names
    }
}

/// Everything a write invocation needs beyond the [`Intent`] itself.
#[derive(Debug, Clone)]
pub struct WriteEnv {
    /// The repository a `Fetch` runs in.
    ///
    /// `None` for a clone: its destination **does not exist yet**, which is §24.1's precondition,
    /// and rendering `-C` for it would name a directory git is about to create.
    pub work_dir: Option<PathBuf>,
    /// The empty directory `core.hooksPath` points at.
    ///
    /// It lives here rather than on [`WriteExec`] because `write_base_args` renders it and takes
    /// only the intent and this environment — holding it in both places would be one value stated
    /// twice, with a security control as the value.
    pub hooks_dir: PathBuf,
    /// Exactly one `-c credential.helper=<value>`, never absent.
    pub credential: CredentialChannel,
    /// The drivers to neutralise, carrying the proof they were enumerated.
    pub filters: FilterDrivers,
}

fn cfg(argv: &mut Vec<OsString>, value: OsString) {
    argv.push(OsString::from("-c"));
    argv.push(value);
}

/// Pull driver names out of `git config --null --get-regexp` output.
///
/// Each record is `key\nvalue` and records are NUL-separated. The key is
/// `filter.<driver>.<attribute>` and **the driver may itself contain dots**, so the name is what
/// lies between the first `.` and the last — taking `split('.').nth(1)` would truncate
/// `filter.my.driver.clean` to `my` and neutralise a driver that does not exist while leaving the
/// real one live.
fn parse_filter_drivers(stdout: &[u8]) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for record in stdout.split(|b| *b == 0) {
        let Ok(text) = std::str::from_utf8(record) else {
            continue;
        };
        let key = text.split('\n').next().unwrap_or("");
        let Some(rest) = key.strip_prefix("filter.") else {
            continue;
        };
        let Some((driver, _attribute)) = rest.rsplit_once('.') else {
            continue;
        };
        if !driver.is_empty() && !names.iter().any(|n| n == driver) {
            names.push(driver.to_owned());
        }
    }
    names
}

/// The options that precede the subcommand on the **write** path.
///
/// Unchanged from §3.2's read invocation: `--no-optional-locks`, `core.fsmonitor=false`,
/// `core.hooksPath=<empty-dir>`, `protocol.ext.allow=never`, `diff.external=`, `core.askPass=`,
/// argv only and never a shell. What differs is exactly one thing — `credential.helper` takes an
/// explicit value from [`CredentialChannel`] — plus the filter neutralisation a checkout needs.
#[must_use]
pub fn write_base_args(intent: &Intent, env: &WriteEnv) -> Vec<OsString> {
    let mut argv: Vec<OsString> = Vec::with_capacity(24);
    // [p2-24b] The **intent's** repository wins, because it is the one the type guarantees. The
    // env's stays as the fallback for a caller that has one and an intent that does not; a fetch
    // now carries its own, so the two cannot disagree about which repository is written to.
    if let Some(dir) = intent
        .work_dir()
        .map(std::path::Path::to_path_buf)
        .or_else(|| env.work_dir.clone())
    {
        argv.push(OsString::from("-C"));
        argv.push(dir.into_os_string());
    }
    argv.push(OsString::from("--no-optional-locks"));
    cfg(&mut argv, OsString::from("core.fsmonitor=false"));
    let mut hooks = OsString::from("core.hooksPath=");
    hooks.push(&env.hooks_dir);
    cfg(&mut argv, hooks);
    cfg(&mut argv, OsString::from("protocol.ext.allow=never"));
    cfg(&mut argv, OsString::from("diff.external="));
    cfg(&mut argv, OsString::from("core.askPass="));
    argv.extend(env.credential.helper_args());
    // §3.2's parenthesis — *"clean/smudge filters are not disabled … but phase 1 never checks
    // out"* — becomes load-bearing here, because a clone checks out.
    for driver in env.filters.names() {
        cfg(
            &mut argv,
            OsString::from(format!("filter.{driver}.process=")),
        );
        cfg(&mut argv, OsString::from(format!("filter.{driver}.clean=")));
        cfg(
            &mut argv,
            OsString::from(format!("filter.{driver}.smudge=")),
        );
    }
    let _ = intent;
    argv
}

/// The second and last git spawn site in the product.
///
/// The filter enumeration below spawns from **this file** deliberately: a separate helper
/// elsewhere in the core would be a fourth `Command::new` site and `no-unaudited-git-spawn` pins
/// three. The write module owning its own precondition read is what keeps that list at three
/// without an exception.
#[derive(Debug, Clone)]
pub struct WriteExec {
    git: PathBuf,
}

enum Stop {
    Exited(std::process::ExitStatus),
    Cancelled,
}

impl WriteExec {
    /// Point the write path at a git binary.
    #[must_use]
    pub fn new(git: PathBuf) -> WriteExec {
        WriteExec { git }
    }

    /// Enumerate the filter drivers the effective config declares.
    ///
    /// **The three outcomes are three different facts and this function keeps them apart.**
    /// `git config --get-regexp` exits `0` having matched, exits **`1` having matched nothing**,
    /// and exits above that on a real failure. Collapsing the middle one into the last would
    /// refuse every ordinary clone; collapsing it into a silent empty list is the defect
    /// [`FilterDrivers`] exists to prevent. So: `0` and `1` both produce an enumeration — one
    /// with names, one empty and **known** to be empty — and anything else is an error, on which
    /// the caller refuses the clone rather than running it unfiltered.
    pub fn filter_drivers(&self) -> GitResult<FilterDrivers> {
        let mut cmd = Command::new(&self.git);
        cmd.arg("--no-optional-locks");
        cmd.args([
            "config",
            "--null",
            "--get-regexp",
            r"^filter\..*\.(clean|smudge|process)$",
        ]);
        neutralise_env(&mut cmd);
        cmd.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let out = cmd.output().map_err(|e| classify_spawn(&e))?;
        match out.status.code() {
            Some(0) => Ok(FilterDrivers::enumerated(parse_filter_drivers(&out.stdout))),
            // Matched nothing. The config was read and declares no filter driver.
            Some(1) => Ok(FilterDrivers::enumerated(Vec::new())),
            other => Err(GitError::Internal {
                detail: format!(
                    "filter enumeration failed with {}; refusing rather than cloning unfiltered",
                    other.map_or_else(|| "a signal".to_owned(), |c| c.to_string())
                ),
            }),
        }
    }

    /// Run one intent to completion, streaming the child's stderr a line at a time.
    ///
    /// Spawned through `CommandGroup::group_spawn` exactly as `core/src/git/exec.rs` does, so a
    /// cancel kills the **group** rather than the child: killing a Windows child without a Job
    /// Object leaves orphans holding file locks, and the staging removal then fails with *Access
    /// is denied*.
    ///
    /// The child's stdout and stderr are read by the core and **never forwarded** — the core's own
    /// stdout carries protocol frames and nothing else. `--progress` writes to stderr, which is
    /// where the stage parser reads, so stderr is delivered to `on_stderr` on the calling thread
    /// while stdout is drained on another. Draining both is what stops a chatty child filling a
    /// pipe buffer and blocking forever.
    pub fn run(
        &self,
        intent: &Intent,
        env: &WriteEnv,
        cancel: &CancelToken,
        on_stderr: &mut dyn FnMut(&str),
    ) -> GitResult<()> {
        cancel.check()?;

        let mut cmd = Command::new(&self.git);
        cmd.args(write_base_args(intent, env));
        cmd.args(intent.argv());
        neutralise_env(&mut cmd);
        cmd.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = cmd.group_spawn().map_err(|e| classify_spawn(&e))?;
        let missing = || GitError::Internal {
            detail: "child pipe was not created".to_owned(),
        };
        let stdout = child.inner().stdout.take().ok_or_else(missing)?;
        let stderr = child.inner().stderr.take().ok_or_else(missing)?;

        let drain = thread::spawn(move || {
            let mut sink = Vec::new();
            let _ = BufReader::new(stdout).read_to_end(&mut sink);
        });
        let (tx, rx) = mpsc::channel::<String>();
        let lines = thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                if tx.send(line).is_err() {
                    break;
                }
            }
        });

        let started = Instant::now();
        let mut poll = Duration::from_micros(250);
        let stop = loop {
            while let Ok(line) = rx.try_recv() {
                on_stderr(&line);
            }
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
            thread::sleep(poll);
            poll = (poll * 2).min(Duration::from_millis(10));
        };

        // The reader threads end when their pipes close, which the exit or the kill above has
        // already caused. Draining after the join is what delivers the last progress lines.
        let _ = lines.join();
        let _ = drain.join();
        while let Ok(line) = rx.try_recv() {
            on_stderr(&line);
        }

        match stop {
            Stop::Cancelled => Err(GitError::Cancelled),
            Stop::Exited(status) if status.success() => Ok(()),
            Stop::Exited(status) => Err(GitError::Internal {
                detail: format!(
                    "git exited with {} after {} ms",
                    status.code().unwrap_or(-1),
                    started.elapsed().as_millis()
                ),
            }),
        }
    }
}
