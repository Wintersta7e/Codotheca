//! §20.2's device-flow pump: one worker thread per connect.
//!
//! The worker polls the forge and publishes `accounts/connect_progress` until the flow is
//! granted, denied, expired, cancelled or not stored.

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

/// Everything one device-flow pump needs from the composition root.
pub struct ConnectPumpDeps {
    /// The HTTP transport every device-flow request goes through.
    pub transport: Arc<dyn HttpTransport>,
    /// The clock the poll interval is slept on and the deadline is read from.
    pub clock: Arc<dyn Clock>,
    /// Where `accounts/connect_progress` events are published.
    pub events: Arc<dyn EventSink>,
    /// The keychain the granted token is stored in.
    pub tokens: Arc<dyn TokenStore>,
    /// What records the completed connection once a token is granted.
    pub sink: Arc<dyn ConnectSink>,
    /// The forge host the flow runs against.
    pub host: String,
    /// The public OAuth client id; empty means no flow can start.
    pub client_id: String,
    /// The scopes requested, from the provider's fixed scope sets.
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

/// A token the forge granted at the end of a device flow, handed to the [`ConnectSink`].
#[derive(Debug)]
pub struct GrantedToken {
    /// The forge host the flow ran against.
    pub host: String,
    /// The granted access token.
    pub token: SecretToken,
    /// The grant the token response named, or `None` when it named none — which is unknown.
    pub scopes: Option<Vec<String>>,
    /// Unix seconds at which the grant arrived.
    pub granted_at: i64,
}

/// Why a granted token could not be recorded; the pump reports it as R78's `not_stored`.
#[derive(Debug, thiserror::Error)]
pub enum ConnectSinkError {
    /// The keychain refused to store the token.
    #[error("the token store refused the grant: {reason}")]
    TokenStore {
        /// The keychain error's text.
        reason: String,
    },
    /// The viewer read or the account row write failed.
    #[error("the account sink refused the grant: {reason}")]
    Refused {
        /// The provider's or the store's error text.
        reason: String,
    },
}

impl ConnectSinkError {
    /// The keychain's refusal, carried by its text.
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

/// A handle on one live device flow and the worker polling it; clones share the same flow.
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
    pub fn start(deps: ConnectPumpDeps) -> Self {
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
        Self { inner }
    }

    /// The live grant, with `expires_in_secs` computed from the absolute deadline.
    #[must_use]
    pub fn grant(&self, now: i64) -> Option<DeviceGrant> {
        self.inner.grant(now)
    }

    /// Why no flow started, when none did.
    ///
    /// `grant()` answering `None` has four possible causes and they are not interchangeable: an
    /// unregistered application, an unreachable forge, a refusal, and an unreadable answer. A
    /// caller that reports one sentence for all four tells three users the wrong thing.
    #[must_use]
    pub fn start_error(&self) -> Option<&device::ConnectError> {
        self.inner.start_error.as_ref()
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
    /// Why no flow started, when none did. Kept for the caller to report **by name**.
    start_error: Option<device::ConnectError>,
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
        // The reason is kept, not discarded. `flow.ok()` threw away all four `ConnectError`
        // variants, and the caller then reported the same sentence for every one of them — so an
        // offline user was told this build has no OAuth client id compiled in, which is a lie
        // about the product's own build and sends them to fix the wrong thing.
        let (flow, start_error) = match flow {
            Ok(flow) => (Some(flow), None),
            Err(error) => (None, Some(error)),
        };
        Self {
            deps,
            cancel,
            start_error,
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

    fn granted(&self, token: SecretToken, scopes: Option<Vec<String>>) {
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
        match self
            .deps
            .sink
            .connect_granted(self.deps.tokens.as_ref(), grant)
        {
            Ok(()) => {
                if !self.is_stopped() {
                    self.finish(ConnectStage::Granted);
                }
            }
            Err(error) => {
                // The device code has been redeemed and is single-use, so every further poll can
                // only fail. Leaving the flow live spun for the code's whole lifetime and then
                // reported `expired` — a false statement to a user who has just authorised the
                // application and is watching this screen to learn whether it worked.
                //
                // R78 gave that outcome its own terminal stage: the forge granted it and this
                // machine could not store it, which is neither `denied` (the user refusing) nor
                // `expired` (the deadline). The reason travels with it, because which of the two
                // sides refused is the whole of what the user is owed here.
                let reason = error.to_string();
                eprintln!("accounts: the granted token could not be recorded: {reason}");
                self.finish_with(ConnectStage::NotStored, Some(reason));
            }
        }
    }

    fn grant(&self, now: i64) -> Option<DeviceGrant> {
        // One statement, so the state lock is released before `finish` takes it again. The outer
        // `None` is no flow at all; the inner one is a flow past its deadline.
        let grant = lock(&self.state).flow.as_ref().map(|flow| {
            (now < flow.expires_at).then(|| DeviceGrant {
                user_code: flow.user_code.clone(),
                verification_uri: flow.verification_uri.clone(),
                expires_in_secs: remaining_secs(flow.expires_at, now),
                interval_secs: flow.interval_secs,
            })
        })?;
        if grant.is_none() {
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
        self.finish_with(stage, None);
    }

    /// `finish`, with R78's reason. Every stage but `NotStored` passes `None`.
    fn finish_with(&self, stage: ConnectStage, reason: Option<String>) {
        let Some(interval_secs) = lock(&self.state).flow.take().map(|flow| flow.interval_secs)
        else {
            return;
        };
        self.emit_stage(stage, interval_secs, reason);
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
        self.emit_stage(stage, interval_secs, None);
    }

    /// **`reason` is non-null for exactly one stage.** Carrying it on every stage would make an
    /// ordinary poll look like a failure with nothing to say.
    fn emit_stage(&self, stage: ConnectStage, interval_secs: u32, reason: Option<String>) {
        debug_assert!(
            reason.is_none() || stage == ConnectStage::NotStored,
            "only not_stored carries a reason"
        );
        if let Ok(payload) = serde_json::to_value(ConnectProgress {
            stage,
            interval_secs,
            reason,
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

/// One grant as [`RecordingConnectSink`] saw it, without the token.
#[cfg(feature = "testkit")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedConnectGrant {
    /// The forge host the flow ran against.
    pub host: String,
    /// `None` when the token response named no `scope` field — recorded as it arrived, so a test
    /// can tell an unstated grant from an empty one.
    pub scopes: Option<Vec<String>>,
    /// Unix seconds at which the grant arrived.
    pub granted_at: i64,
}

/// A [`ConnectSink`] that records each grant, and stores the token when built by `storing`.
#[cfg(feature = "testkit")]
#[derive(Debug, Default)]
pub struct RecordingConnectSink {
    token_ref: Option<String>,
    grants: Mutex<Vec<RecordedConnectGrant>>,
}

#[cfg(feature = "testkit")]
impl RecordingConnectSink {
    /// A sink that records grants and stores no token.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A sink that also stores each granted token under `token_ref`.
    #[must_use]
    pub const fn storing(token_ref: String) -> Self {
        Self {
            token_ref: Some(token_ref),
            grants: Mutex::new(Vec::new()),
        }
    }

    /// Every grant recorded so far, in arrival order.
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
/// **It carries no requested tier.** The tier is derived from the grant the server returned, so
/// holding the requested one here would be a second opinion about the same value — and the one
/// that is wrong whenever the user grants less than was asked for.
pub struct IndexConnectSink {
    index: Arc<Mutex<crate::index::Index>>,
    provider: Arc<dyn crate::provider::Provider>,
    /// Set when this flow is a **scope upgrade** of an existing account rather than a new
    /// connection. §20.2: the new token replaces the old **in the same keychain entry**, and the
    /// tier, the grant and its observation time are rewritten together — so the row is updated,
    /// never inserted, and `token_ref` is deliberately untouched.
    upgrading: Option<(crate::protocol::AccountId, String)>,
}

impl std::fmt::Debug for IndexConnectSink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IndexConnectSink")
            .field("upgrading", &self.upgrading)
            .finish_non_exhaustive()
    }
}

impl IndexConnectSink {
    /// The sink for a new connection: it inserts a fresh `account` row.
    #[must_use]
    pub fn new(
        index: Arc<Mutex<crate::index::Index>>,
        provider: Arc<dyn crate::provider::Provider>,
    ) -> Self {
        Self {
            index,
            provider,
            upgrading: None,
        }
    }

    /// The same sink, upgrading `account`'s grant in place under its existing `token_ref`.
    #[must_use]
    pub fn upgrading(
        index: Arc<Mutex<crate::index::Index>>,
        provider: Arc<dyn crate::provider::Provider>,
        account: crate::protocol::AccountId,
        token_ref: String,
    ) -> Self {
        Self {
            index,
            provider,
            upgrading: Some((account, token_ref)),
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
        // The tier the **server** granted, never the one the request asked for. The upgrade asks
        // for the private set and the user may grant less; writing `private` because it was
        // requested claims an access the token does not have, which §20.3's two tiers exist to
        // stop. `X-OAuth-Scopes` on the viewer response is the authority because it is read back
        // from the API; the token response's own `scope` field is the fallback; neither present
        // is unknown, and unknown takes the narrower tier rather than the over-claiming one.
        let observed = viewer.granted_scopes.or(grant.scopes);
        let scope_tier = observed.as_deref().map_or(
            crate::protocol::ScopeTier::Public,
            super::commands::tier_for,
        );
        let granted_scopes = observed.unwrap_or_default();
        // An upgrade reuses the account's existing entry name; a new connection composes one.
        let entry = match &self.upgrading {
            Some((_, token_ref)) => token_ref.clone(),
            None => super::keychain::token_ref(PROVIDER_ID, &grant.host, &login),
        };

        // 2. The keychain, before the row. A failure here writes nothing at all.
        tokens
            .store(&entry, &grant.token)
            .map_err(|error| ConnectSinkError::token_store(&error))?;

        // 3. The row, in one transaction, with the lock taken only now.
        if let Some((account, _)) = self.upgrading {
            let mut guard = self.index.lock().unwrap_or_else(PoisonError::into_inner);
            let _tx_guard = crate::proto::txguard::TxGuard::enter();
            let tx = guard
                .conn_mut()
                .transaction()
                .map_err(|error| ConnectSinkError::Refused {
                    reason: error.to_string(),
                })?;
            super::store::record_upgraded_scope(
                &tx,
                account,
                scope_tier,
                &granted_scopes,
                grant.granted_at,
            )
            .map_err(|error| ConnectSinkError::Refused {
                reason: error.to_string(),
            })?;
            let committed = tx.commit().map_err(|error| ConnectSinkError::Refused {
                reason: error.to_string(),
            });
            // `tx` borrowed this guard's connection, so the commit is the earliest release.
            drop(guard);
            return committed;
        }

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
            scope_tier,
            // Verbatim from the server, never a source literal.
            granted_scopes,
            token_ref: entry,
        };
        let mut guard = self.index.lock().unwrap_or_else(PoisonError::into_inner);
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
        // `tx` borrowed this guard's connection, so the commit is the earliest release.
        drop(guard);
        Ok(())
    }
}

/// The adapter id stored in `account.provider`. One forge today; the column carries no CHECK
/// precisely so a second one costs no migration.
const PROVIDER_ID: &str = "github";
