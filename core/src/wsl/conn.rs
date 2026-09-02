//! §13 — one worker per active distro, kept alive.
//!
//! Requests are single-flighted under a mutex: the core writes one small frame and then reads
//! until the reply carrying that id arrives. This ordering avoids the deadlock caused by filling
//! a child's input while its output is left undrained, without adding a protocol reader thread.

use crate::proto::frame::{read_frame, FrameError};
use crate::proto::wire::RequestId;
use crate::wsl::deploy::{
    chmod_argv, cleanup_argvs, deploy_paths, home_argv, list_root_argv, mkdir_argv, parse_size,
    plan_deploy, size_argv, worker_fingerprint, write_argv,
};
use crate::wsl::distros::{launch_argv, WslCli};
use crate::wsl::proto::{
    write_worker_call, WorkerCall, WorkerEvent, WorkerFault, WorkerGit, WorkerOutbound,
    WorkerRequest, WORKER_PROTOCOL_VERSION,
};
use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::io::Write;
use std::process::{Child, Output};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Debug)]
pub enum WslError {
    Launch { distro: String, detail: String },
    Deploy { distro: String, detail: String },
    Protocol { detail: String },
    VersionMismatch { worker: u32, core: u32 },
    Fault(WorkerFault),
    Closed,
}

impl WslError {
    /// True when the distro could not be reached at all.
    #[must_use]
    pub fn is_unavailable(&self) -> bool {
        matches!(
            self,
            Self::Launch { .. } | Self::Deploy { .. } | Self::Closed
        )
    }
}

impl std::fmt::Display for WslError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Launch { distro, detail } => {
                write!(f, "cannot start a worker in {distro}: {detail}")
            }
            Self::Deploy { distro, detail } => {
                write!(f, "cannot install the worker in {distro}: {detail}")
            }
            Self::Protocol { detail } => write!(f, "worker protocol error: {detail}"),
            Self::VersionMismatch { worker, core } => {
                write!(
                    f,
                    "worker speaks protocol {worker}, this build speaks {core}"
                )
            }
            Self::Fault(fault) => write!(f, "worker fault: {fault:?}"),
            Self::Closed => write!(f, "the worker connection is closed"),
        }
    }
}

impl std::error::Error for WslError {}

pub struct WorkerIo {
    pub reader: Box<dyn std::io::Read + Send>,
    pub writer: Box<dyn std::io::Write + Send>,
    pub stop: Box<dyn Fn() + Send + Sync>,
}

impl std::fmt::Debug for WorkerIo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkerIo").finish_non_exhaustive()
    }
}

pub trait WorkerLauncher: Send + Sync + std::fmt::Debug {
    fn launch(&self, distro: &str) -> Result<WorkerIo, WslError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerHello {
    pub protocol_version: u32,
    pub worker_version: String,
    pub git: WorkerGit,
    pub pid: u32,
}

#[derive(Debug)]
pub struct WslWorker {
    distro: String,
    hello: WorkerHello,
    io: Mutex<WorkerIo>,
    next_id: AtomicU64,
}

impl WslWorker {
    /// Reads `Hello` before returning so an incompatible deployed worker fails at connection.
    pub fn connect(distro: &str, mut io: WorkerIo) -> Result<WslWorker, WslError> {
        let hello = match read_hello(&mut io) {
            Ok(hello) => hello,
            Err(err) => {
                (io.stop)();
                return Err(err);
            }
        };
        Ok(WslWorker {
            distro: distro.to_owned(),
            hello,
            io: Mutex::new(io),
            next_id: AtomicU64::new(1),
        })
    }

    #[must_use]
    pub fn distro(&self) -> &str {
        &self.distro
    }

    #[must_use]
    pub fn hello(&self) -> &WorkerHello {
        &self.hello
    }

    #[must_use]
    pub fn git(&self) -> &WorkerGit {
        &self.hello.git
    }

    pub fn call(&self, request: &WorkerRequest) -> Result<serde_json::Value, WslError> {
        self.stream(request, &mut |_| {})
    }

    pub fn stream(
        &self,
        request: &WorkerRequest,
        on_event: &mut dyn FnMut(WorkerEvent),
    ) -> Result<serde_json::Value, WslError> {
        let id = RequestId(self.next_id.fetch_add(1, Ordering::Relaxed));
        let mut io = self.io.lock().map_err(|_| WslError::Closed)?;
        write_worker_call(
            &mut io.writer,
            &WorkerCall {
                id,
                request: request.clone(),
            },
        )
        .map_err(|err| WslError::Protocol {
            detail: err.to_string(),
        })?;

        let mut buffer = Vec::new();
        loop {
            match read_one(&mut io, &mut buffer)? {
                WorkerOutbound::Event {
                    id: received,
                    event,
                } if received == id => {
                    on_event(event);
                }
                WorkerOutbound::Reply { id: received, ok } if received == id => return Ok(ok),
                WorkerOutbound::Fail {
                    id: received,
                    fault,
                } if received == id => return Err(WslError::Fault(fault)),
                other => {
                    return Err(WslError::Protocol {
                        detail: format!("frame did not match request {id:?}: {other:?}"),
                    });
                }
            }
        }
    }

    pub fn shutdown(&self) {
        let _ = self.call(&WorkerRequest::Shutdown);
        let io = match self.io.lock() {
            Ok(io) => io,
            Err(poisoned) => poisoned.into_inner(),
        };
        (io.stop)();
    }
}

impl Drop for WslWorker {
    fn drop(&mut self) {
        let io = match self.io.get_mut() {
            Ok(io) => io,
            Err(poisoned) => poisoned.into_inner(),
        };
        (io.stop)();
    }
}

fn read_hello(io: &mut WorkerIo) -> Result<WorkerHello, WslError> {
    let mut buffer = Vec::new();
    let frame = read_one(io, &mut buffer)?;
    let WorkerOutbound::Hello {
        protocol_version,
        worker_version,
        git,
        pid,
    } = frame
    else {
        return Err(WslError::Protocol {
            detail: "the first worker frame was not hello".to_owned(),
        });
    };
    if protocol_version != WORKER_PROTOCOL_VERSION {
        return Err(WslError::VersionMismatch {
            worker: protocol_version,
            core: WORKER_PROTOCOL_VERSION,
        });
    }
    Ok(WorkerHello {
        protocol_version,
        worker_version,
        git,
        pid,
    })
}

fn read_one(io: &mut WorkerIo, buffer: &mut Vec<u8>) -> Result<WorkerOutbound, WslError> {
    match read_frame(&mut io.reader, buffer) {
        Ok(()) => serde_json::from_slice(buffer).map_err(|err| WslError::Protocol {
            detail: err.to_string(),
        }),
        Err(FrameError::Eof) => Err(WslError::Closed),
        Err(other) => Err(WslError::Protocol {
            detail: other.to_string(),
        }),
    }
}

/// One worker per distro, created on first use and kept until shutdown.
#[derive(Debug)]
pub struct WslWorkerPool {
    launcher: Arc<dyn WorkerLauncher>,
    live: Mutex<BTreeMap<String, Arc<WslWorker>>>,
}

impl WslWorkerPool {
    #[must_use]
    pub fn new(launcher: Arc<dyn WorkerLauncher>) -> WslWorkerPool {
        WslWorkerPool {
            launcher,
            live: Mutex::new(BTreeMap::new()),
        }
    }

    pub fn get(&self, distro: &str) -> Result<Arc<WslWorker>, WslError> {
        {
            let live = self.live.lock().map_err(|_| WslError::Closed)?;
            if let Some(worker) = live.get(distro) {
                return Ok(Arc::clone(worker));
            }
        }

        // A cold distro can take seconds to start, so launching it must not hold the map lock
        // needed by already-running distros.
        let worker = Arc::new(WslWorker::connect(distro, self.launcher.launch(distro)?)?);
        let mut live = self.live.lock().map_err(|_| WslError::Closed)?;
        if let Some(existing) = live.get(distro) {
            let existing = Arc::clone(existing);
            drop(live);
            worker.shutdown();
            return Ok(existing);
        }
        live.insert(distro.to_owned(), Arc::clone(&worker));
        Ok(worker)
    }

    #[must_use]
    pub fn live_distros(&self) -> Vec<String> {
        match self.live.lock() {
            Ok(live) => live.keys().cloned().collect(),
            Err(_) => Vec::new(),
        }
    }

    pub fn shutdown_all(&self) {
        let mut live = match self.live.lock() {
            Ok(live) => live,
            Err(poisoned) => poisoned.into_inner(),
        };
        let workers = std::mem::take(&mut *live);
        drop(live);
        for worker in workers.values() {
            worker.shutdown();
        }
    }
}

/// Installs this build's worker inside a distro and launches it without a shell.
pub struct WslExeLauncher {
    cli: Arc<dyn WslCli>,
    worker_bytes: Arc<Vec<u8>>,
    user: Option<String>,
}

impl std::fmt::Debug for WslExeLauncher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WslExeLauncher")
            .field("cli", &self.cli)
            .field("worker_bytes_len", &self.worker_bytes.len())
            .field("user_configured", &self.user.is_some())
            .finish()
    }
}

impl WslExeLauncher {
    #[must_use]
    pub fn new(
        cli: Arc<dyn WslCli>,
        worker_bytes: Arc<Vec<u8>>,
        user: Option<String>,
    ) -> WslExeLauncher {
        WslExeLauncher {
            cli,
            worker_bytes,
            user,
        }
    }

    fn deploy_command(
        &self,
        distro: &str,
        command_parts: &[String],
    ) -> Result<Vec<OsString>, WslError> {
        let Some((executable, arguments)) = command_parts.split_first() else {
            return Err(deploy_error(distro, "empty deployment command"));
        };
        let argument_refs: Vec<&str> = arguments.iter().map(String::as_str).collect();
        Ok(launch_argv(
            distro,
            self.user.as_deref(),
            executable,
            &argument_refs,
        ))
    }

    fn run_output(&self, distro: &str, argv: &[String]) -> Result<Vec<u8>, WslError> {
        let command = self.deploy_command(distro, argv)?;
        let borrowed: Vec<&OsStr> = command.iter().map(OsString::as_os_str).collect();
        let output = self
            .cli
            .output(&borrowed)
            .map_err(|err| deploy_error(distro, err.to_string()))?;
        checked_deploy_output(distro, argv, output)
    }

    fn run_with_input(
        &self,
        distro: &str,
        argv: &[String],
        input: &[u8],
    ) -> Result<Vec<u8>, WslError> {
        let command = self.deploy_command(distro, argv)?;
        let borrowed: Vec<&OsStr> = command.iter().map(OsString::as_os_str).collect();
        let mut child = self
            .cli
            .spawn_piped(&borrowed)
            .map_err(|err| deploy_error(distro, err.to_string()))?;
        let Some(mut sink) = child.stdin.take() else {
            stop_child(&mut child);
            return Err(deploy_error(distro, "deployment command had no stdin"));
        };
        // `tee` copies its stdin straight back to stdout. Writing the whole binary while
        // nothing drains that pipe fills it: `tee` blocks on the write, stops reading stdin,
        // and both sides wait forever. Measured — deploying a 2.5 MB worker hung indefinitely
        // until the two output pipes were drained on their own threads.
        let mut stdout = child.stdout.take();
        let mut stderr = child.stderr.take();
        let out_drain = std::thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(pipe) = stdout.as_mut() {
                let _ = std::io::Read::read_to_end(pipe, &mut buf);
            }
            buf
        });
        let err_drain = std::thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(pipe) = stderr.as_mut() {
                let _ = std::io::Read::read_to_end(pipe, &mut buf);
            }
            buf
        });

        let write_result = sink.write_all(input).and_then(|()| sink.flush());
        // Closing stdin is what tells the child the stream ended.
        drop(sink);
        if let Err(err) = write_result {
            stop_child(&mut child);
            return Err(deploy_error(distro, err.to_string()));
        }
        let status = child
            .wait()
            .map_err(|err| deploy_error(distro, err.to_string()))?;
        let output = Output {
            status,
            stdout: out_drain.join().unwrap_or_default(),
            stderr: err_drain.join().unwrap_or_default(),
        };
        checked_deploy_output(distro, argv, output)
    }

    fn install(&self, distro: &str) -> Result<String, WslError> {
        let fingerprint = worker_fingerprint(self.worker_bytes.as_slice());
        let home = String::from_utf8(self.run_output(distro, &home_argv())?)
            .map_err(|err| deploy_error(distro, err.to_string()))?;
        let home = home.trim();
        let paths = deploy_paths(home, &fingerprint).ok_or_else(|| {
            deploy_error(
                distro,
                format!("cannot build a worker install path under {home:?}"),
            )
        })?;

        let siblings = match self.run_output(distro, &list_root_argv(&paths.root)) {
            Ok(output) => sibling_names(&output),
            Err(_) => Vec::new(),
        };
        // Presence is the *whole executable*, not its directory and not merely a file at that
        // path. An install interrupted between the `mkdir` and the copy leaves an empty
        // directory; one interrupted during the copy leaves a short file. Either read as
        // "deployed" launches something that cannot run, and the symptom — a pipe that closes
        // with no frame — names nothing. The directory is content-addressed, so a copy of the
        // right length at that path is the right build.
        let want = u64::try_from(self.worker_bytes.len()).unwrap_or(u64::MAX);
        let exe_present = self
            .run_output(distro, &size_argv(&paths.exe))
            .ok()
            .and_then(|output| parse_size(&output))
            == Some(want);
        let plan = plan_deploy(&paths, exe_present, &siblings);
        if plan.install {
            self.run_output(distro, &mkdir_argv(&paths.version_dir))?;
            self.run_with_input(
                distro,
                &write_argv(&paths.exe),
                self.worker_bytes.as_slice(),
            )?;
        }
        // Always, not only after a copy: a deploy killed between the copy and the mode change
        // leaves a complete file the size check accepts and `--exec` still refuses. One extra
        // call per worker start, against the cost of starting a virtual machine.
        self.run_output(distro, &chmod_argv(&paths.exe))?;
        for stale_dir in &plan.stale_dirs {
            for argv in cleanup_argvs(stale_dir) {
                // Refusing to remove a non-empty stale directory is the intended safety bound.
                let _ = self.run_output(distro, &argv);
            }
        }
        Ok(plan.exe)
    }
}

impl WorkerLauncher for WslExeLauncher {
    fn launch(&self, distro: &str) -> Result<WorkerIo, WslError> {
        let exe = self.install(distro)?;
        let argv = launch_argv(distro, self.user.as_deref(), &exe, &[]);
        let borrowed: Vec<&OsStr> = argv.iter().map(OsString::as_os_str).collect();
        let mut child = self
            .cli
            .spawn_piped(&borrowed)
            .map_err(|err| WslError::Launch {
                distro: distro.to_owned(),
                detail: err.to_string(),
            })?;
        let Some(writer) = child.stdin.take() else {
            stop_child(&mut child);
            return Err(missing_pipe_error(distro));
        };
        let Some(reader) = child.stdout.take() else {
            drop(writer);
            stop_child(&mut child);
            return Err(missing_pipe_error(distro));
        };

        let child = Arc::new(Mutex::new(Some(child)));
        let stop_handle = Arc::clone(&child);
        Ok(WorkerIo {
            reader: Box::new(reader),
            writer: Box::new(writer),
            stop: Box::new(move || {
                let Ok(mut child) = stop_handle.lock() else {
                    return;
                };
                if let Some(mut child) = child.take() {
                    stop_child(&mut child);
                }
            }),
        })
    }
}

fn deploy_error(distro: &str, detail: impl Into<String>) -> WslError {
    WslError::Deploy {
        distro: distro.to_owned(),
        detail: detail.into(),
    }
}

fn missing_pipe_error(distro: &str) -> WslError {
    WslError::Launch {
        distro: distro.to_owned(),
        detail: "wsl.exe did not provide stdin and stdout".to_owned(),
    }
}

fn checked_deploy_output(
    distro: &str,
    argv: &[String],
    output: Output,
) -> Result<Vec<u8>, WslError> {
    if output.status.success() {
        return Ok(output.stdout);
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stderr = stderr.trim();
    let detail = if stderr.is_empty() {
        format!("deployment command {argv:?} failed with {}", output.status)
    } else {
        format!(
            "deployment command {argv:?} failed with {}: {stderr}",
            output.status
        )
    };
    Err(deploy_error(distro, detail))
}

fn sibling_names(output: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(output)
        .lines()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .collect()
}

fn stop_child(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(all(test, feature = "testkit"))]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use super::{WorkerIo, WorkerLauncher, WslError, WslWorkerPool};
    use crate::testing::wsl::LoopbackLauncher;
    use crate::wsl::mounts::MountTable;
    use crate::wsl::proto::{WalkRequest, WorkerEvent, WorkerGit, WorkerRequest};
    use crate::wsl::serve::WorkerContext;
    use std::sync::Arc;

    const MOUNTINFO: &str = "28 1 8:32 / / rw - ext4 /dev/sdc rw\n";

    fn launcher() -> Arc<LoopbackLauncher> {
        Arc::new(LoopbackLauncher::new(|distro: &str| WorkerContext {
            distro: distro.to_owned(),
            git: Box::new(crate::testing::FakeGitBackend::new()),
            mounts: MountTable::from_mountinfo(MOUNTINFO),
            presence: WorkerGit::Present {
                version: "2.43.0".to_owned(),
            },
        }))
    }

    #[test]
    fn connecting_reads_hello_before_the_first_request() {
        let pool = WslWorkerPool::new(launcher());
        let worker = pool.get("alpha").expect("connects");
        assert_eq!(worker.distro(), "alpha");
        assert_eq!(
            worker.hello().protocol_version,
            crate::wsl::proto::WORKER_PROTOCOL_VERSION
        );
        assert!(matches!(worker.git(), WorkerGit::Present { .. }));
        pool.shutdown_all();
    }

    #[test]
    fn a_distro_is_launched_once_and_the_worker_is_kept_alive() {
        let launcher = launcher();
        let pool = WslWorkerPool::new(launcher.clone());
        for _ in 0..5 {
            let worker = pool.get("alpha").expect("connects");
            worker.call(&WorkerRequest::Ping).expect("pings");
        }
        assert_eq!(launcher.launched(), vec!["alpha".to_owned()]);
        assert_eq!(pool.live_distros(), vec!["alpha".to_owned()]);
        pool.shutdown_all();
    }

    #[test]
    fn two_distros_get_two_workers() {
        let launcher = launcher();
        let pool = WslWorkerPool::new(launcher.clone());
        pool.get("alpha").expect("connects");
        pool.get("beta").expect("connects");
        assert_eq!(
            launcher.launched(),
            vec!["alpha".to_owned(), "beta".to_owned()]
        );
        assert_eq!(
            pool.live_distros(),
            vec!["alpha".to_owned(), "beta".to_owned()]
        );
        pool.shutdown_all();
        assert!(pool.live_distros().is_empty());
    }

    #[test]
    fn a_reply_is_matched_to_its_own_request() {
        let pool = WslWorkerPool::new(launcher());
        let worker = pool.get("alpha").expect("connects");
        let mounts = worker
            .call(&WorkerRequest::Mounts {
                path: "/home/me/widget".to_owned(),
            })
            .expect("mount facts");
        assert_eq!(mounts["store_key"], "wsl:alpha:/");
        let ping = worker.call(&WorkerRequest::Ping).expect("pings");
        assert_eq!(ping["pong"], true);
        pool.shutdown_all();
    }

    #[test]
    fn a_stream_delivers_events_before_the_reply() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join("p/.git")).expect("mkdir");
        let pool = WslWorkerPool::new(launcher());
        let worker = pool.get("alpha").expect("connects");
        let mut repos = 0_u32;
        let summary = worker
            .stream(
                &WorkerRequest::Walk(WalkRequest {
                    root: dir.path().to_string_lossy().into_owned(),
                    follow_links: false,
                    descend_into_repos: false,
                    bare_candidates: false,
                    skip_extra: Vec::new(),
                }),
                &mut |event| {
                    if matches!(event, WorkerEvent::Repo(_)) {
                        repos += 1;
                    }
                },
            )
            .expect("walks");
        assert_eq!(repos, 1);
        assert_eq!(summary["found_repos"], 1);
        pool.shutdown_all();
    }

    #[test]
    fn a_failed_launch_is_unavailable_and_leaves_no_entry_behind() {
        #[derive(Debug)]
        struct Refusing;

        impl WorkerLauncher for Refusing {
            fn launch(&self, distro: &str) -> Result<WorkerIo, WslError> {
                Err(WslError::Launch {
                    distro: distro.to_owned(),
                    detail: "the distro is not running".to_owned(),
                })
            }
        }

        let pool = WslWorkerPool::new(Arc::new(Refusing));
        let err = pool.get("alpha").expect_err("refuses");
        assert!(err.is_unavailable());
        assert!(pool.live_distros().is_empty());
    }
}
