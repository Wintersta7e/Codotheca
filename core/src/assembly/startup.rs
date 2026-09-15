//! What must happen before `run_loop`, and in what order.
//!
//! **Step 1 is fatal and every later step is not.** §11.2a makes an unopenable index a report
//! the shell draws instead of a window; a failed sweep or a missing git is a `core/error` event
//! and a degraded app, because refusing to start over a cosmetic fault would turn one fault into
//! a dead application.

use crate::assembly::CoreHandler;
use crate::index::{Index, IndexError};
use crate::proto::pubsub::EventSink;
use crate::surfaces::startup_failure;
use std::path::Path;
use std::sync::PoisonError;

/// What the four non-fatal steps managed to do. A `None` field is a step that failed and
/// emitted `core/error`; it is never a step that was skipped silently.
#[derive(Debug, Default)]
pub struct StartupSummary {
    pub git: Option<crate::git::GitVersion>,
    pub art: Option<crate::art::StartupReport>,
    pub orphans: Option<crate::session::orphan::OrphanReport>,
    /// What seeding the identity set added. `Some(SeedReport::default())` is a set already in
    /// force — which is the ordinary second launch — and is not the same as a failed seed.
    pub identity: Option<crate::identity::people::SeedReport>,
}

/// Open the index, or write §11.2a's report and exit.
///
/// A **recognised** fatal — a schema from the future, a failed migration, a corrupt database —
/// is not a degraded mode. The report goes to disk where the shell reads it *without opening the
/// database* (it cannot; the core is the only writer), and the process exits `EXIT_INDEX_FATAL`.
/// An **unrecognised** `IndexError` is not swallowed: it is returned, and `main` exits non-zero
/// with the reason on stderr.
///
/// A clean open clears a stale report, so yesterday's failure does not draw over today's
/// working app.
///
/// # Errors
/// The `IndexError` itself, when it is not one §11.2a recognises.
pub fn open_index(data_dir: &Path, now: i64) -> Result<Index, IndexError> {
    match Index::open_at(data_dir, now) {
        Ok(index) => {
            startup_failure::clear(data_dir);
            Ok(index)
        }
        Err(err) => {
            if let Some(failure) = startup_failure::from_index_error(&err, now) {
                // A report we cannot write is still a fatal index; the exit code is the part the
                // shell cannot miss, so a failed write must not turn this into a normal start.
                let _ = startup_failure::write(data_dir, &failure);
                std::process::exit(i32::from(startup_failure::EXIT_INDEX_FATAL));
            }
            Err(err)
        }
    }
}

/// The four calls that must happen before `run_loop`, in the order their side effects require.
///
/// 4. **The identity set, before any scan can run.** J1.5 folds each repository's committers
///    against the set and writes §5.5's `is_reference` from the result, so a set seeded *after*
///    the first walk would classify every project as somebody else's. Seeding here — earlier than
///    any command can arrive — is what makes the first scan of a fresh install compute authorship
///    against the addresses the user already commits with.
///
/// 1. **Orphan closure first.** A crash left sessions open; §9 credits their segments and closes
///    them `orphaned`. It runs before the art sweep because it is the only step that can change
///    `project.condition_signal`, and the art scene derives from the row it corrects — running it
///    second would render every crashed-out project's card from stale state and then restale it.
/// 2. **The art sweep.** §7.5 demotes renditions whose bitmap is gone, so the shelf asks for a
///    re-render instead of pointing at a missing file.
/// 3. **The git floor, last and non-fatally.** It spawns a process, so it is the one step that
///    can be slow or fail environmentally. A missing or too-old git is a drawn window (§11.2a),
///    never a refusal to start — the app has plenty to show from the index alone.
pub fn run_startup(handler: &mut CoreHandler) -> StartupSummary {
    let mut summary = StartupSummary::default();
    let now = handler.clock.now_unix();

    // 1. Orphan closure.
    if let Some(sessions) = handler.sessions.as_mut() {
        let mut guard = handler.index.lock().unwrap_or_else(PoisonError::into_inner);
        let mut ctx = crate::commands::launch::LaunchCtx {
            index: &mut guard,
            sessions,
            spawner: handler.spawner.as_ref(),
            events: handler.events.as_ref(),
            mounts: handler.mount.as_ref(),
            now,
        };
        match crate::commands::launch::startup(&mut ctx) {
            Ok(report) => summary.orphans = Some(report),
            Err(e) => {
                drop(guard);
                emit_startup_error(handler.events.as_ref(), "orphan closure", &e.message);
            }
        }
    }

    // 2. The art sweep.
    {
        let guard = handler.index.lock().unwrap_or_else(PoisonError::into_inner);
        let ctx = crate::art::ArtCtx {
            index: &guard,
            events: handler.events.as_ref(),
            now,
        };
        match crate::art::startup(&ctx) {
            Ok(report) => summary.art = Some(report),
            Err(e) => {
                drop(guard);
                emit_startup_error(handler.events.as_ref(), "art sweep", &e.to_string());
            }
        }
    }

    // 3. The git floor. No index lock is held across it — it spawns a process, and R39's rule is
    // that no arm holds the lock across a git invocation.
    let cancel = crate::cancel::CancelToken::new();
    let ctx = crate::git::JobContext::new(crate::git::JobClass::Interactive, &cancel, None);
    match crate::git::require_floor(handler.git.as_ref(), &ctx) {
        Ok(version) => {
            // Recorded where `CoreSnapshot.gitVersion` reads it, so the figure has one owner.
            let guard = handler.index.lock().unwrap_or_else(PoisonError::into_inner);
            let _ = guard.set_app_meta("git_version", &version.raw);
            drop(guard);
            summary.git = Some(version);
        }
        Err(e) => emit_startup_error(handler.events.as_ref(), "git floor", &e.to_string()),
    }

    // 4. The identity set, which nothing else seeds.
    {
        let emails = crate::identity::gitconfig::user_emails(
            &handler.firstrun.sources.home,
            handler.firstrun.sources.xdg_config.as_deref(),
        );
        let guard = handler.index.lock().unwrap_or_else(PoisonError::into_inner);
        match crate::identity::people::seed(guard.conn(), &emails, now) {
            Ok(report) => summary.identity = Some(report),
            Err(e) => {
                drop(guard);
                emit_startup_error(handler.events.as_ref(), "identity seed", &e.to_string());
            }
        }
    }

    summary
}

/// A startup step that failed is a `core/error` event and a `None` field, never a panic and
/// never a silent skip. The message is diagnostic (§2.4) and is not rendered.
fn emit_startup_error(events: &dyn EventSink, step: &str, message: &str) {
    events.emit(
        "core",
        "error",
        serde_json::json!({
            "code": "INTERNAL",
            "message": format!("startup: {step}: {message}"),
        }),
    );
}
