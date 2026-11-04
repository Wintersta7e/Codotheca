#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! No git child outlives the core (§3.4).
//!
//! The real binary is spawned over a few hundred empty repositories, told to scan, and ended while
//! its discovery is still starting `git rev-parse` children — by closing stdin (the shell's orderly
//! quit) and, on Windows, by killing it outright; afterwards no process whose command line names
//! this test's corpus may remain. A second process is killed outright while it holds a running git
//! child, which must die with it. On Windows a child the core was still starting when it exited
//! was left suspended for ever, holding the handles it inherited; on Linux a child is reparented
//! and runs on after the core that asked for it is gone.
use std::io::Write as _;
use std::path::Path;
use std::time::{Duration, Instant};

use codotheca_core::proto::frame::{read_frame, write_frame};

/// Enough repositories that discovery is still spawning when the quit lands, on either platform.
const REPOS: usize = 300;
/// How long the core may take to exit once told to.
const EXIT_DEADLINE: Duration = Duration::from_secs(60);
/// How long a child may take to end once its parent is gone by force: a kill-on-close group
/// ends it at once; outside one, the helper's child sleeps for a minute.
#[cfg(windows)]
const SETTLE: Duration = Duration::from_secs(10);
/// How long discovery may take to report its first repository.
const BURST_DEADLINE: Duration = Duration::from_secs(120);

struct Core {
    child: std::process::Child,
    next_id: u64,
}

impl Core {
    fn spawn(data: &Path) -> Self {
        let log = std::fs::File::create(data.join("core-stderr.txt")).expect("stderr file");
        let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_codotheca-core"))
            .arg(format!("--data-dir={}", data.display()))
            .arg("--epoch=11")
            .arg(format!("--parent-pid={}", std::process::id()))
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            // A file, not a pipe: a child that inherited the core's handles would hold a pipe
            // open and turn a read of it into a hang instead of an answer.
            .stderr(log)
            .spawn()
            .expect("core spawns");
        let hello = read_json(&mut child);
        assert_eq!(hello["t"], "hello", "{hello}");
        let mut core = Self { child, next_id: 1 };
        core.request("app.hello_ack", &serde_json::json!({}));
        core
    }

    fn request(&mut self, command: &str, args: &serde_json::Value) -> serde_json::Value {
        let id = self.next_id;
        self.next_id += 1;
        let frame = serde_json::json!({"t": "request", "id": id, "command": command, "args": args});
        let body = serde_json::to_vec(&frame).expect("frame");
        {
            let stdin = self.child.stdin.as_mut().expect("stdin");
            write_frame(stdin, &body).expect("write");
            stdin.flush().expect("flush");
        }
        loop {
            let reply = read_json(&mut self.child);
            if reply["id"] == serde_json::json!(id) {
                assert_eq!(reply["t"], "response", "{command} was refused: {reply}");
                return reply["ok"].clone();
            }
        }
    }

    /// Add `root`, start a full scan, and return once a git child naming `marker` has been seen
    /// alive while discovery is still running — that is, while it is starting children. Seeing
    /// one is also what proves [`survivors`] can see one: a listing that never matches would
    /// pass every assertion below.
    fn scan_until_mid_burst(&mut self, root: &Path, marker: &str) -> serde_json::Value {
        let added = self.request(
            "roots.add",
            &serde_json::json!({
                "pathBytes": {"b64": base64_of(root.to_str().expect("UTF-8 root").as_bytes())},
                "confirmLarge": true,
                "distro": "",
            }),
        );
        assert!(
            added["refusedBecause"].is_null() && !added["root"].is_null(),
            "the root was refused, so nothing below is about scanning: {added}"
        );
        self.request("scan.start", &serde_json::json!({"full": true}));
        let until = Instant::now() + BURST_DEADLINE;
        loop {
            let seen = survivors(marker);
            if !seen.is_empty() {
                let status = self.request("scan.status", &serde_json::json!({}));
                let found = status["foundRepos"].as_u64().unwrap_or(0);
                assert!(
                    status["running"] == serde_json::json!(true)
                        && found < u64::try_from(REPOS).unwrap(),
                    "discovery finished before the quit, so this run tests nothing: {status}"
                );
                eprintln!(
                    "mid-burst: {} live git child(ren), {status}\n{}",
                    seen.len(),
                    seen[0]
                );
                return status;
            }
            assert!(
                Instant::now() < until,
                "no git child naming the corpus was ever seen alive"
            );
        }
    }

    fn wait_exit(&mut self) {
        let until = Instant::now() + EXIT_DEADLINE;
        while self.child.try_wait().expect("try_wait").is_none() {
            assert!(
                Instant::now() < until,
                "the core did not exit within {EXIT_DEADLINE:?}"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

fn read_json(child: &mut std::process::Child) -> serde_json::Value {
    let stdout = child.stdout.as_mut().expect("stdout");
    let mut buf = Vec::new();
    read_frame(stdout, &mut buf).expect("the core is still speaking");
    serde_json::from_slice(&buf).expect("json")
}

fn base64_of(bytes: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// `REPOS` empty repositories under `root`: one `git init` with no template, copied.
fn corpus(root: &Path) {
    let seed = root.join("r0");
    let status = std::process::Command::new("git")
        .args(["init", "-q", "--template="])
        .arg(&seed)
        .status()
        .expect("git runs");
    assert!(status.success(), "git init failed");
    for n in 1..REPOS {
        copy_tree(&seed, &root.join(format!("r{n}")));
    }
}

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("mkdir");
    for entry in std::fs::read_dir(from).expect("read_dir") {
        let entry = entry.expect("entry");
        let target = to.join(entry.file_name());
        if entry.file_type().expect("file type").is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).expect("copy");
        }
    }
}

/// Every live process whose command line holds `marker`, as `pid  command line`.
#[cfg(unix)]
fn survivors(marker: &str) -> Vec<String> {
    let me = std::process::id().to_string();
    let mut found = Vec::new();
    for entry in std::fs::read_dir("/proc").expect("/proc") {
        let Ok(entry) = entry else { continue };
        let pid = entry.file_name().to_string_lossy().into_owned();
        if pid == me || !pid.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        let Ok(raw) = std::fs::read(entry.path().join("cmdline")) else {
            continue;
        };
        let line = String::from_utf8_lossy(&raw).replace('\0', " ");
        if line.contains(marker) {
            found.push(format!("{pid}  {line}"));
        }
    }
    found
}

/// Every live `git.exe` whose command line holds `marker`, as `pid parent  command line`.
#[cfg(windows)]
fn survivors(marker: &str) -> Vec<String> {
    let script = "Get-CimInstance Win32_Process -Filter \"Name='git.exe'\" | ForEach-Object { \
                  \"$($_.ProcessId) $($_.ParentProcessId)  $($_.CommandLine)\" }";
    let out = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .stdin(std::process::Stdio::null())
        .output()
        .expect("powershell runs");
    assert!(out.status.success(), "the process listing failed: {out:?}");
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|line| line.contains(marker))
        .map(str::to_owned)
        .collect()
}

/// A corpus directory whose name no other run's command line can carry.
fn marked_root(tmp: &Path) -> (std::path::PathBuf, String) {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let marker = format!("childexit{}x{nanos}", std::process::id());
    let root = tmp.join(&marker);
    std::fs::create_dir_all(&root).unwrap();
    corpus(&root);
    (root, marker)
}

#[test]
fn an_orderly_quit_mid_discovery_leaves_no_git_child_behind() {
    let tmp = tempfile::tempdir().expect("tmp");
    let (root, marker) = marked_root(tmp.path());
    let data = tempfile::tempdir().expect("tmp");
    let mut core = Core::spawn(data.path());
    let status = core.scan_until_mid_burst(&root, &marker);

    // The shell's quit: stdin closes, the core answers `StdinEof` and exits.
    drop(core.child.stdin.take());
    core.wait_exit();

    // Read the instant the core is gone: an orderly exit ends its children first, so there is
    // nothing to wait for — and a child still running now is one the core left behind.
    let left = survivors(&marker);
    assert!(
        left.is_empty(),
        "{} git child(ren) outlived an orderly quit at {status}:\n{}",
        left.len(),
        left.join("\n")
    );
}

/// Set to a repository path, this test binary becomes the process the next test kills.
#[cfg(windows)]
const HELPER_ENV: &str = "CODOTHECA_TEST_HOLD_GIT_CHILD";
/// What the helper prints once its git child has been started and resumed.
#[cfg(windows)]
const HELPER_READY: &str = "git child running";

/// Not a check of its own: with [`HELPER_ENV`] set it runs, through the read seam, a git whose
/// alias sleeps for a minute — a child that depends on no pipe of its parent's, so only its
/// process group can end it early.
#[cfg(windows)]
#[test]
fn helper_holds_one_git_child_open() {
    let Ok(repo_path) = std::env::var(HELPER_ENV) else {
        return;
    };
    let repo_path = std::path::PathBuf::from(repo_path);
    let repo = codotheca_core::git::RepoHandle::resolve(
        &repo_path,
        codotheca_core::git::StoreKey::new("s"),
        codotheca_core::mount::StoreClass::Local,
    )
    .unwrap();
    let hooks = codotheca_core::git::ensure_empty_hooks_dir(repo_path.parent().unwrap()).unwrap();
    let _ = codotheca_core::git::GitExec::system(hooks).run_piped(
        &repo,
        &[
            std::ffi::OsStr::new("-c"),
            std::ffi::OsStr::new("alias.hold=!sleep 60"),
            std::ffi::OsStr::new("hold"),
        ],
        codotheca_core::git::RunLimits::none(),
        &codotheca_core::cancel::CancelToken::new(),
        |_stdin: &mut dyn std::io::Write| {
            // The writer runs only once the spawn has returned: the child is in its group and
            // running, which is the state a kill-on-close group must end.
            eprintln!("{HELPER_READY}");
            Ok(())
        },
        |stdout: &mut dyn std::io::BufRead| {
            let mut sink = Vec::new();
            std::io::Read::read_to_end(stdout, &mut sink)
        },
    );
}

/// A process killed outright takes its running git children with it: the Job Object closes with
/// the parent's last handle and kills every process in it. Windows only — Linux has no
/// kill-on-close group without `unsafe`, and a reparented child there runs to its own end. A child
/// still being created when its parent dies is out of reach of either; an orderly quit never
/// leaves one there (the test above).
#[cfg(windows)]
#[test]
fn a_killed_process_takes_its_running_git_child_with_it() {
    let tmp = tempfile::tempdir().expect("tmp");
    let (root, marker) = marked_root(tmp.path());
    let mut helper = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "helper_holds_one_git_child_open", "--nocapture"])
        .env(HELPER_ENV, root.join("r0"))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("helper spawns");
    let stderr = helper.stderr.take().unwrap();
    let mut lines = std::io::BufRead::lines(std::io::BufReader::new(stderr));
    assert!(
        lines.any(|line| line.is_ok_and(|l| l.contains(HELPER_READY))),
        "the helper never started its git child"
    );
    let running = survivors(&marker);
    // One or two: git for Windows' launcher runs the real `git.exe` as its own child.
    assert!(!running.is_empty(), "the git child is not visible");

    helper.kill().expect("kill");
    helper.wait().expect("wait");

    let until = Instant::now() + SETTLE;
    let left = loop {
        let left = survivors(&marker);
        if left.is_empty() || Instant::now() >= until {
            break left;
        }
        std::thread::sleep(Duration::from_millis(200));
    };
    assert!(
        left.is_empty(),
        "the git child was still alive {SETTLE:?} after its parent was killed:\n{}",
        left.join("\n")
    );
}

/// A core killed outright in the middle of a spawn burst leaves nothing behind. Some child is
/// always between its creation and its own Job Object at that moment; it is ended only because
/// the core itself sits in a kill-on-close job from startup, which every child joins as it is
/// created. Windows only: Linux has no kill-on-close group without `unsafe`, and a reparented
/// child there runs its one `rev-parse` to its own end.
#[cfg(windows)]
#[test]
fn a_core_killed_mid_spawn_burst_leaves_no_git_child_behind() {
    let tmp = tempfile::tempdir().expect("tmp");
    let (root, marker) = marked_root(tmp.path());
    let data = tempfile::tempdir().expect("tmp");
    let mut core = Core::spawn(data.path());
    let status = core.scan_until_mid_burst(&root, &marker);

    core.child.kill().expect("kill");
    core.wait_exit();

    let until = Instant::now() + SETTLE;
    let left = loop {
        let left = survivors(&marker);
        if left.is_empty() || Instant::now() >= until {
            break left;
        }
        std::thread::sleep(Duration::from_millis(200));
    };
    assert!(
        left.is_empty(),
        "{} git child(ren) were still alive {SETTLE:?} after the core was killed at {status}:\n{}",
        left.len(),
        left.join("\n")
    );
}
