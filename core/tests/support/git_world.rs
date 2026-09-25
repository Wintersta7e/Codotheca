//! §47.9's git fixture world: the hostile environment and the re-exec runner that carries it.
//!
//! **The hostile variables are set on a CHILD process, never on this one.** `std::env::set_var`
//! in a test process races every other test running in parallel threads of the same binary —
//! `core/src/bin/recording_git.rs` records the same trap — so a test that needs a hostile parent
//! re-executes its own binary with the variables set on that child alone, and the child runs the
//! one named test.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// Set on the re-executed child, so the named test knows it is the half that runs the product.
pub(crate) const CHILD_MARKER: &str = "CODOTHECA_GIT_WORLD_CHILD";

/// The directory the parent prepared, handed to the child.
pub(crate) const CHILD_DIR: &str = "CODOTHECA_GIT_WORLD_DIR";

/// Is this process the re-executed child?
pub(crate) fn is_child() -> bool {
    std::env::var_os(CHILD_MARKER).is_some()
}

/// The directory the parent handed down.
pub(crate) fn child_dir() -> PathBuf {
    PathBuf::from(std::env::var_os(CHILD_DIR).expect("the parent sets the child's directory"))
}

/// Re-execute this test binary to run exactly `test` with `env` set on the child only.
///
/// **A child that ran nothing passes**, and that is the failure this checks for: `--exact` with a
/// name that matches no test runs zero tests and exits 0. So the child's summary line must report
/// one test passed, or this panics. The child's output is relayed so its counts are visible.
pub(crate) fn run_in_child(test: &str, dir: &Path, env: &[(String, OsString)]) -> Output {
    let mut cmd = Command::new(std::env::current_exe().expect("the test binary"));
    cmd.args([test, "--exact", "--nocapture", "--test-threads=1"]);
    cmd.env(CHILD_MARKER, "1").env(CHILD_DIR, dir);
    for (key, value) in env {
        cmd.env(key, value);
    }
    let out = cmd.output().expect("the child runs");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    eprintln!("--- child `{test}` stdout ---\n{stdout}--- child stderr ---\n{stderr}");
    assert!(
        out.status.success(),
        "the child running `{test}` failed ({:?})",
        out.status
    );
    assert!(
        stdout.contains("test result: ok. 1 passed"),
        "the child ran no test named `{test}`, so nothing was asserted"
    );
    out
}

/// What `codotheca-recording-git` wrote: the argv it was given and the environment it ran in.
#[derive(Debug, Clone)]
pub(crate) struct Recording {
    /// Every argv element after argv\[0\], lossily decoded.
    pub(crate) argv: Vec<String>,
    /// Every environment entry, `KEY=VALUE`.
    pub(crate) env: Vec<String>,
}

/// Read the stand-in's recording for a child keyed on `key` — the absolute last argv element,
/// or the `-C` directory — which it writes beside the key as `<name>.recorded`.
pub(crate) fn read_recording(key: &Path) -> Recording {
    let mut path = key.to_path_buf();
    let mut name = path
        .file_name()
        .expect("the key has a file name")
        .to_os_string();
    name.push(".recorded");
    path.set_file_name(name);
    let blob = std::fs::read(&path).unwrap_or_else(|e| {
        panic!(
            "no recording at {}: the child never spawned the stand-in ({e})",
            path.display()
        )
    });
    let mut argv = Vec::new();
    let mut env = Vec::new();
    for record in blob.split(|b| *b == 0) {
        let text = String::from_utf8_lossy(record);
        if let Some(rest) = text.strip_prefix("ARGV\t") {
            argv.push(rest.to_owned());
        } else if let Some(rest) = text.strip_prefix("ENV\t") {
            env.push(rest.to_owned());
        }
    }
    assert!(
        !argv.is_empty() && !env.is_empty(),
        "the recording holds no argv or no environment, so every assertion over it is vacuous"
    );
    Recording { argv, env }
}

/// The recording stand-in for git, declared as a `testkit` binary.
#[cfg(feature = "testkit")]
pub(crate) fn recording_git() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_codotheca-recording-git"))
}

/// A hostile parent environment: every §47.3 scrub variable at a destructive value, and the one
/// config route §47.3 keeps.
#[derive(Debug, Clone)]
pub(crate) struct HostileEnv {
    /// Variables `neutralise_env` must remove from every git child.
    pub(crate) scrubbed: Vec<(String, OsString)>,
    /// Variables it must leave alone — the user's own config file is config (§47.3's *Kept*).
    pub(crate) kept: Vec<(String, OsString)>,
}

impl HostileEnv {
    /// Every variable, scrubbed and kept, for `run_in_child`.
    pub(crate) fn all(&self) -> Vec<(String, OsString)> {
        self.scrubbed.iter().chain(&self.kept).cloned().collect()
    }
}

/// §47.9 C's hostile parent, rooted at `root`.
///
/// `GIT_CONFIG_KEY_7` sits **past** `GIT_CONFIG_COUNT`: a scrub that walked `0..COUNT` would
/// leave it, and only an enumeration of the parent's environment finds it. The `GIT_TRACE`
/// family carries three spellings for the same reason.
pub(crate) fn hostile_parent_env(root: &Path) -> HostileEnv {
    let marker_dir = root.join("exec-path-marker");
    std::fs::create_dir_all(&marker_dir).expect("marker directory");
    let global = root.join("hostile-global.gitconfig");
    std::fs::write(&global, "[fetch]\n\tprune = true\n").expect("global config");
    let os = |value: &str| OsString::from(value);
    let path = |p: PathBuf| p.into_os_string();
    HostileEnv {
        scrubbed: vec![
            ("GIT_CONFIG_COUNT".to_owned(), os("1")),
            ("GIT_CONFIG_KEY_0".to_owned(), os("fetch.prune")),
            ("GIT_CONFIG_VALUE_0".to_owned(), os("true")),
            ("GIT_CONFIG_KEY_7".to_owned(), os("remote.origin.prune")),
            ("GIT_CONFIG_VALUE_7".to_owned(), os("true")),
            (
                "GIT_CONFIG_PARAMETERS".to_owned(),
                os("'fetch.prunetags'='true'"),
            ),
            (
                "GIT_ALLOW_PROTOCOL".to_owned(),
                os("file:ext:codotheca-hostile-helper"),
            ),
            ("GIT_PROTOCOL_FROM_USER".to_owned(), os("1")),
            ("GIT_EXEC_PATH".to_owned(), path(marker_dir)),
            ("GIT_TRACE".to_owned(), path(root.join("trace.txt"))),
            (
                "GIT_TRACE_PACKET".to_owned(),
                path(root.join("trace-packet.txt")),
            ),
            (
                "GIT_TRACE2_EVENT".to_owned(),
                path(root.join("trace2-event.txt")),
            ),
            ("GIT_GRAFT_FILE".to_owned(), path(root.join("grafts"))),
            (
                "GIT_REPLACE_REF_BASE".to_owned(),
                os("refs/hostile-replace/"),
            ),
            ("GIT_SHALLOW_FILE".to_owned(), path(root.join("shallow"))),
        ],
        kept: vec![("GIT_CONFIG_GLOBAL".to_owned(), path(global))],
    }
}

/// What reached one child, by variable name, against a hostile environment.
#[derive(Debug, Default)]
pub(crate) struct ScrubReport {
    /// Scrubbed variables that reached the child — each one a failure.
    pub(crate) leaked: Vec<String>,
    /// Kept variables that did **not** reach it — each one a failure too.
    pub(crate) lost: Vec<String>,
    /// How many were planted, printed so a run that planted nothing is visible.
    pub(crate) planted: usize,
}

/// Compare a child's recorded environment (`KEY=VALUE` entries) against `hostile`.
pub(crate) fn scrub_report(hostile: &HostileEnv, child_env: &[String]) -> ScrubReport {
    let reached = |key: &str| {
        child_env.iter().any(|entry| {
            entry
                .split_once('=')
                .is_some_and(|(k, _)| k.eq_ignore_ascii_case(key))
        })
    };
    let mut report = ScrubReport {
        planted: hostile.scrubbed.len() + hostile.kept.len(),
        ..ScrubReport::default()
    };
    for (key, _) in &hostile.scrubbed {
        if reached(key) {
            report.leaked.push(key.clone());
        }
    }
    for (key, _) in &hostile.kept {
        if !reached(key) {
            report.lost.push(key.clone());
        }
    }
    report
}
