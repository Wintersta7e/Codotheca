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
use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use command_group::CommandGroup;

use crate::cancel::CancelToken;
use crate::git::{classify_spawn, neutralise_env, transport_refused, GitError, GitResult};
use crate::gitw::backend::RunOutput;
use crate::gitw::credential::CredentialChannel;
use crate::gitw::intent::{Intent, StdoutUse};

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
    pub const fn enumerated(names: Vec<String>) -> Self {
        Self { names }
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
    /// The repository an intent runs in, when the intent itself names none.
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
    // env's stays as the fallback for a caller that has one and an intent that does not; the
    // verifying read carries its own, so the two cannot disagree about which repository is read.
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
    // [p2-24b] §24.1: *"An invocation may create bytes and may never remove or overwrite one."*
    //
    // **`git fetch` spawns `git maintenance run --auto`, and that prunes.** Measured, not
    // reasoned about: `GIT_TRACE=1 git -c gc.auto=1 fetch` shows
    // `run_command: git maintenance run --auto --no-quiet` as a child, and the same fetch with
    // these three renders no such line. The threshold is `gc.auto`'s 6,700 loose objects, so
    // without them the invariant holds by luck about a repository's shape rather than by
    // construction — and §24.7C's pre-flight now runs a real fetch against a user's working copy.
    //
    // The corpus generator has set all three since it was written (`core/src/corpus/git_cmd.rs`),
    // for determinism. The write path needs them for a stronger reason.
    cfg(&mut argv, OsString::from("gc.auto=0"));
    cfg(&mut argv, OsString::from("gc.autoDetach=false"));
    cfg(&mut argv, OsString::from("maintenance.auto=false"));
    // §47.3's two uniform pins, on every intent. A `fetch.bundleURI` in the user's config wrote
    // `refs/bundles/*`, and a creation-token list wrote `.git/config`, under every other pin —
    // measured on both platforms, even when the fetch itself was refused (§47 M3). Only the
    // empty value stopped it; `transfer.bundleURI=false` stops a server-advertised list.
    cfg(&mut argv, OsString::from("fetch.bundleURI="));
    cfg(&mut argv, OsString::from("transfer.bundleURI=false"));
    // The intent's own `-c` pins (§47.3), each exactly once.
    for pin in intent.config_pins() {
        cfg(&mut argv, OsString::from(*pin));
    }
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
    argv
}

/// The one permitted difference between a test's write child and the production one.
///
/// §47.9 C, D-6: the local fixtures are reached over `file`, which production's
/// `GIT_ALLOW_PROTOCOL` refuses. It appends `file` to the list of every intent that uses a transport, **rendered
/// last**, and the verifying read admits a `file://` URL under its root as a network remote.
/// Layer C asserts it is the only difference.
#[cfg(feature = "testkit")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransportFixture {
    root: PathBuf,
}

#[cfg(feature = "testkit")]
impl TransportFixture {
    /// A fixture whose local remotes all live under `root`.
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Where the fixture's local remotes live.
    #[must_use]
    pub fn root(&self) -> &std::path::Path {
        &self.root
    }

    /// The intent's transport list with `file` appended, for every intent that uses one.
    #[must_use]
    pub fn widened(&self, intent: &Intent) -> Option<String> {
        match intent {
            Intent::Clone { .. } | Intent::VerifyRead { .. } => {
                Some(format!("{}:file", intent.allowed_protocols()))
            }
        }
    }

    /// Is `url` one of this fixture's local remotes — a `file://` URL or an absolute path under
    /// its root? The verifying read classifies such a URL as network, and nothing else.
    #[must_use]
    pub fn admits(&self, url: &str) -> bool {
        let normal = |text: &str| text.replace('\\', "/").trim_end_matches('/').to_owned();
        let path = url.strip_prefix("file://").unwrap_or(url);
        // `file:///C:/…` on Windows carries one slash before the drive.
        let path = path
            .strip_prefix('/')
            .filter(|rest| rest.as_bytes().get(1) == Some(&b':'))
            .unwrap_or(path);
        let root = normal(&self.root.to_string_lossy());
        !root.is_empty() && normal(path).starts_with(&root)
    }
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
    /// Test-only: widens the transport list for local fixtures, and nothing else.
    #[cfg(feature = "testkit")]
    fixture: Option<TransportFixture>,
}

enum Stop {
    Exited(std::process::ExitStatus),
    Cancelled,
    /// The intent's deadline elapsed after this many milliseconds and the group was killed.
    Deadline(u64),
}

/// The child's output once the wait is over.
struct Finished {
    stop: Stop,
    elapsed_ms: u128,
}

impl WriteExec {
    /// Point the write path at a git binary.
    #[must_use]
    pub const fn new(git: PathBuf) -> Self {
        Self {
            git,
            #[cfg(feature = "testkit")]
            fixture: None,
        }
    }

    /// The write path with the test's one permitted difference, [`TransportFixture`].
    #[cfg(feature = "testkit")]
    #[must_use]
    pub const fn with_transport_fixture(git: PathBuf, fixture: TransportFixture) -> Self {
        Self {
            git,
            fixture: Some(fixture),
        }
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
    ///
    /// # Errors
    /// `GitError::Missing` when git is not found, `PermissionDenied` when it cannot be run, and
    /// `Internal` for any other spawn failure or an exit above `1` or by signal.
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
    /// **Streams (§47.3).** Stdin, when the intent has a payload, is written on its own thread and
    /// then closed; otherwise it is `Stdio::null()`. Stdout is kept when the intent parses it and
    /// drained otherwise — never forwarded, since the core's own stdout carries protocol frames
    /// and nothing else. Stderr is delivered to `on_stderr` on the calling thread.
    ///
    /// **The intent's deadline is enforced here, by the core.** On expiry the whole process group
    /// is killed and waited for — a `git-remote-https` left running would hold the connection —
    /// and the call returns `Budget`.
    ///
    /// **Environment.** The intent's own pins go on after `neutralise_env` has removed the
    /// parent's values, so a user's `GIT_ALLOW_PROTOCOL` can neither widen nor survive them.
    ///
    /// # Errors
    /// `GitError::Cancelled` when `cancel` fires before the spawn or while the child runs;
    /// `GitError::Budget` when the intent's deadline elapses; the spawn's classification
    /// (`Missing`, `PermissionDenied`, `Internal`) when the group cannot be started or waited on;
    /// `TransportRefused` when git refused a transport the intent does not list; and `Internal`
    /// when a pipe is missing, stdin cannot be written, or git exits unsuccessfully otherwise.
    pub fn run(
        &self,
        intent: &Intent,
        env: &WriteEnv,
        cancel: &CancelToken,
        on_stderr: &mut dyn FnMut(&str),
    ) -> GitResult<RunOutput> {
        cancel.check()?;

        let mut cmd = Command::new(&self.git);
        cmd.args(write_base_args(intent, env));
        cmd.args(intent.argv());
        neutralise_env(&mut cmd);
        for (key, value) in intent.env_pins(&env.hooks_dir) {
            cmd.env(key, value);
        }
        #[cfg(feature = "testkit")]
        if let Some(widened) = self.fixture.as_ref().and_then(|f| f.widened(intent)) {
            cmd.env("GIT_ALLOW_PROTOCOL", widened);
        }
        let payload = intent.stdin_payload();
        cmd.stdin(if payload.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

        let mut child = cmd.group_spawn().map_err(|e| classify_spawn(&e))?;
        let missing = || GitError::Internal {
            detail: "child pipe was not created".to_owned(),
        };
        let writer = match payload {
            Some(bytes) => {
                let mut stdin = child.inner().stdin.take().ok_or_else(missing)?;
                // Written on its own thread and then closed, while stdout and stderr drain on
                // others: writing everything before reading deadlocks once a pipe fills (§3.3).
                Some(thread::spawn(move || {
                    let outcome = stdin.write_all(&bytes).and_then(|()| stdin.flush());
                    drop(stdin);
                    outcome
                }))
            }
            None => None,
        };
        let stdout = child.inner().stdout.take().ok_or_else(missing)?;
        let stderr = child.inner().stderr.take().ok_or_else(missing)?;

        let keep = intent.stdout_use() == StdoutUse::Parse;
        let reader = thread::spawn(move || {
            let mut sink = Vec::new();
            let _ = BufReader::new(stdout).read_to_end(&mut sink);
            if keep {
                sink
            } else {
                Vec::new()
            }
        });
        let (tx, rx) = mpsc::channel::<String>();
        let lines = thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                if tx.send(line).is_err() {
                    break;
                }
            }
        });

        // Every stderr line reaches the caller, and the one that names a refused transport is
        // kept: it is what turns a failed exit into `TransportRefused` rather than `Internal`.
        let mut refused: Option<String> = None;
        let mut deliver = |line: &str| {
            if refused.is_none() {
                refused = transport_refused(line);
            }
            on_stderr(line);
        };
        let finished = wait(&mut child, &rx, &mut deliver, cancel, intent.deadline())?;

        // The reader threads end when their pipes close, which the exit or the kill above has
        // already caused. Draining after the join is what delivers the last progress lines.
        let _ = lines.join();
        let parsed = reader.join().unwrap_or_default();
        while let Ok(line) = rx.try_recv() {
            deliver(&line);
        }
        let wrote = writer.map(|handle| {
            handle
                .join()
                .unwrap_or_else(|_| Err(std::io::Error::other("stdin writer panicked")))
        });

        match finished.stop {
            Stop::Cancelled => Err(GitError::Cancelled),
            Stop::Deadline(after_ms) => Err(GitError::Budget { after_ms }),
            Stop::Exited(status) if status.success() => {
                if let Some(Err(e)) = wrote {
                    return Err(GitError::Internal {
                        detail: format!("stdin could not be written: {e}"),
                    });
                }
                Ok(RunOutput { stdout: parsed })
            }
            Stop::Exited(status) => Err(refused.map_or_else(
                || GitError::Internal {
                    detail: format!(
                        "git exited with {} after {} ms",
                        status.code().unwrap_or(-1),
                        finished.elapsed_ms
                    ),
                },
                |protocol| GitError::TransportRefused { protocol },
            )),
        }
    }
}

/// Watch the child until it exits, `cancel` fires, or `deadline` elapses — delivering stderr lines
/// as they arrive. The cancel and the deadline kill the **group** and wait for it.
fn wait(
    child: &mut command_group::GroupChild,
    rx: &mpsc::Receiver<String>,
    deliver: &mut dyn FnMut(&str),
    cancel: &CancelToken,
    deadline: Option<Duration>,
) -> GitResult<Finished> {
    let started = Instant::now();
    let mut poll = Duration::from_micros(250);
    let stop = loop {
        while let Ok(line) = rx.try_recv() {
            deliver(&line);
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
        if let Some(limit) = deadline {
            let elapsed = started.elapsed();
            if elapsed >= limit {
                // The group, not the child: a transport helper outliving git would keep the
                // connection open and the process tree alive past the answer.
                let _ = child.kill();
                let _ = child.wait();
                break Stop::Deadline(u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX));
            }
        }
        thread::sleep(poll);
        poll = (poll * 2).min(Duration::from_millis(10));
    };
    Ok(Finished {
        stop,
        elapsed_ms: started.elapsed().as_millis(),
    })
}
