//! A stand-in for `git` that records what it was **actually given**, for the write audit.
//!
//! **Why this exists rather than an assertion over `Intent::argv()`.** R88: *"a test that asserts
//! a URL, an argv, a command name or a function call is asserting an intention, and an intention
//! passes whether or not anything acts on it."* Between `Intent::argv()` and the child sit
//! `write_base_args`, `WriteExec` and the `Command` builder — which is exactly where `--prune`, a
//! re-inherited `credential.helper` or a restored `GIT_ASKPASS` would enter unseen. So the audit
//! points the write path at **this** program and reads the file it writes: that file is the
//! artefact, and the `Vec<OsString>` is the intention.
//!
//! **Why a declared `[[bin]]` rather than a script.** Windows `CreateProcess` will not spawn a
//! shebang file, and a stand-in that silently fails to spawn on the one target the orphan-lock
//! trap lives on is the same class as the plan-16 tests that fed Unix paths to a Windows parser
//! and read green on the platform that never ran them. Declared with
//! `required-features = ["testkit"]` beside `codotheca-corpus`, so `--all-features` clippy builds
//! it and a missing path is a hard error at gate time rather than at spawn time.
//!
//! **It answers what the write path asks.** `SystemMutatingGit::run` first reads the effective
//! config to enumerate filter drivers, then runs the intent; the verifying read's two parsed
//! steps, `ls-remote --get-url` and `ls-remote`, are answered on stdout as git would, and every
//! invocation's stdin is recorded beside its argv and environment.
//!
//! **`git --version`**, the governed floor's probe (§47.8), is answered as a release above the
//! floor and recorded beside the directory it runs in — the empty hooks directory.
//!
//! **Mode comes from argv\[0\], not from the environment.** A test that needs the
//! no-filters-configured answer copies this binary under a name containing `nofilters`; one that
//! needs a git below the governed floor, under a name containing `oldgit`, which answers
//! `--version` as 2.28.0. An
//! environment variable would be process-global and therefore racy across parallel tests — and
//! worse, `neutralise_env` is part of what this program exists to observe, so a recorder needing
//! a variable to survive it would be observing itself.

use std::ffi::OsString;
use std::io::{Read as _, Write as _};
use std::path::PathBuf;

/// Separator between records. NUL, because an argv element or an environment value may contain
/// anything else — including a newline.
const SEP: u8 = 0;

/// The driver this stand-in reports as configured, so the audit can prove the production path
/// enumerated it **and** rendered its three neutralising options into the child's real argv.
const AUDIT_DRIVER: &str = "auditdriver";

fn main() {
    let argv0 = std::env::args_os().next().unwrap_or_default();
    let invoked_as = PathBuf::from(&argv0)
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let no_filters = invoked_as.contains("nofilters");
    let old_git = invoked_as.contains("oldgit");

    let args: Vec<OsString> = std::env::args_os().skip(1).collect();

    // The precondition read. `git config --get-regexp` exits 1 when it matches nothing, and the
    // write path must tell that apart from a failure — so this stand-in can produce both.
    if args.iter().any(|a| a == "config") {
        if no_filters {
            std::process::exit(1);
        }
        let record = format!("filter.{AUDIT_DRIVER}.clean\ngit-audit-filter");
        // Through `claim_stdout`, never `std::io::stdout()` directly. That function hands the
        // handle out **once**, behind an `AtomicBool`, which is what makes *"stdout carries
        // protocol frames and nothing else"* checkable at runtime rather than merely stated —
        // and `scripts/check-stdout-discipline.mjs` bans the direct call everywhere in
        // `core/src` but the one module that owns it. A stand-in for git legitimately writes to
        // stdout, because that is where `git config` answers; it takes the handle the same way
        // `core/src/bin/worker.rs` does.
        let Some(mut out) = codotheca_core::proto::transport::claim_stdout() else {
            std::process::exit(5);
        };
        if out.write_all(record.as_bytes()).is_err() || out.write_all(&[SEP]).is_err() {
            std::process::exit(4);
        }
        return;
    }

    let Some(last) = args.last() else {
        // Nothing to key the recording on. Exit non-zero so the audit fails loudly rather than
        // reading a file that was never written.
        std::process::exit(2);
    };

    // **The recording is keyed on an ABSOLUTE path, and a relative one is refused.**
    //
    // Measured, not hypothetical: an earlier version keyed on the last argv element whatever it
    // was. The retired `Intent::Fetch` rendered its **remote name** last — `origin` — so this
    // wrote `origin.recorded` relative to its own working directory, which is the crate root,
    // and left a 9 KB file **inside the repository**. A test binary that writes into the source
    // tree is a defect whether or not anyone notices the file.
    //
    // Refusing here also keeps the audit honest: a variant with no path to key on cannot be
    // spawn-recorded at all, and the test must say so rather than read a file from somewhere else.
    //
    // **An invocation that runs *in* a repository is keyed on its `-C` directory** — both paths
    // render `-C <dir>` before the subcommand, so a verifying read ending in a remote name still
    // names an absolute path to record beside. It is only ever the directory's sibling
    // `<dir>.recorded`, never a file inside the repository being recorded.
    let work_dir = args
        .windows(2)
        .find(|pair| pair.first().is_some_and(|flag| flag == "-C"))
        .and_then(|pair| pair.get(1))
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute());
    // The version probe runs in the empty hooks directory and names no path, so it is keyed on
    // that working directory.
    let version_probe = matches!(args.as_slice(), [only] if only == "--version");
    let keyed_on_last = PathBuf::from(last).is_absolute();
    let mut target = if version_probe {
        std::env::current_dir().unwrap_or_else(|_| std::process::exit(6))
    } else if keyed_on_last {
        PathBuf::from(last)
    } else if let Some(dir) = work_dir {
        dir
    } else {
        std::process::exit(6);
    };
    let mut name = target.file_name().unwrap_or_default().to_os_string();
    name.push(".recorded");
    target.set_file_name(name);

    // Stdin is argv by another channel (§47.2 rule 2), so it is recorded too. `Stdio::null()`
    // reads as empty.
    let mut stdin = Vec::new();
    if std::io::stdin().read_to_end(&mut stdin).is_err() {
        std::process::exit(7);
    }

    // **Appended, one `CALL` record per invocation.** The verifying read spawns up to three
    // children in one repository, all keyed on its `-C` directory; overwriting would keep only
    // the last and hide the order the audit asserts.
    let mut blob: Vec<u8> = b"CALL".to_vec();
    blob.push(SEP);
    for arg in &args {
        blob.extend_from_slice(b"ARGV\t");
        blob.extend_from_slice(arg.to_string_lossy().as_bytes());
        blob.push(SEP);
    }
    for (key, value) in std::env::vars_os() {
        blob.extend_from_slice(b"ENV\t");
        blob.extend_from_slice(key.to_string_lossy().as_bytes());
        blob.push(b'=');
        blob.extend_from_slice(value.to_string_lossy().as_bytes());
        blob.push(SEP);
    }
    blob.extend_from_slice(b"STDIN\t");
    blob.extend_from_slice(&stdin);
    blob.push(SEP);

    let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&target)
    else {
        std::process::exit(3);
    };
    if file.write_all(&blob).is_err() {
        std::process::exit(4);
    }

    if let Some(answer) = answer(&args, last, version_probe, old_git) {
        let Some(mut out) = codotheca_core::proto::transport::claim_stdout() else {
            std::process::exit(5);
        };
        if out.write_all(answer.as_bytes()).is_err() {
            std::process::exit(4);
        }
    }

    // A real `git clone` creates its destination. Creating it here keeps the caller's own
    // post-conditions meaningful without pretending to be git in any other way. An invocation
    // keyed on its `-C` directory runs in a repository that already exists and creates nothing.
    if keyed_on_last {
        let _ = std::fs::create_dir_all(PathBuf::from(last));
    }
}

/// What the stand-in prints on stdout, as git would: the version probe's line, and the
/// verifying read's two parsed steps.
fn answer(
    args: &[OsString],
    last: &OsString,
    version_probe: bool,
    old_git: bool,
) -> Option<String> {
    // The verifying read's two parsed steps answer on stdout, as git does. The URL is a network
    // one under a reserved example host, so the read proceeds to its advertisement, which names
    // the all-zero object: present nowhere, so the objects step runs and is recorded too.
    if version_probe {
        Some(if old_git {
            "git version 2.28.0\n".to_owned()
        } else {
            "git version 2.99.0.recording\n".to_owned()
        })
    } else if args.iter().any(|a| a == "ls-remote") {
        if args.iter().any(|a| a == "--get-url") {
            Some(format!(
                "https://forge.example/{}.git\n",
                last.to_string_lossy()
            ))
        } else {
            let zero = "0".repeat(40);
            Some(format!("{zero}\tHEAD\n{zero}\trefs/heads/main\n"))
        }
    } else {
        None
    }
}
