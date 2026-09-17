//! Codotheca core, the binary.
//!
//! stdout carries protocol frames and nothing else. Every diagnostic goes to stderr, which
//! the shell drains into its rolling log. `claim_stdout` takes the handle before anything else
//! can write to it, and every diagnostic below goes through `note`; nothing here prints.
//!
//! This file is composition and nothing else. The shape is plan 03's — argv, advisory lock,
//! stdout, transport, `hello`, `run_loop` — with the startup sequence and the real handler
//! inserted between `hello` and the loop.

#![forbid(unsafe_code)]

use codotheca_core::assembly::startup::{open_index, run_startup};
use codotheca_core::assembly::{CoreDeps, CoreHandler};
use codotheca_core::clock::{local_utc_offset_min, Clock, SystemClock};
use codotheca_core::git::{GitBackend, GitExec, GitSlots, SystemGit};
use codotheca_core::gitw::credential::{
    configure_data_dir, run_credential_helper, CREDENTIAL_MODE_FLAG,
};
use codotheca_core::lifecycle::{
    parse_args, CoreLock, LockError, OsParentProbe, EXIT_BAD_ARGS, EXIT_LOCK_HELD,
};
use codotheca_core::mount::{MountResolver, SystemMountResolver};
use codotheca_core::proto::dispatch::{run_loop, send_hello};
use codotheca_core::proto::pubsub::{EventSink, Publisher, PublisherSink, TOPIC_HIGH_WATER};
use codotheca_core::proto::transport::{claim_stdout, Transport, WRITER_CAPACITY};
use codotheca_core::proto::wire::Epoch;
use codotheca_core::scan::ScanSupervisor;
use codotheca_core::session::manager::SessionManager;
use std::ffi::OsStr;
use std::io::{Read as _, Write as _};
use std::path::Path;
use std::process::ExitCode;
use std::sync::{Arc, Mutex};

fn note(line: &str) {
    let _ = writeln!(std::io::stderr(), "{line}");
}

fn credential_helper_mode() -> Option<ExitCode> {
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() != Some(OsStr::new(CREDENTIAL_MODE_FLAG)) {
        return None;
    }
    let Some(nonce_path) = args.next() else {
        note("codotheca-core: credential helper is missing its nonce path");
        return Some(ExitCode::from(EXIT_BAD_ARGS));
    };
    let Some(channel) = args.next() else {
        note("codotheca-core: credential helper is missing its channel");
        return Some(ExitCode::from(EXIT_BAD_ARGS));
    };
    let Some(operation) = args.next() else {
        note("codotheca-core: credential helper is missing its operation");
        return Some(ExitCode::from(EXIT_BAD_ARGS));
    };
    if args.next().is_some() {
        note("codotheca-core: credential helper received an unexpected argument");
        return Some(ExitCode::from(EXIT_BAD_ARGS));
    }

    let Some(mut out) = claim_stdout() else {
        note("codotheca-core: stdout was already claimed");
        return Some(ExitCode::FAILURE);
    };
    let operation = operation.to_string_lossy();
    let result = match operation.as_ref() {
        // Git may ask to persist or remove what `get` returned. This helper is one-shot and stores
        // nothing, so both operations deliberately answer with no credential protocol records.
        "store" | "erase" => Ok(()),
        "get" => {
            let mut nonce = String::new();
            let read = std::fs::File::open(&nonce_path)
                .and_then(|file| file.take(1_025).read_to_string(&mut nonce));
            match read {
                Ok(_)
                    if nonce.len() == 64 && nonce.bytes().all(|byte| byte.is_ascii_hexdigit()) =>
                {
                    channel.to_str().map_or_else(
                        || {
                            Err(std::io::Error::new(
                                std::io::ErrorKind::InvalidInput,
                                "credential channel is not valid Unicode",
                            ))
                        },
                        |channel| run_credential_helper(&nonce, channel, &mut out),
                    )
                }
                Ok(_) => Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "credential nonce file is malformed",
                )),
                Err(error) => Err(error),
            }
        }
        _ => Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "unsupported credential helper operation",
        )),
    };
    if let Err(error) = result {
        note(&format!("codotheca-core: credential helper: {error}"));
        return Some(ExitCode::FAILURE);
    }
    Some(ExitCode::SUCCESS)
}

/// Plan 16's `FirstRunEnv`, assembled from the environment.
///
/// Private: nothing outside the binary composes it.
fn first_run_env(data_dir: &Path) -> codotheca_core::firstrun::FirstRunEnv {
    let _ = data_dir;
    codotheca_core::firstrun::FirstRunEnv {
        sources: codotheca_core::firstrun::sources::SourceEnv {
            home: std::env::var_os("HOME")
                .or_else(|| std::env::var_os("USERPROFILE"))
                .map_or_else(std::path::PathBuf::new, std::path::PathBuf::from),
            app_data: std::env::var_os("APPDATA").map(std::path::PathBuf::from),
            xdg_config: std::env::var_os("XDG_CONFIG_HOME").map(std::path::PathBuf::from),
        },
        classifier: Arc::new(
            codotheca_core::firstrun::classify::NativeRootClassifier::new(Arc::new(
                SystemMountResolver::new(),
            )),
        ),
        // §13's probe, reading the two quiet `wsl.exe` listings. Neither listing names a distro,
        // so neither can start one — starting a stopped distro stays an explicit, consented act.
        // A machine with no `wsl.exe` answers with an empty list, which is what the `NoDistros`
        // stand-in used to say and is now said by the code that would actually find them.
        distros: Arc::new(codotheca_core::wsl::distros::SystemDistroProbe::system()),
        platform: codotheca_core::index::path::native_platform(),
        skip: codotheca_core::scan::skiplist::SkipList::default(),
        cache: codotheca_core::firstrun::roots::SuggestionCache::new(),
    }
}

/// §13's dispatcher, or `None`.
///
/// Three things have to be true at once: the shell handed this build a worker path, that file
/// reads, and the host lists at least one distro. Any of them missing means there is nothing to
/// dispatch to — and the caller says so rather than installing a stand-in that reports every
/// distro as empty.
///
/// The consent set is read here, once, from `app_meta` (plan 18's deviation: `settings` is not a
/// table). A malformed value consents to nothing, which is "ask again" and never "start it".
fn build_wsl_dispatcher(
    worker: Option<&Path>,
    index: &Arc<Mutex<codotheca_core::index::Index>>,
) -> Option<Arc<codotheca_core::wsl::dispatch::WslDispatcher>> {
    let bytes = std::fs::read(worker?)
        .map_err(|e| note(&format!("codotheca-core: wsl worker unreadable: {e}")))
        .ok()?;
    let cli: Arc<dyn codotheca_core::wsl::distros::WslCli> =
        Arc::new(codotheca_core::wsl::distros::SystemWslCli::new());
    let installed = codotheca_core::wsl::distros::installed_distros(cli.as_ref()).ok()?;
    if installed.is_empty() {
        return None;
    }
    let consented = {
        let guard = index.lock().ok()?;
        let set = codotheca_core::wsl::dispatch::read_consented(guard.conn());
        drop(guard);
        set
    };
    let launcher = Arc::new(codotheca_core::wsl::conn::WslExeLauncher::new(
        cli,
        Arc::new(bytes),
        None,
    ));
    Some(Arc::new(codotheca_core::wsl::dispatch::WslDispatcher::new(
        Arc::new(codotheca_core::wsl::conn::WslWorkerPool::new(launcher)),
        installed,
        consented,
    )))
}

#[allow(clippy::too_many_lines)]
fn main() -> ExitCode {
    // This mode runs while the core that started Git holds the advisory lock, by design, so it
    // must return before ordinary argv parsing or lock acquisition. Its stdout carries Git's
    // credential protocol, not length-prefixed protocol frames. No stdout-discipline exemption is
    // needed: `claim_stdout()` hands the handle out once behind the transport module's `AtomicBool`,
    // and that single ownership is the invariant the gate enforces (the worker binary is precedent).
    if let Some(exit) = credential_helper_mode() {
        return exit;
    }

    let args = match parse_args(std::env::args().skip(1)) {
        Ok(a) => a,
        Err(e) => {
            note(&format!("codotheca-core: {e}"));
            return ExitCode::from(EXIT_BAD_ARGS);
        }
    };

    if let Err(error) = configure_data_dir(&args.data_dir) {
        note(&format!(
            "codotheca-core: credential data directory: {error}"
        ));
        return ExitCode::from(EXIT_BAD_ARGS);
    }

    let _lock = match CoreLock::acquire(&args.data_dir) {
        Ok(l) => l,
        Err(LockError::Held) => {
            note("codotheca-core: another core holds the advisory lock for this data directory");
            return ExitCode::from(EXIT_LOCK_HELD);
        }
        Err(e) => {
            note(&format!("codotheca-core: {e}"));
            return ExitCode::from(EXIT_BAD_ARGS);
        }
    };

    let Some(stdout) = claim_stdout() else {
        note("codotheca-core: stdout was already claimed");
        return ExitCode::FAILURE;
    };

    let transport = match Transport::start(std::io::stdin(), stdout, WRITER_CAPACITY) {
        Ok(t) => t,
        Err(e) => {
            note(&format!("codotheca-core: transport: {e}"));
            return ExitCode::FAILURE;
        }
    };

    let epoch = Epoch(args.epoch);
    if let Err(e) = send_hello(&transport.sink, epoch) {
        note(&format!("codotheca-core: hello: {e}"));
        return ExitCode::FAILURE;
    }

    // One `Publisher`, owned by one `PublisherSink`, shared with every worker as
    // `Arc<dyn EventSink>`. A second one would restart every topic at seq 1, which the shell
    // reads as a replay followed by a permanent gap (§2.3).
    let events = Arc::new(PublisherSink::new(Publisher::new(
        transport.sink.clone(),
        epoch,
        TOPIC_HIGH_WATER,
    )));

    let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
    // A recognised fatal already exited inside `open_index`, with §11.2a's report written for
    // the shell to draw. Reaching this arm means the error was not one of the three.
    let index = match open_index(&args.data_dir, clock.now_unix()) {
        Ok(index) => index,
        Err(e) => {
            note(&format!("codotheca-core: index: {e}"));
            return ExitCode::FAILURE;
        }
    };
    let index = Arc::new(Mutex::new(index));

    let git_exec = Arc::new(GitExec::system(args.data_dir.join("empty-hooks")));
    let slots = Arc::new(GitSlots::for_machine());
    let git: Arc<dyn GitBackend> = Arc::new(SystemGit::new(
        Arc::clone(&git_exec),
        slots,
        Arc::clone(&clock),
    ));
    let mount: Arc<dyn MountResolver> = Arc::new(SystemMountResolver::new());

    let activity: Box<dyn codotheca_core::session::watch::ActivitySource> =
        match codotheca_core::session::watch::NotifyActivitySource::new() {
            Ok(source) => Box::new(source),
            // §9's watcher is a nicety: without it a segment closes on its idle deadline
            // instead of on the last edit. Not a reason to refuse to start.
            Err(e) => {
                note(&format!("codotheca-core: activity watcher: {e}"));
                events.emit(
                    "core",
                    "degraded",
                    serde_json::json!({
                        "what": "activity_watcher",
                        "message": format!("{e}"),
                    }),
                );
                Box::new(codotheca_core::session::watch::NullActivitySource)
            }
        };

    // §13's dispatcher, built only when this build has a worker to run and the host has
    // distros to run it in. **No null stand-in**: on Linux there is no bridge to cross, and on
    // Windows an absent worker is a fact the scan reports rather than a silence it passes off
    // as an empty distro.
    let wsl = build_wsl_dispatcher(args.worker.as_deref(), &index);
    if args.worker.is_some() && wsl.is_none() {
        note("codotheca-core: no WSL worker; distros will not be scanned");
    }

    // §4.1a's pump. Started here because the shelf is uncomputed until it runs: the walk writes
    // rows and nothing derives a fact about them without J1-J6. `CoreHandler` owns it and stops
    // it in `shutdown`, before the publisher closes.
    let jobs = codotheca_core::assembly::jobs::JobPump::start(
        Arc::clone(&index),
        Arc::clone(&git),
        Arc::clone(&clock),
        Arc::clone(&events) as Arc<dyn EventSink>,
    );

    // The same `Arc<Mutex<Index>>` reached a different way, never a second connection. The
    // launcher and the scan context share this one store.
    let scan_store: Arc<dyn codotheca_core::scan::presence::ScanStore> = Arc::new(
        codotheca_core::scan::store::SqliteScanStore::new(Arc::clone(&index)),
    );

    // §20.6 and §20.13's production implementations, constructed here and nowhere else.
    //
    // `ReqwestTransport::new` is the only `reqwest::blocking::Client` in the process, and
    // `core/tests/http_transport.rs` asserts that by walking the sources. A transport that
    // cannot be built is not fatal: the product is fully functional with zero accounts, and
    // every remote value is *unknown* until one exists, so the account surface refuses rather
    // than the core failing to start.
    let http_transport: Arc<dyn codotheca_core::http::HttpTransport> =
        match codotheca_core::http::ReqwestTransport::new() {
            Ok(transport) => Arc::new(transport),
            Err(error) => {
                note(&format!(
                    "codotheca-core: no HTTP transport ({error}); account commands will refuse"
                ));
                Arc::new(codotheca_core::http::RefusingTransport)
            }
        };
    // §21's decorator over §20's transport, and **the provider is handed the decorated one**.
    //
    // Assembled by `assembly::sync::build_forge` rather than inline, so the wiring can be
    // asserted by **running** it: `core/tests/sync_assembly.rs` calls the same function over a
    // fake and proves a provider call drains an observation. A check that read this file could
    // only see that a decorator was constructed, not what the provider was handed — which is
    // exactly the difference that matters (R88).
    let (provider, observing) = codotheca_core::assembly::sync::build_forge(
        Arc::clone(&http_transport),
        Arc::clone(&clock),
        codotheca_core::provider::listing::GITHUB_CANONICAL_HOST.to_owned(),
    );
    let tokens: Arc<dyn codotheca_core::accounts::keychain::TokenStore> =
        Arc::new(codotheca_core::accounts::keychain::KeyringTokenStore::new());

    // §21.1's runner, beside the job pump and stopped in the same order. It is started here for
    // the same reason `jobs` is: nothing schedules a forge read without it, and a runner
    // constructed in a test and never in the product is the defect R1's family catalogues.
    let sync = codotheca_core::assembly::sync::SyncPump::start(
        Arc::clone(&index),
        codotheca_core::sync::SyncDeps {
            provider: Arc::clone(&provider),
            transport: Arc::clone(&observing),
            tokens: Arc::clone(&tokens),
            clock: Arc::clone(&clock),
            cancel: codotheca_core::cancel::CancelToken::new(),
        },
        Arc::clone(&events) as Arc<dyn EventSink>,
    );

    let deps = CoreDeps {
        index: Arc::clone(&index),
        provider,
        tokens,
        sync,
        http: Arc::clone(&http_transport),
        client_id: codotheca_core::accounts::device::GITHUB_CLIENT_ID.to_owned(),
        clock: Arc::clone(&clock),
        git: Arc::clone(&git),
        mount: Arc::clone(&mount),
        spawner: Box::new(codotheca_core::launch::spawn::OsSpawner),
        sessions: SessionManager::new(
            Arc::clone(&clock),
            Arc::clone(&events) as Arc<dyn EventSink>,
            activity,
            Arc::new(codotheca_core::session::activity::GitIgnoreCheck {
                exec: Arc::clone(&git_exec),
                limits: codotheca_core::git::RunLimits::none(),
            }),
        ),
        // A supervisor over plan 07's `Send + Sync` launcher seam, not a `JobRunner`: no
        // `rusqlite::Connection` crosses it, which is what keeps the one-writer rule intact.
        scans: ScanSupervisor::new(Arc::new(
            codotheca_core::scan::launcher::ThreadScanLauncher::new(
                codotheca_core::scan::launcher::ScanLauncherDeps {
                    store: Arc::clone(&scan_store),
                    index: Arc::clone(&index),
                    git: Arc::clone(&git),
                    mounts: Arc::clone(&mount),
                    clock: Arc::clone(&clock),
                    skip: Arc::new(codotheca_core::scan::skiplist::SkipList::default()),
                    wsl,
                    // The real queue. `NullJobSink` is the absence of a scheduler, not a fake of
                    // one, and installing it here is what left every discovery uncomputed.
                    jobs: jobs.sink(),
                    events: Arc::clone(&events) as Arc<dyn EventSink>,
                },
            ),
        )),
        scan_store: Arc::clone(&scan_store),
        firstrun: first_run_env(&args.data_dir),
        jobs: jobs.clone(),
        events: Arc::clone(&events),
        // §8.1's era bands cut on the local calendar year, so this is a correctness input.
        tz_offset_min: local_utc_offset_min(),
    };

    let mut handler = CoreHandler::new(deps);
    let summary = run_startup(&mut handler);
    note(&format!("codotheca-core: startup {summary:?}"));

    let parent = OsParentProbe::new(args.parent_pid);
    let exit = run_loop(transport, &events, &mut handler, epoch, &parent);
    note(&format!("codotheca-core: exiting ({exit:?})"));
    ExitCode::SUCCESS
}
