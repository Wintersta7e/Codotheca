#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::{Duration, Instant};

use codotheca_core::accounts::device::{poll_once, request_device_code, GITHUB_CLIENT_ID};
use codotheca_core::accounts::keychain::TokenStore;
use codotheca_core::accounts::pump::{
    ConnectPump, ConnectPumpDeps, ConnectSink, GrantedToken, RecordingConnectSink,
};
use codotheca_core::clock::Clock;
use codotheca_core::http::{HttpRequest, HttpResponse, HttpTransport, TransportError};
use codotheca_core::proto::EventSink;
use codotheca_core::protocol::ConnectStage;
use codotheca_core::provider::scopes::SCOPES_PUBLIC;
use codotheca_core::testing::{FakeClock, FakeTokenStore, FakeTransport};
use serde_json::json;

const HOST: &str = "example.invalid";
const CLIENT_ID: &str = "client-id-for-tests";
const DEVICE_SENTINEL: &str = "device-secret-sentinel";
const ACCESS_SENTINEL: &str = "access-secret-sentinel";
const TOKEN_REF: &str = "test:example.invalid:octo";
const SLICE_MS: u64 = 250;

#[test]
fn the_poll_is_core_side_and_the_code_is_verbatim() {
    let code = "UI-SHOULD-NOT-ASSUME-EIGHT-CHARS";
    let transport = Arc::new(FakeTransport::new());
    transport.push(response(&json!({
        "device_code": DEVICE_SENTINEL,
        "user_code": code,
        "verification_uri": "https://example.invalid/device",
        "expires_in": 60,
        "interval": 2
    })));
    transport.push(response(&json!({"error": "authorization_pending"})));
    transport.push(response(&json!({"error": "slow_down", "interval": 9})));
    transport.push(response(&json!({"error": "authorization_pending"})));
    transport.push(response(&json!({
        "access_token": ACCESS_SENTINEL,
        "scope": "alpha,beta gamma,,",
        "token_type": "bearer"
    })));

    let clock = Arc::new(StepClock::new(1_000));
    let events = Arc::new(RecordingEvents::new(Arc::clone(&clock) as Arc<dyn Clock>));
    let tokens = Arc::new(FakeTokenStore::available());
    let sink = Arc::new(StoreGrantSink::new(TOKEN_REF));
    let pump = ConnectPump::start(deps(
        Arc::clone(&transport) as Arc<dyn HttpTransport>,
        Arc::clone(&clock) as Arc<dyn Clock>,
        Arc::clone(&events) as Arc<dyn EventSink>,
        Arc::clone(&tokens) as Arc<dyn TokenStore>,
        sink,
    ));

    clock.wait_for_sleeps(1);
    let first = pump.grant(clock.now_unix()).expect("flow is live");
    clock.advance(3);
    let second = pump
        .grant(clock.now_unix())
        .expect("same flow is still live");
    assert_eq!(first.user_code, code);
    assert_eq!(second.user_code, code);
    assert!(
        code.len() != 8,
        "the fixture must catch fixed-width UI assumptions"
    );
    assert!(
        second.expires_in_secs < first.expires_in_secs,
        "re-entry must report the remaining deadline, not the original duration"
    );
    assert_eq!(
        device_code_request_count(&transport.requests()),
        1,
        "a second connect while pending must not issue a second device-code request"
    );

    clock.permit_sleeps(slices_for(2));
    events.wait_for_stage(ConnectStage::Pending, 2);
    clock.permit_sleeps(slices_for(2));
    events.wait_for_stage(ConnectStage::SlowDown, 1);
    let slow_down_at = events
        .first_stage_at(ConnectStage::SlowDown)
        .expect("slow down");

    clock.permit_sleeps(slices_for(9));
    events.wait_for_stage(ConnectStage::Pending, 3);
    let replacement_pending_at = events
        .pending_after(slow_down_at)
        .expect("pending after replacement interval");
    assert_eq!(
        replacement_pending_at - slow_down_at,
        9,
        "slow_down must replace the poll interval with the server's interval"
    );

    clock.permit_sleeps(slices_for(9));
    events.wait_for_stage(ConnectStage::Granted, 1);
    pump.stop();

    assert!(tokens.holds(TOKEN_REF, ACCESS_SENTINEL));
    assert_eq!(
        device_code_request_count(&transport.requests()),
        1,
        "the whole pump run must have one device-code request"
    );
}

#[test]
fn terminal_oauth_errors_emit_their_reason() {
    let expired = run_one_terminal_error("expired_token");
    assert!(expired.saw_stage(ConnectStage::Expired));

    let denied = run_one_terminal_error("access_denied");
    assert!(denied.saw_stage(ConnectStage::Denied));
}

#[test]
fn cancel_prevents_a_late_success_token_from_being_stored() {
    let transport = Arc::new(DelayedSuccessTransport::new());
    let clock = Arc::new(StepClock::new(2_000));
    let events = Arc::new(RecordingEvents::new(Arc::clone(&clock) as Arc<dyn Clock>));
    let tokens = Arc::new(FakeTokenStore::available());
    let sink = Arc::new(StoreGrantSink::new(TOKEN_REF));
    let pump = ConnectPump::start(deps(
        Arc::clone(&transport) as Arc<dyn HttpTransport>,
        Arc::clone(&clock) as Arc<dyn Clock>,
        Arc::clone(&events) as Arc<dyn EventSink>,
        Arc::clone(&tokens) as Arc<dyn TokenStore>,
        sink,
    ));

    clock.wait_for_sleeps(1);
    clock.permit_sleeps(slices_for(1));
    transport.wait_for_poll();
    pump.cancel();
    transport.release_success();
    events.wait_for_stage(ConnectStage::Cancelled, 1);
    pump.stop();

    assert!(
        tokens.entry_names().is_empty(),
        "a success that arrives after cancel must not be stored"
    );
}

#[test]
fn empty_client_id_fails_before_any_request() {
    let transport = FakeTransport::new();
    let error = request_device_code(&transport, "github.com", GITHUB_CLIENT_ID, SCOPES_PUBLIC, 0)
        .expect_err("empty client id fails");

    // Case-insensitively: the reason has to be *named*, and pinning its capitalisation would
    // make a copy edit a test failure.
    assert!(
        error
            .to_string()
            .to_ascii_lowercase()
            .contains("no oauth client id"),
        "the refusal must name its reason: {error}"
    );
    assert_eq!(
        transport.request_count(),
        0,
        "an unregistered application must not reach the wire"
    );
}

#[test]
fn stop_interrupts_the_wait_slice() {
    let transport = Arc::new(FakeTransport::new());
    transport.push(response(&json!({
        "device_code": DEVICE_SENTINEL,
        "user_code": "STOP-CODE",
        "verification_uri": "https://example.invalid/device",
        "expires_in": 60,
        "interval": 5
    })));
    let clock = Arc::new(NotifyingRealClock::new(3_000));
    let events = Arc::new(RecordingEvents::new(Arc::clone(&clock) as Arc<dyn Clock>));
    let tokens = Arc::new(FakeTokenStore::available());
    let pump = ConnectPump::start(deps(
        Arc::clone(&transport) as Arc<dyn HttpTransport>,
        Arc::clone(&clock) as Arc<dyn Clock>,
        Arc::clone(&events) as Arc<dyn EventSink>,
        Arc::clone(&tokens) as Arc<dyn TokenStore>,
        Arc::new(RecordingConnectSink::new()),
    ));

    clock.wait_for_sleep();
    let (tx, rx) = std::sync::mpsc::channel();
    let stopper = pump.clone();
    std::thread::spawn(move || {
        stopper.stop();
        tx.send(()).expect("stop result sends");
    });
    rx.recv_timeout(Duration::from_millis(900))
        .expect("stop returns inside one short wait slice");
}

#[test]
fn secrets_never_enter_errors_or_events() {
    let mut strings = Vec::new();
    strings.push(error_from_bad_poll().to_string());

    let transport = Arc::new(FakeTransport::new());
    transport.push(response(&json!({
        "device_code": DEVICE_SENTINEL,
        "user_code": "VISIBLE-CODE",
        "verification_uri": "https://example.invalid/device",
        "expires_in": 10,
        "interval": 1
    })));
    transport.push(response(&json!({
        "access_token": ACCESS_SENTINEL,
        "scope": "one,two three"
    })));
    let clock = Arc::new(FakeClock::new(4_000));
    let events = Arc::new(RecordingEvents::new(Arc::clone(&clock) as Arc<dyn Clock>));
    let tokens = Arc::new(FakeTokenStore::available());
    let pump = ConnectPump::start(deps(
        Arc::clone(&transport) as Arc<dyn HttpTransport>,
        Arc::clone(&clock) as Arc<dyn Clock>,
        Arc::clone(&events) as Arc<dyn EventSink>,
        Arc::clone(&tokens) as Arc<dyn TokenStore>,
        Arc::new(StoreGrantSink::new(TOKEN_REF)),
    ));
    events.wait_for_stage(ConnectStage::Granted, 1);
    pump.stop();

    strings.extend(events.serialised());
    for text in strings {
        assert!(
            !text.contains(DEVICE_SENTINEL),
            "device code leaked into observable text: {text}"
        );
        assert!(
            !text.contains(ACCESS_SENTINEL),
            "access token leaked into observable text: {text}"
        );
    }
}

#[must_use]
fn deps(
    transport: Arc<dyn HttpTransport>,
    clock: Arc<dyn Clock>,
    events: Arc<dyn EventSink>,
    tokens: Arc<dyn TokenStore>,
    sink: Arc<dyn ConnectSink>,
) -> ConnectPumpDeps {
    ConnectPumpDeps {
        transport,
        clock,
        events,
        tokens,
        sink,
        host: HOST.to_owned(),
        client_id: CLIENT_ID.to_owned(),
        scopes: SCOPES_PUBLIC,
    }
}

#[must_use]
fn response(body: &serde_json::Value) -> HttpResponse {
    HttpResponse {
        status: 200,
        headers: Vec::new(),
        body: serde_json::to_vec(&body).expect("json encodes"),
    }
}

#[must_use]
fn device_code_request_count(requests: &[HttpRequest]) -> usize {
    requests
        .iter()
        .filter(|request| request.url.ends_with("/login/device/code"))
        .count()
}

#[must_use]
fn slices_for(secs: u64) -> usize {
    usize::try_from(secs.saturating_mul(1_000) / SLICE_MS).expect("slice count fits")
}

#[must_use]
fn run_one_terminal_error(error: &str) -> Arc<RecordingEvents> {
    let transport = Arc::new(FakeTransport::new());
    transport.push(response(&json!({
        "device_code": DEVICE_SENTINEL,
        "user_code": "TERMINAL-CODE",
        "verification_uri": "https://example.invalid/device",
        "expires_in": 10,
        "interval": 1
    })));
    transport.push(response(&json!({"error": error})));
    let clock = Arc::new(FakeClock::new(1_000));
    let events = Arc::new(RecordingEvents::new(Arc::clone(&clock) as Arc<dyn Clock>));
    let tokens = Arc::new(FakeTokenStore::available());
    let pump = ConnectPump::start(deps(
        transport,
        Arc::clone(&clock) as Arc<dyn Clock>,
        Arc::clone(&events) as Arc<dyn EventSink>,
        tokens,
        Arc::new(RecordingConnectSink::new()),
    ));
    events.wait_for_stage(stage_for_error(error), 1);
    pump.stop();
    events
}

#[must_use]
fn stage_for_error(error: &str) -> ConnectStage {
    match error {
        "expired_token" => ConnectStage::Expired,
        "access_denied" => ConnectStage::Denied,
        other => panic!("unexpected terminal error fixture: {other}"),
    }
}

#[must_use]
fn error_from_bad_poll() -> codotheca_core::accounts::device::ConnectError {
    let transport = FakeTransport::new();
    transport.push(response(&json!({
        "device_code": DEVICE_SENTINEL,
        "user_code": "VISIBLE-CODE",
        "verification_uri": "https://example.invalid/device",
        "expires_in": 10,
        "interval": 1
    })));
    transport.push(HttpResponse {
        status: 400,
        headers: Vec::new(),
        body: serde_json::to_vec(&json!({
            "error": "bad_verification_code",
            "error_description": format!("{DEVICE_SENTINEL} {ACCESS_SENTINEL}")
        }))
        .expect("json encodes"),
    });
    let flow = request_device_code(&transport, HOST, CLIENT_ID, SCOPES_PUBLIC, 4_000)
        .expect("device flow starts");
    poll_once(&transport, HOST, CLIENT_ID, &flow).expect_err("bad poll fails")
}

#[derive(Debug)]
struct StoreGrantSink {
    token_ref: String,
    grants: Mutex<Vec<Vec<String>>>,
}

impl StoreGrantSink {
    #[must_use]
    fn new(token_ref: &str) -> Self {
        Self {
            token_ref: token_ref.to_owned(),
            grants: Mutex::new(Vec::new()),
        }
    }
}

impl ConnectSink for StoreGrantSink {
    fn connect_granted(
        &self,
        tokens: &dyn TokenStore,
        grant: GrantedToken,
    ) -> Result<(), codotheca_core::accounts::pump::ConnectSinkError> {
        tokens
            .store(&self.token_ref, &grant.token)
            .map_err(
                |error| codotheca_core::accounts::pump::ConnectSinkError::Refused {
                    reason: error.to_string(),
                },
            )?;
        self.grants
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(grant.scopes);
        Ok(())
    }
}

#[derive(Debug)]
struct StepClock {
    unix_secs: AtomicI64,
    mono_ms: AtomicU64,
    state: Mutex<StepState>,
    slept: Condvar,
    permits: Condvar,
}

#[derive(Debug, Default)]
struct StepState {
    sleeps: Vec<Duration>,
    permits: usize,
}

impl StepClock {
    #[must_use]
    fn new(unix_secs: i64) -> Self {
        Self {
            unix_secs: AtomicI64::new(unix_secs),
            mono_ms: AtomicU64::new(0),
            state: Mutex::new(StepState::default()),
            slept: Condvar::new(),
            permits: Condvar::new(),
        }
    }

    fn advance(&self, secs: i64) {
        self.unix_secs.fetch_add(secs, Ordering::SeqCst);
        self.mono_ms.fetch_add(
            u64::try_from(secs.saturating_mul(1_000)).unwrap_or(0),
            Ordering::SeqCst,
        );
    }

    fn advance_ms(&self, ms: u64) {
        let previous = self.mono_ms.fetch_add(ms, Ordering::SeqCst);
        let elapsed_secs = previous.saturating_add(ms) / 1_000 - previous / 1_000;
        self.unix_secs.fetch_add(
            i64::try_from(elapsed_secs).unwrap_or(i64::MAX),
            Ordering::SeqCst,
        );
    }

    fn wait_for_sleeps(&self, count: usize) {
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        while state.sleeps.len() < count {
            let now = Instant::now();
            assert!(now < deadline, "timed out waiting for {count} sleep(s)");
            let timeout = deadline.saturating_duration_since(now);
            state = self
                .slept
                .wait_timeout(state, timeout)
                .expect("sleep condvar waits")
                .0;
        }
    }

    fn permit_sleeps(&self, count: usize) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.permits = state.permits.saturating_add(count);
        self.permits.notify_all();
    }
}

impl Clock for StepClock {
    fn now_unix(&self) -> i64 {
        self.unix_secs.load(Ordering::SeqCst)
    }

    fn monotonic_ms(&self) -> u64 {
        self.mono_ms.load(Ordering::SeqCst)
    }

    fn sleep(&self, dur: Duration) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.sleeps.push(dur);
        self.slept.notify_all();
        while state.permits == 0 {
            state = self.permits.wait(state).expect("sleep permit waits");
        }
        state.permits -= 1;
        drop(state);
        self.advance_ms(u64::try_from(dur.as_millis()).unwrap_or(u64::MAX));
    }
}

#[derive(Debug)]
struct NotifyingRealClock {
    unix_secs: AtomicI64,
    mono_ms: AtomicU64,
    sleeps: Mutex<usize>,
    slept: Condvar,
}

impl NotifyingRealClock {
    #[must_use]
    fn new(unix_secs: i64) -> Self {
        Self {
            unix_secs: AtomicI64::new(unix_secs),
            mono_ms: AtomicU64::new(0),
            sleeps: Mutex::new(0),
            slept: Condvar::new(),
        }
    }

    fn wait_for_sleep(&self) {
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut sleeps = self.sleeps.lock().unwrap_or_else(PoisonError::into_inner);
        while *sleeps == 0 {
            let now = Instant::now();
            assert!(now < deadline, "timed out waiting for the pump to sleep");
            let timeout = deadline.saturating_duration_since(now);
            sleeps = self
                .slept
                .wait_timeout(sleeps, timeout)
                .expect("sleep condvar waits")
                .0;
        }
    }
}

impl Clock for NotifyingRealClock {
    fn now_unix(&self) -> i64 {
        self.unix_secs.load(Ordering::SeqCst)
    }

    fn monotonic_ms(&self) -> u64 {
        self.mono_ms.load(Ordering::SeqCst)
    }

    fn sleep(&self, dur: Duration) {
        let mut sleeps = self.sleeps.lock().unwrap_or_else(PoisonError::into_inner);
        *sleeps = sleeps.saturating_add(1);
        self.slept.notify_all();
        drop(sleeps);
        std::thread::sleep(dur);
        self.mono_ms.fetch_add(
            u64::try_from(dur.as_millis()).unwrap_or(u64::MAX),
            Ordering::SeqCst,
        );
    }
}

#[derive(Debug)]
struct RecordedEvent {
    topic: String,
    event: String,
    payload: serde_json::Value,
    at: i64,
}

#[derive(Debug)]
struct RecordingEvents {
    clock: Arc<dyn Clock>,
    events: Mutex<Vec<RecordedEvent>>,
    changed: Condvar,
}

impl RecordingEvents {
    #[must_use]
    fn new(clock: Arc<dyn Clock>) -> Self {
        Self {
            clock,
            events: Mutex::new(Vec::new()),
            changed: Condvar::new(),
        }
    }

    fn wait_for_stage(&self, stage: ConnectStage, count: usize) {
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut events = self.events.lock().unwrap_or_else(PoisonError::into_inner);
        while count_stage(&events, stage) < count {
            let now = Instant::now();
            assert!(
                now < deadline,
                "timed out waiting for stage {stage:?}; events so far: {events:?}"
            );
            let timeout = deadline.saturating_duration_since(now);
            events = self
                .changed
                .wait_timeout(events, timeout)
                .expect("event condvar waits")
                .0;
        }
    }

    #[must_use]
    fn saw_stage(&self, stage: ConnectStage) -> bool {
        count_stage(
            &self.events.lock().unwrap_or_else(PoisonError::into_inner),
            stage,
        ) > 0
    }

    #[must_use]
    fn first_stage_at(&self, stage: ConnectStage) -> Option<i64> {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .find(|event| event_stage(event) == Some(stage))
            .map(|event| event.at)
    }

    #[must_use]
    fn pending_after(&self, after: i64) -> Option<i64> {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .find(|event| event_stage(event) == Some(ConnectStage::Pending) && event.at > after)
            .map(|event| event.at)
    }

    #[must_use]
    fn serialised(&self) -> Vec<String> {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .map(|event| {
                serde_json::to_string(&json!({
                    "topic": event.topic,
                    "event": event.event,
                    "payload": event.payload
                }))
                .expect("event serialises")
            })
            .collect()
    }
}

impl EventSink for RecordingEvents {
    fn emit(&self, topic: &str, event: &str, payload: serde_json::Value) {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(RecordedEvent {
                topic: topic.to_owned(),
                event: event.to_owned(),
                payload,
                at: self.clock.now_unix(),
            });
        self.changed.notify_all();
    }
}

#[must_use]
fn count_stage(events: &[RecordedEvent], stage: ConnectStage) -> usize {
    events
        .iter()
        .filter(|event| event_stage(event) == Some(stage))
        .count()
}

#[must_use]
fn event_stage(event: &RecordedEvent) -> Option<ConnectStage> {
    if event.topic != "accounts" || event.event != "connect_progress" {
        return None;
    }
    serde_json::from_value(
        event
            .payload
            .get("stage")
            .cloned()
            .unwrap_or(serde_json::Value::Null),
    )
    .ok()
}

#[derive(Debug)]
struct DelayedSuccessTransport {
    sent: Mutex<Vec<HttpRequest>>,
    poll_started: Mutex<bool>,
    poll_started_cv: Condvar,
    release: Mutex<bool>,
    release_cv: Condvar,
}

impl DelayedSuccessTransport {
    #[must_use]
    fn new() -> Self {
        Self {
            sent: Mutex::new(Vec::new()),
            poll_started: Mutex::new(false),
            poll_started_cv: Condvar::new(),
            release: Mutex::new(false),
            release_cv: Condvar::new(),
        }
    }

    fn wait_for_poll(&self) {
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut started = self
            .poll_started
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        while !*started {
            let now = Instant::now();
            assert!(now < deadline, "timed out waiting for a poll request");
            let timeout = deadline.saturating_duration_since(now);
            started = self
                .poll_started_cv
                .wait_timeout(started, timeout)
                .expect("poll condvar waits")
                .0;
        }
    }

    fn release_success(&self) {
        *self.release.lock().unwrap_or_else(PoisonError::into_inner) = true;
        self.release_cv.notify_all();
    }
}

impl HttpTransport for DelayedSuccessTransport {
    fn send(&self, req: &HttpRequest) -> Result<HttpResponse, TransportError> {
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(req.clone());
        if req.url.ends_with("/login/device/code") {
            return Ok(response(&json!({
                "device_code": DEVICE_SENTINEL,
                "user_code": "CANCEL-CODE",
                "verification_uri": "https://example.invalid/device",
                "expires_in": 10,
                "interval": 1
            })));
        }

        *self
            .poll_started
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = true;
        self.poll_started_cv.notify_all();
        let mut release = self.release.lock().unwrap_or_else(PoisonError::into_inner);
        while !*release {
            release = self.release_cv.wait(release).expect("release waits");
        }
        Ok(response(&json!({
            "access_token": ACCESS_SENTINEL,
            "scope": "one two"
        })))
    }
}

// ---------------------------------------------------------------------------
// The production ConnectSink. R49: the trait and its real implementation land together, and the
// real one is exercised — a production impl that only compiles is half the defect.
// ---------------------------------------------------------------------------

/// §20's ordering rule, which is the whole of the safety here: **read the viewer, then the
/// keychain, then the row.** A failed keychain store must leave no `account` row behind — a row
/// whose `token_ref` names an entry that does not exist reads to every later caller as a
/// connected account with an unreadable token.
#[test]
fn the_production_sink_writes_the_keychain_before_the_row() {
    let dir = tempfile::tempdir().expect("tmp");
    let index = Arc::new(Mutex::new(
        codotheca_core::index::Index::open_at(dir.path(), 1_000).expect("index opens"),
    ));
    let transport = Arc::new(FakeTransport::new());
    transport.push(response(
        &json!({ "login": "octo", "name": "Octo Fixture" }),
    ));
    let provider: Arc<dyn codotheca_core::provider::Provider> =
        Arc::new(codotheca_core::provider::GitHubProvider::new(
            Arc::clone(&transport) as Arc<dyn HttpTransport>,
            codotheca_core::provider::listing::GITHUB_CANONICAL_HOST.to_owned(),
        ));
    let sink = codotheca_core::accounts::pump::IndexConnectSink::new(
        Arc::clone(&index),
        provider,
        codotheca_core::protocol::ScopeTier::Public,
    );

    let tokens = FakeTokenStore::available();
    sink.connect_granted(
        &tokens,
        GrantedToken {
            host: "forge.example.invalid".to_owned(),
            token: codotheca_core::accounts::keychain::SecretToken::new(ACCESS_SENTINEL.to_owned()),
            scopes: vec!["read:user".to_owned(), "an:invented:scope".to_owned()],
            granted_at: 2_000,
        },
    )
    .expect("a granted token is recorded");

    let entry = "github:forge.example.invalid:octo";
    assert_eq!(tokens.entry_names(), [entry.to_owned()]);
    assert!(tokens.holds(entry, ACCESS_SENTINEL));

    let guard = index.lock().unwrap_or_else(PoisonError::into_inner);
    let (login, token_ref, scopes, kind): (String, String, String, String) = guard
        .conn()
        .query_row(
            "SELECT login, token_ref, granted_scopes, auth_kind FROM account",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .expect("exactly one account row");
    assert_eq!(login, "octo");
    assert_eq!(
        token_ref, entry,
        "the row names the keychain entry, not a token"
    );
    assert_eq!(kind, "device");
    // Verbatim from the server, including a scope no source file contains.
    assert!(scopes.contains("an:invented:scope"), "{scopes}");
    assert!(
        !scopes.contains(ACCESS_SENTINEL) && !token_ref.contains(ACCESS_SENTINEL),
        "the token reached the database"
    );
}

#[test]
fn a_failed_keychain_store_writes_no_account_row() {
    let dir = tempfile::tempdir().expect("tmp");
    let index = Arc::new(Mutex::new(
        codotheca_core::index::Index::open_at(dir.path(), 1_000).expect("index opens"),
    ));
    let transport = Arc::new(FakeTransport::new());
    transport.push(response(&json!({ "login": "octo", "name": null })));
    let provider: Arc<dyn codotheca_core::provider::Provider> =
        Arc::new(codotheca_core::provider::GitHubProvider::new(
            Arc::clone(&transport) as Arc<dyn HttpTransport>,
            codotheca_core::provider::listing::GITHUB_CANONICAL_HOST.to_owned(),
        ));
    let sink = codotheca_core::accounts::pump::IndexConnectSink::new(
        Arc::clone(&index),
        provider,
        codotheca_core::protocol::ScopeTier::Public,
    );

    let tokens = FakeTokenStore::refusing_store();
    let outcome = sink.connect_granted(
        &tokens,
        GrantedToken {
            host: "forge.example.invalid".to_owned(),
            token: codotheca_core::accounts::keychain::SecretToken::new(ACCESS_SENTINEL.to_owned()),
            scopes: vec!["read:user".to_owned()],
            granted_at: 2_000,
        },
    );
    assert!(outcome.is_err(), "a refused keychain must fail the connect");

    let guard = index.lock().unwrap_or_else(PoisonError::into_inner);
    let rows: i64 = guard
        .conn()
        .query_row("SELECT count(*) FROM account", [], |row| row.get(0))
        .expect("countable");
    assert_eq!(rows, 0, "the row was written before the keychain succeeded");
}
