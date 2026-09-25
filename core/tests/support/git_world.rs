//! §47.9's git fixture world: the hostile environment and the re-exec runner that carries it.
//!
//! **The hostile variables are set on a CHILD process, never on this one.** `std::env::set_var`
//! in a test process races every other test running in parallel threads of the same binary —
//! `core/src/bin/recording_git.rs` records the same trap — so a test that needs a hostile parent
//! re-executes its own binary with the variables set on that child alone, and the child runs the
//! one named test.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fmt::Write as _;
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

/// What `codotheca-recording-git` wrote for one invocation: its argv, environment and stdin.
#[derive(Debug, Clone)]
pub(crate) struct Recording {
    /// Every argv element after argv\[0\], lossily decoded.
    pub(crate) argv: Vec<String>,
    /// Every environment entry, `KEY=VALUE`.
    pub(crate) env: Vec<String>,
    /// The bytes it read on stdin; empty for `Stdio::null()`.
    pub(crate) stdin: Vec<u8>,
}

impl Recording {
    /// The value of `key` in this invocation's environment.
    pub(crate) fn env_value(&self, key: &str) -> Option<&str> {
        self.env.iter().find_map(|entry| {
            entry
                .split_once('=')
                .and_then(|(k, v)| (k == key).then_some(v))
        })
    }
}

/// Every invocation the stand-in recorded for `key` — the absolute last argv element, or the `-C`
/// directory — in the order they ran. It appends to `<name>.recorded` beside the key.
pub(crate) fn read_recordings(key: &Path) -> Vec<Recording> {
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
    let mut calls: Vec<Recording> = Vec::new();
    for record in blob.split(|b| *b == 0) {
        if record == b"CALL" {
            calls.push(Recording {
                argv: Vec::new(),
                env: Vec::new(),
                stdin: Vec::new(),
            });
            continue;
        }
        let Some(call) = calls.last_mut() else {
            continue;
        };
        if let Some(rest) = record.strip_prefix(b"ARGV\t") {
            call.argv.push(String::from_utf8_lossy(rest).into_owned());
        } else if let Some(rest) = record.strip_prefix(b"ENV\t") {
            call.env.push(String::from_utf8_lossy(rest).into_owned());
        } else if let Some(rest) = record.strip_prefix(b"STDIN\t") {
            call.stdin = rest.to_vec();
        }
    }
    assert!(
        !calls.is_empty()
            && calls
                .iter()
                .all(|c| !c.argv.is_empty() && !c.env.is_empty()),
        "the recording holds an invocation with no argv or no environment, so every assertion \
         over it is vacuous"
    );
    calls
}

/// The **last** invocation recorded for `key`.
pub(crate) fn read_recording(key: &Path) -> Recording {
    read_recordings(key)
        .pop()
        .expect("read_recordings asserts at least one")
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
///
/// **A leak is the planted VALUE arriving**, not the name: the write path sets its own
/// `GIT_ALLOW_PROTOCOL` after the scrub (§47.3), so the name is present by design and only the
/// parent's value reaching the child is the defect.
pub(crate) fn scrub_report(hostile: &HostileEnv, child_env: &[String]) -> ScrubReport {
    let arrived = |key: &str, value: &OsString| {
        let value = value.to_string_lossy();
        child_env.iter().any(|entry| {
            entry
                .split_once('=')
                .is_some_and(|(k, v)| k.eq_ignore_ascii_case(key) && v == value)
        })
    };
    let mut report = ScrubReport {
        planted: hostile.scrubbed.len() + hostile.kept.len(),
        ..ScrubReport::default()
    };
    for (key, value) in &hostile.scrubbed {
        if arrived(key, value) {
            report.leaked.push(key.clone());
        }
    }
    for (key, value) in &hostile.kept {
        if !arrived(key, value) {
            report.lost.push(key.clone());
        }
    }
    report
}

/// §45.6's hostile **read** profile: the config that lies to a naive read.
///
/// Each key, left alone, makes a read report less than is there — untracked files hidden, a dirty
/// submodule silenced, a stale untracked cache believed, a log that prints signatures or colour
/// into what a parser reads, a reflog never written, paths quoted into a form nobody unquotes.
pub(crate) const HOSTILE_READ_PROFILE: [(&str, &str); 9] = [
    ("status.showUntrackedFiles", "no"),
    ("core.untrackedCache", "true"),
    ("diff.ignoreSubmodules", "all"),
    ("submodule.sub.ignore", "all"),
    ("log.showSignature", "true"),
    ("format.pretty", "oneline"),
    ("core.logAllRefUpdates", "false"),
    ("color.ui", "always"),
    ("core.quotePath", "true"),
];

/// Deliver the hostile read profile through the repository's own config, and populate the
/// untracked cache it enables so a stale one exists to be believed.
pub(crate) fn apply_hostile_read_profile(repo: &super::TestRepo) {
    for (key, value) in HOSTILE_READ_PROFILE {
        repo.git(&["config", key, value]);
    }
    repo.git(&["update-index", "--untracked-cache"]);
    repo.git(&["status", "--porcelain"]);
}

/// The same profile as the user's global config, as the environment that delivers it
/// ([`user_global`]).
pub(crate) fn hostile_read_global(dir: &Path) -> Vec<(String, OsString)> {
    let mut text = String::new();
    for (key, value) in HOSTILE_READ_PROFILE {
        let (section, name) = key.rsplit_once('.').expect("a dotted key");
        let header = section.split_once('.').map_or_else(
            || format!("[{section}]"),
            |(outer, inner)| format!("[{outer} \"{inner}\"]"),
        );
        let _ = writeln!(text, "{header}\n\t{name} = {value}");
    }
    user_global(dir, &text)
}

/// The production read backend over `repo`, with its own empty hooks directory.
pub(crate) fn system_git(repo: &super::TestRepo) -> codotheca_core::git::SystemGit {
    codotheca_core::git::SystemGit::new(
        std::sync::Arc::new(repo.exec()),
        std::sync::Arc::new(codotheca_core::git::GitSlots::for_machine()),
        std::sync::Arc::new(codotheca_core::clock::SystemClock::new()),
    )
}

/// The environment that gives a product child `text` as the user's global config, on every git
/// the floor matrix runs: `GIT_CONFIG_GLOBAL` (read from git 2.32) **and** `HOME` holding the
/// same file as `.gitconfig`, which older releases read instead. `GIT_CONFIG_GLOBAL` alone left
/// a pre-2.32 child reading the developer's own file, and a test on it passing vacuously.
pub(crate) fn user_global(dir: &Path, text: &str) -> Vec<(String, OsString)> {
    let home = dir.join("user-home");
    std::fs::create_dir_all(&home).expect("user home");
    let file = home.join(".gitconfig");
    std::fs::write(&file, text).expect("user global");
    vec![
        ("GIT_CONFIG_GLOBAL".to_owned(), file.into_os_string()),
        ("HOME".to_owned(), home.clone().into_os_string()),
        ("XDG_CONFIG_HOME".to_owned(), home.into_os_string()),
    ]
}

/// The `git --version` line of the git the product runs under test.
pub(crate) fn test_git_version() -> String {
    let out = Command::new(super::test_git())
        .arg("--version")
        .output()
        .expect("git --version");
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

/// Whether the git under test reads config from `GIT_CONFIG_COUNT` (git 2.31). Measured, not
/// looked up: a release without it has no environment route for a scrub to close.
pub(crate) fn test_git_reads_config_env() -> bool {
    let dir = tempfile::tempdir().expect("tempdir");
    let out = Command::new(super::test_git())
        .current_dir(dir.path())
        .args(["config", "--get", "codotheca.probe"])
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_COUNT", "1")
        .env("GIT_CONFIG_KEY_0", "codotheca.probe")
        .env("GIT_CONFIG_VALUE_0", "yes")
        .output()
        .expect("git config");
    String::from_utf8_lossy(&out.stdout).trim() == "yes"
}

/// Whether the git under test lists `key` in `git help --config` — how a floor-matrix run tells
/// a hazard the release cannot have from a fixture that stopped reproducing one.
pub(crate) fn test_git_lists_key(key: &str) -> bool {
    let out = Command::new(super::test_git())
        .args(["help", "--config"])
        .output()
        .expect("git help --config");
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .any(|line| line.trim().eq_ignore_ascii_case(key))
}

// ---------------------------------------------------------------------------
// §47.9 C — D10's fixture world, the hostile write profile, the four routes and the comparator.
// ---------------------------------------------------------------------------

/// The four routes by which config reaches a git child (§47.9 C).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Route {
    /// The work repository's own `.git/config`.
    RepoConfig,
    /// A `GIT_CONFIG_GLOBAL` file — kept by the scrub, because the user's config is config.
    Global,
    /// An `include.path` target named from the repository's config.
    Include,
    /// `GIT_CONFIG_COUNT`, `GIT_CONFIG_KEY_n`, `GIT_CONFIG_VALUE_n` in the parent environment.
    Count,
}

impl Route {
    /// Every route, in the order a run reports them.
    pub(crate) const ALL: [Self; 4] = [Self::RepoConfig, Self::Global, Self::Include, Self::Count];

    /// The route's slug, used in directory names and printed lines.
    pub(crate) const fn slug(self) -> &'static str {
        match self {
            Self::RepoConfig => "repo-config",
            Self::Global => "global",
            Self::Include => "include",
            Self::Count => "count",
        }
    }
}

/// D10's fixture world, rooted in one directory: a work repository dressed with everything a
/// careless write could damage, a bare origin that has moved on, a bare decoy, a local bundle and
/// a creation-token list, the marker programs and the directory they write into.
#[derive(Debug)]
pub(crate) struct World {
    /// Everything lives under here.
    pub(crate) root: PathBuf,
    /// A clean `HOME` for fixture git.
    pub(crate) home: PathBuf,
    /// The work repository — the one every intent but a clone runs in.
    pub(crate) work: PathBuf,
    /// A bare origin with a diverged `main` and an extra branch.
    pub(crate) origin: PathBuf,
    /// A bare mirror of origin that `url.<decoy>.insteadOf` points reads at.
    pub(crate) decoy: PathBuf,
    /// Where every marker program writes; it must stay empty.
    pub(crate) markers: PathBuf,
    /// `PATH` entry holding the marker transport helper `git-remote-codotheca`.
    pub(crate) helper_dir: PathBuf,
    /// The creation-token bundle list `fetch.bundleURI` points at.
    pub(crate) token_list: PathBuf,
    /// Hooks that write markers, for `core.hooksPath`.
    pub(crate) marker_hooks: PathBuf,
    /// A template directory holding a marker file, for `init.templateDir`.
    pub(crate) template_dir: PathBuf,
    /// The clone URL a user's `insteadOf` rewrites to the origin.
    pub(crate) clone_url: String,
}

fn marker_script(path: &Path, marker: &Path) {
    std::fs::write(
        path,
        format!(
            "#!/bin/sh\ntouch '{}'\nexit 1\n",
            marker.to_string_lossy().replace('\\', "/")
        ),
    )
    .expect("marker script");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    }
}

impl World {
    /// Fixture git in `cwd`, isolated from the developer's config and from every `GIT_*`
    /// variable the process inherited — a child carrying a route's `GIT_CONFIG_COUNT` or the
    /// hostile parent's `GIT_TRACE` must not have the comparator itself obey them.
    pub(crate) fn git(&self, cwd: &Path, args: &[&str]) -> String {
        let mut cmd = Command::new("git");
        for (key, _) in std::env::vars_os() {
            if key
                .to_string_lossy()
                .to_ascii_uppercase()
                .starts_with("GIT_")
            {
                cmd.env_remove(key);
            }
        }
        let out = cmd
            .current_dir(cwd)
            .env("HOME", &self.home)
            .env("XDG_CONFIG_HOME", &self.home)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", self.home.join("empty.gitconfig"))
            .env("GIT_AUTHOR_NAME", "Fixture")
            .env("GIT_AUTHOR_EMAIL", "fixture@example.invalid")
            .env("GIT_COMMITTER_NAME", "Fixture")
            .env("GIT_COMMITTER_EMAIL", "fixture@example.invalid")
            .env("GIT_TERMINAL_PROMPT", "0")
            .args(["-c", "commit.gpgsign=false", "-c", "core.autocrlf=false"])
            .args(args)
            .output()
            .expect("fixture git");
        assert!(
            out.status.success(),
            "fixture git {args:?} in {} failed: {}",
            cwd.display(),
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    fn commit_file(&self, repo: &Path, name: &str, body: &str, message: &str) -> String {
        std::fs::write(repo.join(name), body).expect("fixture file");
        self.git(repo, &["add", name]);
        self.git(repo, &["commit", "-q", "-m", message]);
        self.git(repo, &["rev-parse", "HEAD"]).trim().to_owned()
    }

    /// The paths of a world already built under `root` — how a re-executed child finds it.
    pub(crate) fn at(root: &Path) -> Self {
        let root = root.canonicalize().expect("canonical root");
        Self {
            home: root.join("home"),
            work: root.join("work"),
            origin: root.join("origin.git"),
            decoy: root.join("decoy.git"),
            markers: root.join("markers"),
            helper_dir: root.join("helpers"),
            token_list: root.join("list.cfg"),
            marker_hooks: root.join("marker-hooks"),
            template_dir: root.join("template"),
            clone_url: "https://forge.invalid/acme/widget.git".to_owned(),
            root,
        }
    }

    /// Marker programs: a transport helper on `PATH`, a gpg, an fsmonitor, every hook, and a
    /// template file. Each would leave a file in `markers` if it ever ran.
    fn plant_markers(&self) {
        marker_script(
            &self.helper_dir.join("git-remote-codotheca"),
            &self.markers.join("helper"),
        );
        marker_script(&self.root.join("marker-gpg"), &self.markers.join("gpg"));
        marker_script(
            &self.root.join("marker-fsmonitor"),
            &self.markers.join("fsmonitor"),
        );
        for hook in [
            "reference-transaction",
            "post-checkout",
            "post-merge",
            "pre-auto-gc",
            "post-rewrite",
            "pre-push",
            "fsmonitor-watchman",
        ] {
            marker_script(
                &self.marker_hooks.join(hook),
                &self.markers.join(format!("hook-{hook}")),
            );
        }
        std::fs::write(
            self.template_dir.join("template-marker"),
            b"from the template\n",
        )
        .expect("template marker");
    }

    /// Build D10's world under `root`, the work repository's `HEAD` detached when `detached`.
    pub(crate) fn build(root: &Path, detached: bool) -> Self {
        let root = root.canonicalize().expect("canonical root");
        for dir in ["home", "markers", "helpers", "marker-hooks", "template"] {
            std::fs::create_dir_all(root.join(dir)).expect("fixture dir");
        }
        std::fs::write(root.join("home").join("empty.gitconfig"), b"").expect("empty global");
        let w = Self::at(&root);

        w.plant_markers();

        // A seed, the bare origin cloned from it, and the work clone of the origin.
        let seed = w.root.join("seed");
        w.git(
            &w.root,
            &["init", "-q", "-b", "main", &seed.to_string_lossy()],
        );
        let base = w.commit_file(&seed, "a.txt", "one\n", "one");
        w.commit_file(&seed, "b.txt", "two\n", "two");
        w.git(
            &w.root,
            &[
                "clone",
                "-q",
                "--bare",
                &seed.to_string_lossy(),
                &w.origin.to_string_lossy(),
            ],
        );
        w.git(
            &w.root,
            &[
                "clone",
                "-q",
                &w.origin.to_string_lossy(),
                &w.work.to_string_lossy(),
            ],
        );

        // Origin moves on: a diverged `main` and an extra branch the work repository lacks.
        w.commit_file(&seed, "c.txt", "diverged\n", "diverged on origin");
        w.git(&seed, &["checkout", "-q", "-b", "extra"]);
        w.commit_file(&seed, "e.txt", "extra\n", "extra branch");
        w.git(
            &seed,
            &[
                "push",
                "-q",
                "--force",
                &w.origin.to_string_lossy(),
                "main",
                "extra",
            ],
        );
        w.git(
            &w.root,
            &[
                "clone",
                "-q",
                "--mirror",
                &w.origin.to_string_lossy(),
                &w.decoy.to_string_lossy(),
            ],
        );
        let bundle = w.root.join("all.bundle");
        w.git(
            &w.origin,
            &["bundle", "create", &bundle.to_string_lossy(), "--all"],
        );
        std::fs::write(
            &w.token_list,
            format!(
                "[bundle]\n\tversion = 1\n\tmode = all\n\theuristic = creationToken\n\
                 [bundle \"one\"]\n\turi = {}\n\tcreationToken = 1\n",
                bundle.to_string_lossy().replace('\\', "/")
            ),
        )
        .expect("token list");

        // The work repository, dressed: a local-only branch checked out, a local-only annotated
        // tag, a stale tracking ref, three stashes, an ignored non-junk file, a dirty and a staged
        // file, a graft file and a replace ref.
        w.git(&w.work, &["checkout", "-q", "-b", "feature"]);
        let local = w.commit_file(&w.work, "l.txt", "local only\n", "local only");
        w.git(&w.work, &["tag", "-a", "local-tag", "-m", "local"]);
        w.git(&w.work, &["update-ref", "refs/remotes/origin/stale", &base]);
        for n in 1..=3 {
            std::fs::write(w.work.join("a.txt"), format!("stash {n}\n")).expect("stash edit");
            w.git(&w.work, &["stash", "push", "-q", "-m", &format!("s{n}")]);
        }
        std::fs::write(w.work.join(".gitignore"), b"secret.env\n").expect("ignore");
        w.git(&w.work, &["add", ".gitignore"]);
        w.git(&w.work, &["commit", "-q", "-m", "ignore"]);
        std::fs::write(w.work.join("secret.env"), b"TOKEN=local\n").expect("ignored");
        std::fs::write(w.work.join("a.txt"), b"dirty\n").expect("dirty");
        std::fs::write(w.work.join("staged.txt"), b"staged\n").expect("staged");
        w.git(&w.work, &["add", "staged.txt"]);
        std::fs::write(w.work.join(".git/info/grafts"), format!("{local}\n")).expect("grafts");
        w.git(&w.work, &["replace", &base, &local]);
        if detached {
            w.git(&w.work, &["checkout", "-q", "--detach"]);
        }
        w
    }

    /// §47.9 C's hostile write profile, as config text: every key at its destructive value.
    pub(crate) fn hostile_profile(&self) -> String {
        let p = |path: &Path| path.to_string_lossy().replace('\\', "/");
        let mut text = String::new();
        let _ = writeln!(
            text,
            "[remote \"origin\"]\n\tfetch = +refs/heads/*:refs/heads/*\n\tprune = true\n\
             \tpruneTags = true\n\ttagOpt = --tags\n\tfollowRemoteHEAD = always"
        );
        let _ = writeln!(
            text,
            "[fetch]\n\tprune = true\n\tpruneTags = true\n\trecurseSubmodules = yes\n\
             \twriteCommitGraph = true\n\tbundleURI = {}",
            p(&self.token_list)
        );
        let _ = writeln!(text, "[submodule]\n\trecurse = true");
        let _ = writeln!(text, "[transfer]\n\tbundleURI = true");
        let _ = writeln!(
            text,
            "[url \"{}\"]\n\tinsteadOf = {}",
            p(&self.decoy),
            p(&self.origin)
        );
        let _ = writeln!(
            text,
            "[protocol \"file\"]\n\tallow = always\n[protocol \"codotheca\"]\n\tallow = always"
        );
        let _ = writeln!(
            text,
            "[tag]\n\tgpgSign = true\n\tforceSignAnnotated = true\n[gpg]\n\tprogram = {}",
            p(&self.root.join("marker-gpg"))
        );
        let _ = writeln!(
            text,
            "[core]\n\thooksPath = {}\n\tfsmonitor = {}\n\tlogAllRefUpdates = always",
            p(&self.marker_hooks),
            p(&self.root.join("marker-fsmonitor"))
        );
        let _ = writeln!(text, "[gc]\n\tauto = 1\n[maintenance]\n\tauto = true");
        let _ = writeln!(
            text,
            "[credential]\n\thelper = !touch '{}'\n[credential \"https://forge.invalid\"]\n\
             \thelper = !touch '{}'",
            p(&self.markers.join("credential")),
            p(&self.markers.join("credential-url"))
        );
        let _ = writeln!(text, "[init]\n\ttemplateDir = {}", p(&self.template_dir));
        let _ = writeln!(text, "[remote \"helper\"]\n\turl = codotheca::x");
        text
    }

    /// The hostile profile flattened to `(key, value)` pairs, for the `GIT_CONFIG_COUNT` route.
    pub(crate) fn hostile_pairs(&self) -> Vec<(String, String)> {
        let mut pairs = Vec::new();
        let mut section = String::new();
        for line in self.hostile_profile().lines() {
            let line = line.trim();
            if let Some(header) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                section = header.split_once(' ').map_or_else(
                    || header.to_owned(),
                    |(outer, inner)| format!("{outer}.{}", inner.trim_matches('"')),
                );
            } else if let Some((key, value)) = line.split_once(" = ") {
                pairs.push((format!("{section}.{key}"), value.to_owned()));
            }
        }
        pairs
    }

    /// Apply `route`: write whatever files it needs, and return the environment a child running
    /// the product must be given. The user-global file always carries the clone URL's rewrite to
    /// the origin, which is how a user's own config reaches a clone.
    pub(crate) fn apply(&self, route: Route) -> Vec<(String, OsString)> {
        let p = |path: &Path| path.to_string_lossy().replace('\\', "/");
        let profile = self
            .root
            .join(format!("hostile-{}.gitconfig", route.slug()));
        std::fs::write(&profile, self.hostile_profile()).expect("profile");
        let mut global = format!(
            "[url \"{}\"]\n\tinsteadOf = {}\n",
            p(&self.origin),
            self.clone_url
        );
        let mut env: Vec<(String, OsString)> = Vec::new();
        match route {
            Route::RepoConfig => {
                let config = self.work.join(".git/config");
                let mut text = std::fs::read_to_string(&config).expect("config");
                text.push_str(&self.hostile_profile());
                std::fs::write(&config, text).expect("repo config");
            }
            Route::Global => global.push_str(&self.hostile_profile()),
            Route::Include => {
                let include = format!("[include]\n\tpath = {}\n", p(&profile));
                let config = self.work.join(".git/config");
                let mut text = std::fs::read_to_string(&config).expect("config");
                text.push_str(&include);
                std::fs::write(&config, text).expect("repo config");
                global.push_str(&include);
            }
            Route::Count => {
                let pairs = self.hostile_pairs();
                env.push((
                    "GIT_CONFIG_COUNT".to_owned(),
                    pairs.len().to_string().into(),
                ));
                for (n, (key, value)) in pairs.into_iter().enumerate() {
                    env.push((format!("GIT_CONFIG_KEY_{n}"), key.into()));
                    env.push((format!("GIT_CONFIG_VALUE_{n}"), value.into()));
                }
            }
        }
        env.extend(user_global(&self.root, &global));
        env
    }

    /// Whether `env` (from [`Self::apply`]) delivers the hostile profile to the git under test:
    /// `fetch.prune` reads `true` in the work repository. A route the release cannot read would
    /// otherwise pass layer C vacuously.
    pub(crate) fn route_delivers(&self, env: &[(String, OsString)]) -> bool {
        let mut cmd = Command::new(super::test_git());
        for (key, _) in std::env::vars_os() {
            if key
                .to_string_lossy()
                .to_ascii_uppercase()
                .starts_with("GIT_")
            {
                cmd.env_remove(key);
            }
        }
        let out = cmd
            .current_dir(&self.work)
            .args(["config", "--get", "fetch.prune"])
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .envs(env.iter().map(|(key, value)| (key, value)))
            .output()
            .expect("git config");
        String::from_utf8_lossy(&out.stdout).trim() == "true"
    }

    /// The comparator's view of the world at one instant.
    pub(crate) fn snapshot(&self, trace: &Path) -> WorldSnapshot {
        let mut refs = BTreeMap::new();
        for (label, repo) in [
            ("work", &self.work),
            ("origin", &self.origin),
            ("decoy", &self.decoy),
        ] {
            refs.insert(
                label.to_owned(),
                self.git(repo, &["for-each-ref", "--format=%(refname) %(objectname)"]),
            );
        }
        let mut files = BTreeMap::new();
        for (label, dir) in [
            ("work", &self.work),
            ("origin", &self.origin),
            ("decoy", &self.decoy),
        ] {
            hash_tree(dir, dir, label, &mut files);
        }
        let objects = self
            .git(
                &self.work,
                &[
                    "cat-file",
                    "--batch-all-objects",
                    "--batch-check=%(objectname)",
                ],
            )
            .lines()
            .map(str::to_owned)
            .collect();
        let mut markers: Vec<String> = std::fs::read_dir(&self.markers)
            .expect("markers dir")
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        if trace.exists() {
            markers.push(format!("trace file {}", trace.display()));
        }
        markers.sort();
        WorldSnapshot {
            refs,
            files,
            objects,
            markers,
        }
    }
}

fn hash_tree(base: &Path, dir: &Path, label: &str, out: &mut BTreeMap<String, String>) {
    use sha2::Digest as _;
    for entry in std::fs::read_dir(dir)
        .expect("readable fixture dir")
        .flatten()
    {
        let path = entry.path();
        let kind = entry.file_type().expect("kind");
        if kind.is_dir() {
            hash_tree(base, &path, label, out);
        } else {
            let rel = path
                .strip_prefix(base)
                .expect("inside")
                .to_string_lossy()
                .replace('\\', "/");
            let bytes = std::fs::read(&path).unwrap_or_default();
            out.insert(
                format!("{label}/{rel}"),
                format!("{:x}", sha2::Sha256::digest(&bytes)),
            );
        }
    }
}

/// What the comparator records: refs through git, every file's hash, the work repository's
/// object set, and every marker a program left.
#[derive(Debug, Clone)]
pub(crate) struct WorldSnapshot {
    /// `for-each-ref` output per repository.
    pub(crate) refs: BTreeMap<String, String>,
    /// `<repo>/<path>` to SHA-256, for every file of work, origin and decoy.
    pub(crate) files: BTreeMap<String, String>,
    /// The work repository's object ids.
    pub(crate) objects: BTreeSet<String>,
    /// Every marker file present, and the trace file if it exists.
    pub(crate) markers: Vec<String>,
}

/// §47.9 C's comparison: the difference between two snapshots must be exactly what `effect`
/// declares. Returns every violation, each named, so a failing run says what moved.
pub(crate) fn effect_violations(
    effect: codotheca_core::gitw::DeclaredEffect,
    before: &WorldSnapshot,
    after: &WorldSnapshot,
) -> Vec<String> {
    use codotheca_core::gitw::DeclaredEffect;

    let mut found = Vec::new();
    for (repo, refs) in &before.refs {
        let now = after.refs.get(repo).cloned().unwrap_or_default();
        if &now != refs {
            let was: BTreeSet<&str> = refs.lines().collect();
            let is: BTreeSet<&str> = now.lines().collect();
            found.push(format!(
                "{repo}: refs moved — gone {:?}, new {:?}",
                was.difference(&is).collect::<Vec<_>>(),
                is.difference(&was).collect::<Vec<_>>()
            ));
        }
    }
    for (path, hash) in &before.files {
        match after.files.get(path) {
            None => found.push(format!("removed {path}")),
            Some(now) if now != hash => found.push(format!("changed {path}")),
            Some(_) => {}
        }
    }
    let objects_file = |path: &str| {
        path.strip_prefix("work/.git/objects/").is_some_and(|rest| {
            rest.starts_with("pack/")
                || rest.split_once('/').is_some_and(|(dir, _)| {
                    dir.len() == 2 && dir.bytes().all(|b| b.is_ascii_hexdigit())
                })
        })
    };
    for path in after
        .files
        .keys()
        .filter(|p| !before.files.contains_key(*p))
    {
        let allowed = matches!(effect, DeclaredEffect::ObjectsOnly) && objects_file(path);
        if !allowed {
            found.push(format!("added {path}"));
        }
    }
    let lost: Vec<&String> = before.objects.difference(&after.objects).collect();
    if !lost.is_empty() {
        found.push(format!("the object set shrank by {}", lost.len()));
    }
    if !after.markers.is_empty() {
        found.push(format!("markers written: {:?}", after.markers));
    }
    found
}
