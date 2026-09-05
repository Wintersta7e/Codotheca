use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;

use super::device::{self, DeviceFlow, PollOutcome};
use super::keychain::{KeychainError, SecretToken, TokenStore};
use crate::cancel::CancelToken;
use crate::clock::Clock;
use crate::http::HttpTransport;
use crate::proto::EventSink;
use crate::protocol::{ConnectProgress, ConnectStage, DeviceGrant};

const WAIT_SLICE_MS: u64 = 250;

pub struct ConnectPumpDeps {
    pub transport: Arc<dyn HttpTransport>,
    pub clock: Arc<dyn Clock>,
    pub events: Arc<dyn EventSink>,
    pub tokens: Arc<dyn TokenStore>,
    pub sink: Arc<dyn ConnectSink>,
    pub host: String,
    pub client_id: String,
    pub scopes: &'static [&'static str],
}

impl std::fmt::Debug for ConnectPumpDeps {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConnectPumpDeps")
            .field("host", &self.host)
            .field("client_id", &self.client_id)
            .field("scopes", &self.scopes)
            .finish_non_exhaustive()
    }
}

#[derive(Debug)]
pub struct GrantedToken {
    pub host: String,
    pub token: SecretToken,
    pub scopes: Vec<String>,
    pub granted_at: i64,
}

#[derive(Debug, thiserror::Error)]
pub enum ConnectSinkError {
    #[error("the token store refused the grant: {reason}")]
    TokenStore { reason: String },
    #[error("the account sink refused the grant: {reason}")]
    Refused { reason: String },
}

impl ConnectSinkError {
    #[must_use]
    pub fn token_store(error: &KeychainError) -> Self {
        Self::TokenStore {
            reason: error.to_string(),
        }
    }
}

/// Records a completed connection inside the core.
///
/// The production implementation lives in the composition root, where it can store the granted
/// token and record the account without sending the token over the outbound protocol.
pub trait ConnectSink: Send + Sync + std::fmt::Debug {
    /// Stores the granted token and records the account metadata.
    ///
    /// # Errors
    /// Fails when token storage or account recording fails.
    fn connect_granted(
        &self,
        tokens: &dyn TokenStore,
        grant: GrantedToken,
    ) -> Result<(), ConnectSinkError>;
}

#[derive(Clone)]
pub struct ConnectPump {
    inner: Arc<ConnectPumpInner>,
}

impl std::fmt::Debug for ConnectPump {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConnectPump").finish_non_exhaustive()
    }
}

impl ConnectPump {
    /// Starts one core-side device-flow pump.
    #[must_use]
    pub fn start(deps: ConnectPumpDeps) -> ConnectPump {
        let cancel = CancelToken::new();
        let now = deps.clock.now_unix();
        let flow = device::request_device_code(
            deps.transport.as_ref(),
            &deps.host,
            &deps.client_id,
            deps.scopes,
            now,
        );
        let inner = Arc::new(ConnectPumpInner::new(deps, cancel, flow));
        if let Some(interval_secs) = inner.current_interval() {
            inner.emit_progress(ConnectStage::Pending, interval_secs);
            inner.spawn_worker();
        }
        ConnectPump { inner }
    }

    /// The live grant, with `expires_in_secs` computed from the absolute deadline.
    #[must_use]
    pub fn grant(&self, now: i64) -> Option<DeviceGrant> {
        self.inner.grant(now)
    }

    /// Cancels the live flow.
    pub fn cancel(&self) {
        self.inner.cancel_user();
    }

    /// Stop the worker and wait for it.
    ///
    /// Idempotent, with the same outer discipline as `JobPump::stop`: cancel, request stop, join.
    pub fn stop(&self) {
        self.inner.cancel.cancel();
        self.inner.request_stop();
        self.inner.join();
    }
}

struct ConnectPumpInner {
    deps: ConnectPumpDeps,
    cancel: CancelToken,
    state: Mutex<PumpState>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl std::fmt::Debug for ConnectPumpInner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConnectPumpInner")
            .field("deps", &self.deps)
            .field("cancel", &self.cancel)
            .finish_non_exhaustive()
    }
}

#[derive(Debug)]
struct PumpState {
    flow: Option<DeviceFlow>,
    stopping: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Wait {
    Continue,
    Cancelled,
    Expired,
}

impl ConnectPumpInner {
    fn new(
        deps: ConnectPumpDeps,
        cancel: CancelToken,
        flow: Result<DeviceFlow, device::ConnectError>,
    ) -> Self {
        let flow = flow.ok();
        Self {
            deps,
            cancel,
            state: Mutex::new(PumpState {
                flow,
                stopping: false,
            }),
            worker: Mutex::new(None),
        }
    }

    fn spawn_worker(self: &Arc<Self>) {
        let worker = Arc::clone(self);
        if let Ok(handle) = std::thread::Builder::new()
            .name("codotheca-connect".to_owned())
            .spawn(move || worker.poll_loop())
        {
            *lock(&self.worker) = Some(handle);
        }
    }

    fn poll_loop(self: Arc<Self>) {
        loop {
            let Some(interval_secs) = self.current_interval() else {
                return;
            };
            match self.wait_interval(interval_secs) {
                Wait::Continue => {}
                Wait::Cancelled => {
                    self.finish(ConnectStage::Cancelled);
                    return;
                }
                Wait::Expired => {
                    self.finish(ConnectStage::Expired);
                    return;
                }
            }
            if self.flow_expired() {
                self.finish(ConnectStage::Expired);
                return;
            }
            let Some(flow) = self.flow_snapshot() else {
                return;
            };
            match device::poll_once(
                self.deps.transport.as_ref(),
                &self.deps.host,
                &self.deps.client_id,
                &flow,
            ) {
                Ok(outcome) => self.apply_outcome(outcome),
                Err(_) => self.emit_progress(ConnectStage::Pending, flow.interval_secs),
            }
        }
    }

    fn apply_outcome(&self, outcome: PollOutcome) {
        match outcome {
            PollOutcome::Pending => {
                if let Some(interval_secs) = self.current_interval() {
                    self.emit_progress(ConnectStage::Pending, interval_secs);
                }
            }
            PollOutcome::SlowDown { interval_secs } => {
                self.replace_interval(interval_secs);
                self.emit_progress(ConnectStage::SlowDown, interval_secs);
            }
            PollOutcome::Granted(token, scopes) => self.granted(token, scopes),
            PollOutcome::Denied => self.finish(ConnectStage::Denied),
            PollOutcome::Expired => self.finish(ConnectStage::Expired),
        }
    }

    fn granted(&self, token: SecretToken, scopes: Vec<String>) {
        if self.is_stopped() {
            self.finish(ConnectStage::Cancelled);
            return;
        }
        let grant = GrantedToken {
            host: self.deps.host.clone(),
            token,
            scopes,
            granted_at: self.deps.clock.now_unix(),
        };
        if self
            .deps
            .sink
            .connect_granted(self.deps.tokens.as_ref(), grant)
            .is_ok()
            && !self.is_stopped()
        {
            self.finish(ConnectStage::Granted);
        }
    }

    fn grant(&self, now: i64) -> Option<DeviceGrant> {
        let mut expired_interval = None;
        let grant = {
            let state = lock(&self.state);
            let flow = state.flow.as_ref()?;
            if now >= flow.expires_at {
                expired_interval = Some(flow.interval_secs);
                None
            } else {
                Some(DeviceGrant {
                    user_code: flow.user_code.clone(),
                    verification_uri: flow.verification_uri.clone(),
                    expires_in_secs: remaining_secs(flow.expires_at, now),
                    interval_secs: flow.interval_secs,
                })
            }
        };
        if expired_interval.is_some() {
            self.finish(ConnectStage::Expired);
        }
        grant
    }

    fn wait_interval(&self, interval_secs: u32) -> Wait {
        let mut remaining_ms = u64::from(interval_secs).saturating_mul(1_000);
        while remaining_ms > 0 {
            if self.is_stopped() {
                return Wait::Cancelled;
            }
            if self.flow_expired() {
                return Wait::Expired;
            }
            let chunk = remaining_ms.min(WAIT_SLICE_MS);
            self.deps.clock.sleep(Duration::from_millis(chunk));
            remaining_ms = remaining_ms.saturating_sub(chunk);
        }
        Wait::Continue
    }

    fn finish(&self, stage: ConnectStage) {
        let interval_secs = {
            let mut pump_state = lock(&self.state);
            let Some(flow) = pump_state.flow.take() else {
                return;
            };
            flow.interval_secs
        };
        self.emit_progress(stage, interval_secs);
    }

    fn cancel_user(&self) {
        self.cancel.cancel();
        self.request_stop();
        self.finish(ConnectStage::Cancelled);
    }

    fn current_interval(&self) -> Option<u32> {
        lock(&self.state)
            .flow
            .as_ref()
            .map(|flow| flow.interval_secs)
    }

    fn flow_snapshot(&self) -> Option<DeviceFlow> {
        lock(&self.state).flow.clone()
    }

    fn flow_expired(&self) -> bool {
        lock(&self.state)
            .flow
            .as_ref()
            .is_some_and(|flow| self.deps.clock.now_unix() >= flow.expires_at)
    }

    fn replace_interval(&self, interval_secs: u32) {
        if let Some(flow) = lock(&self.state).flow.as_mut() {
            flow.interval_secs = interval_secs;
        }
    }

    fn request_stop(&self) {
        lock(&self.state).stopping = true;
    }

    fn is_stopped(&self) -> bool {
        self.cancel.is_cancelled() || lock(&self.state).stopping
    }

    fn join(&self) {
        let handle = lock(&self.worker).take();
        if let Some(handle) = handle {
            drop(handle.join());
        }
    }

    fn emit_progress(&self, stage: ConnectStage, interval_secs: u32) {
        if let Ok(payload) = serde_json::to_value(ConnectProgress {
            stage,
            interval_secs,
        }) {
            self.deps
                .events
                .emit("accounts", "connect_progress", payload);
        }
    }
}

#[must_use]
fn remaining_secs(expires_at: i64, now: i64) -> u32 {
    u32::try_from(expires_at.saturating_sub(now)).unwrap_or(u32::MAX)
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(feature = "testkit")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedConnectGrant {
    pub host: String,
    pub scopes: Vec<String>,
    pub granted_at: i64,
}

#[cfg(feature = "testkit")]
#[derive(Debug, Default)]
pub struct RecordingConnectSink {
    token_ref: Option<String>,
    grants: Mutex<Vec<RecordedConnectGrant>>,
}

#[cfg(feature = "testkit")]
impl RecordingConnectSink {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn storing(token_ref: String) -> Self {
        Self {
            token_ref: Some(token_ref),
            grants: Mutex::new(Vec::new()),
        }
    }

    #[must_use]
    pub fn grants(&self) -> Vec<RecordedConnectGrant> {
        lock(&self.grants).clone()
    }
}

#[cfg(feature = "testkit")]
impl ConnectSink for RecordingConnectSink {
    fn connect_granted(
        &self,
        tokens: &dyn TokenStore,
        grant: GrantedToken,
    ) -> Result<(), ConnectSinkError> {
        if let Some(token_ref) = self.token_ref.as_deref() {
            tokens
                .store(token_ref, &grant.token)
                .map_err(|error| ConnectSinkError::token_store(&error))?;
        }
        lock(&self.grants).push(RecordedConnectGrant {
            host: grant.host,
            scopes: grant.scopes,
            granted_at: grant.granted_at,
        });
        Ok(())
    }
}

/// The production [`ConnectSink`]: it records a completed connection in the index.
///
/// **It lands in the same change as the trait**, which is R49's rule and four rulings' worth of
/// history — a seam whose only implementation is a fake compiles, passes against that fake, and
/// fails at assembly.
///
/// **Order is normative and it is the whole of §20's safety here: read the viewer, then the
/// keychain, then the row.** A token that does not authenticate must not create an `account`
/// row, and if the keychain store fails **no row is written** — writing it first would leave a
/// row whose `token_ref` names an entry that does not exist, which reads to every later caller
/// as a connected account with an unreadable token.
///
/// **It takes the index lock only around the write that follows the response** (R75). The
/// network call happens on the pump thread before this is reached, so no forge round trip is
/// ever made while the process's one SQLite mutex is held.
pub struct IndexConnectSink {
    index: Arc<Mutex<crate::index::Index>>,
    provider: Arc<dyn crate::provider::Provider>,
    scope_tier: crate::protocol::ScopeTier,
}

impl std::fmt::Debug for IndexConnectSink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IndexConnectSink")
            .field("scope_tier", &self.scope_tier)
            .finish_non_exhaustive()
    }
}

impl IndexConnectSink {
    #[must_use]
    pub fn new(
        index: Arc<Mutex<crate::index::Index>>,
        provider: Arc<dyn crate::provider::Provider>,
        scope_tier: crate::protocol::ScopeTier,
    ) -> Self {
        Self {
            index,
            provider,
            scope_tier,
        }
    }
}

impl ConnectSink for IndexConnectSink {
    fn connect_granted(
        &self,
        tokens: &dyn TokenStore,
        grant: GrantedToken,
    ) -> Result<(), ConnectSinkError> {
        // 1. Read the viewer. This is the network call, and it happens with no lock held.
        let viewer =
            self.provider
                .viewer(&grant.token)
                .map_err(|error| ConnectSinkError::Refused {
                    reason: error.to_string(),
                })?;
        let login = viewer.value.login;
        let display_name = viewer.value.display_name;
        let provider_id = self.provider.canonical_host().to_owned();
        let entry = super::keychain::token_ref(PROVIDER_ID, &grant.host, &login);

        // 2. The keychain, before the row. A failure here writes nothing at all.
        tokens
            .store(&entry, &grant.token)
            .map_err(|error| ConnectSinkError::token_store(&error))?;

        // 3. The row, in one transaction, with the lock taken only now.
        let new = super::store::NewAccount {
            provider: PROVIDER_ID.to_owned(),
            host: if grant.host.is_empty() {
                provider_id
            } else {
                grant.host.clone()
            },
            login,
            display_name,
            auth_kind: crate::protocol::AuthKind::Device,
            scope_tier: self.scope_tier,
            // Verbatim from the server, never a source literal.
            granted_scopes: grant.scopes,
            token_ref: entry.clone(),
        };
        let mut guard = self
            .index
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _tx_guard = crate::proto::txguard::TxGuard::enter();
        let tx = guard
            .conn_mut()
            .transaction()
            .map_err(|error| ConnectSinkError::Refused {
                reason: error.to_string(),
            })?;
        super::store::insert_account(&tx, &new, grant.granted_at).map_err(|error| {
            ConnectSinkError::Refused {
                reason: error.to_string(),
            }
        })?;
        tx.commit().map_err(|error| ConnectSinkError::Refused {
            reason: error.to_string(),
        })?;
        Ok(())
    }
}

/// The adapter id stored in `account.provider`. One forge today; the column carries no CHECK
/// precisely so a second one costs no migration.
const PROVIDER_ID: &str = "github";
