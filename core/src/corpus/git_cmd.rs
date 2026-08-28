//! Every git the generator runs, with the environment pinned so two runs on two operating
//! systems produce byte-identical objects.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::corpus::CorpusError;

/// The fixed identity every corpus commit carries. `.invalid` is reserved by RFC 2606 and can
/// never route anywhere.
pub const CORPUS_AUTHOR: &str = "Corpus Author";
pub const CORPUS_EMAIL: &str = "corpus@example.invalid";

#[derive(Debug, Clone)]
pub struct CorpusGit {
    home: PathBuf,
}

impl CorpusGit {
    pub fn new(home: PathBuf) -> Result<Self, CorpusError> {
        for leaf in ["hooks-empty", "template-empty"] {
            let path = home.join(leaf);
            std::fs::create_dir_all(&path).map_err(|e| CorpusError::Io {
                path,
                message: e.to_string(),
            })?;
        }
        Ok(Self { home })
    }

    /// The empty template directory, so `git init` copies no sample hooks and two machines
    /// produce identical repositories.
    #[must_use]
    pub fn template_arg(&self) -> String {
        format!("--template={}", self.home.join("template-empty").display())
    }

    pub fn version(&self) -> Result<String, CorpusError> {
        let out = self.run(&self.home, 0, &["--version"])?;
        Ok(out.trim_start_matches("git version ").trim().to_owned())
    }

    /// Run git and return trimmed stdout as text.
    pub fn run(&self, cwd: &Path, at_unix: i64, args: &[&str]) -> Result<String, CorpusError> {
        let bytes = self.run_bytes(cwd, at_unix, args, &[])?;
        Ok(String::from_utf8_lossy(&bytes).trim_end().to_owned())
    }

    /// Run git with raw stdin and raw stdout. stdin is written on a worker thread while this
    /// thread drains stdout: writing everything and then reading deadlocks as soon as git's
    /// stdout fills the pipe buffer, which is the failure that cost this project five hours.
    pub fn run_bytes(
        &self,
        cwd: &Path,
        at_unix: i64,
        args: &[&str],
        stdin: &[u8],
    ) -> Result<Vec<u8>, CorpusError> {
        let date = format!("@{at_unix} +0000");
        let hooks = self.home.join("hooks-empty");
        let prefix: Vec<String> = vec![
            "-c".into(),
            "core.autocrlf=false".into(),
            "-c".into(),
            "core.eol=lf".into(),
            "-c".into(),
            "core.longpaths=true".into(),
            "-c".into(),
            "core.fsmonitor=false".into(),
            "-c".into(),
            format!("core.hooksPath={}", hooks.display()),
            "-c".into(),
            "gc.auto=0".into(),
            "-c".into(),
            "gc.autoDetach=false".into(),
            "-c".into(),
            "maintenance.auto=false".into(),
            "-c".into(),
            "commit.gpgsign=false".into(),
            "-c".into(),
            "tag.gpgSign=false".into(),
            "-c".into(),
            "init.defaultBranch=main".into(),
            "-c".into(),
            "advice.detachedHead=false".into(),
            "-c".into(),
            "protocol.file.allow=always".into(),
            "-c".into(),
            format!("user.name={CORPUS_AUTHOR}"),
            "-c".into(),
            format!("user.email={CORPUS_EMAIL}"),
        ];

        let mut command = Command::new("git");
        command
            .current_dir(cwd)
            .args(&prefix)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("XDG_CONFIG_HOME", &self.home)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("TZ", "UTC")
            .env("LC_ALL", "C")
            .env("GIT_AUTHOR_NAME", CORPUS_AUTHOR)
            .env("GIT_AUTHOR_EMAIL", CORPUS_EMAIL)
            .env("GIT_COMMITTER_NAME", CORPUS_AUTHOR)
            .env("GIT_COMMITTER_EMAIL", CORPUS_EMAIL)
            .env("GIT_AUTHOR_DATE", &date)
            .env("GIT_COMMITTER_DATE", &date);
        for leaked in [
            "GIT_DIR",
            "GIT_WORK_TREE",
            "GIT_INDEX_FILE",
            "GIT_OBJECT_DIRECTORY",
            "GIT_ALTERNATE_OBJECT_DIRECTORIES",
            "GIT_CEILING_DIRECTORIES",
            "GIT_ASKPASS",
            "GIT_EDITOR",
            "GIT_PAGER",
            "GIT_TRACE",
            "GIT_CONFIG",
            "GIT_CONFIG_GLOBAL",
        ] {
            command.env_remove(leaked);
        }

        let owned: Vec<String> = args.iter().map(|a| (*a).to_owned()).collect();
        let mut child = command.spawn().map_err(|e| CorpusError::Git {
            args: owned.clone(),
            code: None,
            stderr: e.to_string(),
        })?;

        let payload = stdin.to_vec();
        let mut sink = child.stdin.take();
        let writer = std::thread::spawn(move || {
            if let Some(mut pipe) = sink.take() {
                let _ = pipe.write_all(&payload);
                let _ = pipe.flush();
            }
            // Dropping the handle closes the pipe, so git sees EOF and can exit.
        });

        let output = child.wait_with_output().map_err(|e| CorpusError::Git {
            args: owned.clone(),
            code: None,
            stderr: e.to_string(),
        })?;
        let _ = writer.join();

        if output.status.success() {
            Ok(output.stdout)
        } else {
            Err(CorpusError::Git {
                args: owned,
                code: output.status.code(),
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            })
        }
    }
}

/// Write a file with exactly the bytes given, creating parents. Content is always LF.
pub fn write_file(path: &Path, contents: &[u8]) -> Result<(), CorpusError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| CorpusError::Io {
            path: parent.to_path_buf(),
            message: e.to_string(),
        })?;
    }
    std::fs::write(path, contents).map_err(|e| CorpusError::Io {
        path: path.to_path_buf(),
        message: e.to_string(),
    })
}
