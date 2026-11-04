//! A whole [`CoreHandler`] over the fakes, with the scan launcher the test chooses — so a test
//! can drive the loop (`handle`, `pump`, `snapshot`) through a walk it holds open.

use std::path::Path;
use std::sync::{Arc, Mutex};

use crate::assembly::{CoreDeps, CoreHandler};
use crate::index::Index;
use crate::proto::pubsub::{Publisher, PublisherSink};
use crate::scan::ScanLauncher;
use crate::testing::zone::FixedZone;
use crate::testing::FakeClock;

/// What [`core_handler`] builds: the handler and the two things a test observes it through.
pub struct CoreHandlerParts {
    /// The handler, over `index` and the fakes.
    pub handler: CoreHandler,
    /// The handler's publisher, detached: whatever it queues stays queued for the test to read.
    pub events: Arc<PublisherSink>,
    /// The clock every `now` the handler stamps is read from.
    pub clock: Arc<FakeClock>,
}

impl std::fmt::Debug for CoreHandlerParts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CoreHandlerParts").finish_non_exhaustive()
    }
}

/// A `CoreHandler` over `index` and the fakes, starting scans through `launcher`.
///
/// Its clock starts at `now`, in UTC. The launcher is passed in rather than built here because
/// a launcher that writes `scan_run` rows needs the same one connection the handler uses.
#[must_use]
pub fn core_handler(
    dir: &Path,
    index: &Arc<Mutex<Index>>,
    now: i64,
    launcher: Arc<dyn ScanLauncher>,
) -> CoreHandlerParts {
    let events = Arc::new(PublisherSink::new(Publisher::detached()));
    let clock = Arc::new(FakeClock::new(now));
    let http: Arc<dyn crate::http::HttpTransport> = Arc::new(crate::testing::FakeTransport::new());
    let sync_observing = Arc::new(crate::sync::http::ObservingTransport::new(
        Arc::clone(&http),
        clock.clone(),
    ));
    let sync_http: Arc<dyn crate::http::HttpTransport> = sync_observing.clone();
    let handler = CoreHandler::new(CoreDeps {
        index: Arc::clone(index),
        provider: Arc::new(crate::provider::GitHubProvider::new(
            Arc::clone(&http),
            crate::provider::listing::GITHUB_CANONICAL_HOST.to_owned(),
        )),
        tokens: Arc::new(crate::testing::FakeTokenStore::unavailable()),
        http,
        client_id: "test-client-id".to_owned(),
        clock: clock.clone(),
        git: Arc::new(crate::testing::FakeGitBackend::new()),
        write_git: Arc::new(crate::testing::FakeMutatingGit::new(
            crate::testing::CloneBehaviour::Succeed,
        )),
        trash: Arc::new(crate::testing::CountingTrash::new()),
        mount: Arc::new(crate::testing::FakeMountResolver::default()),
        spawner: Box::new(crate::launch::spawn::RecordingSpawner::new()),
        sessions: crate::session::manager::SessionManager::new(
            clock.clone(),
            events.clone(),
            Box::new(crate::session::watch::FakeActivitySource::new()),
            Arc::new(crate::session::activity::FakeIgnoreCheck::new(&[])),
            Arc::new(FixedZone(0)),
        ),
        scans: crate::scan::ScanSupervisor::new(launcher),
        scan_store: Arc::new(crate::testing::MemScanStore::new()),
        firstrun: crate::firstrun::FirstRunEnv {
            sources: crate::firstrun::sources::SourceEnv {
                home: dir.to_path_buf(),
                app_data: None,
                xdg_config: None,
            },
            classifier: Arc::new(crate::firstrun::classify::FixedClassifier::new(vec![])),
            distros: Arc::new(crate::firstrun::classify::NoDistros),
            platform: crate::index::path::PathPlatform::Unix,
            skip: crate::scan::skiplist::SkipList::default(),
            cache: crate::firstrun::roots::SuggestionCache::new(),
        },
        jobs: crate::assembly::jobs::JobPump::start(
            Arc::clone(index),
            Arc::new(crate::testing::FakeGitBackend::new()),
            clock.clone(),
            events.clone(),
            Arc::new(FixedZone(0)),
        ),
        sync: crate::assembly::sync::SyncPump::start(
            Arc::clone(index),
            crate::sync::SyncDeps {
                provider: Arc::new(crate::provider::GitHubProvider::new(
                    Arc::clone(&sync_http),
                    crate::provider::listing::GITHUB_CANONICAL_HOST.to_owned(),
                )),
                transport: sync_observing,
                tokens: Arc::new(crate::testing::FakeTokenStore::unavailable()),
                clock: clock.clone(),
                cancel: crate::cancel::CancelToken::new(),
                zone: Arc::new(FixedZone(0)),
            },
            events.clone(),
        ),
        events: Arc::clone(&events),
        tz_offset_min: 0,
        zone: Arc::new(FixedZone(0)),
    });
    CoreHandlerParts {
        handler,
        events,
        clock,
    }
}
