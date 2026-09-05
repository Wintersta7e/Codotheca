//! §20.2's OAuth Device Flow, requested and polled **core-side**.
//!
//! **The build carries no client secret, because none exists to carry.** That is the property
//! that makes Device Flow the only OAuth variant safe to ship in an open-source binary, and
//! `core/tests/accounts_redaction.rs` asserts it rather than assuming it.
//!
//! The `device_code` and the access token are both [`SecretToken`]: neither crosses the protocol
//! outbound, neither reaches the rolling log, and no error built here quotes a response body —
//! every message is composed from the provider's own `error` slug.

use crate::accounts::keychain::SecretToken;
use crate::http::{HttpRequest, HttpResponse, HttpTransport, ACCOUNT_LIMITS};

/// The public client id, compiled in, one per provider. **Not a secret.**
///
/// Empty until the application is registered, which is an external act this build cannot
/// perform. [`request_device_code`] refuses an empty one and issues **no** request: failing
/// loudly beats sending a request that will be refused for a reason the user cannot act on.
pub const GITHUB_CLIENT_ID: &str = "";

/// The default poll interval when the server names none. Seconds, per the Device Flow spec.
const DEFAULT_INTERVAL_SECS: u32 = 5;

/// A live flow. **`expires_at` is an absolute deadline, not a duration**: a re-entry has to
/// report the time that is actually left, and a countdown that restarts on drawer reopen is the
/// surface lying about the deadline.
#[derive(Debug, Clone)]
pub struct DeviceFlow {
    pub device_code: SecretToken,
    /// Carried **byte-identical** from the server. Never reformatted, never masked, never
    /// padded: the "eight-character code" describes what the provider returns today, it is not
    /// a validator, and a surface that hard-codes the shape breaks silently when it changes.
    pub user_code: String,
    pub verification_uri: String,
    pub expires_at: i64,
    pub interval_secs: u32,
}

/// One poll's answer.
///
/// No `PartialEq`: it carries a [`SecretToken`], and comparing two secrets is not an operation
/// this codebase wants to make easy. A test matches on the variant.
#[derive(Debug, Clone)]
pub enum PollOutcome {
    Pending,
    /// **The interval in this response replaces the previous one.** Not the old one plus a
    /// constant, and not the old one kept.
    SlowDown {
        interval_secs: u32,
    },
    /// The token, and the scope set **the response actually named**.
    ///
    /// `None` is *the response carried no `scope` field*, which is unknown. `Some(vec![])` is a
    /// field that was present and empty. Flattening the two writes an empty grant for an account
    /// whose grant was never stated, which renders as "no scopes at all".
    Granted(SecretToken, Option<Vec<String>>),
    Denied,
    Expired,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConnectError {
    /// No application is registered, so no flow can start. Named rather than attempted.
    #[error("no OAuth client id is compiled into this build")]
    NoClientId,
    #[error("the forge could not be reached: {0}")]
    Transport(String),
    /// The provider answered, and its own `error` slug is the whole message.
    ///
    /// **Only a slug-shaped value is quoted**, and `refusal` is what decides that. The slug comes
    /// out of the response body, which is the one place a token could be echoed back, so quoting
    /// it verbatim would have made a hostile or broken server's `error` field a channel out of
    /// this process. An OAuth error slug is lower-case ASCII and underscores; anything else is
    /// reported without its text rather than not reported at all.
    #[error("the forge refused the device flow: {0}")]
    Refused(String),
    #[error("the forge's answer could not be read: {0}")]
    Decode(String),
}

fn form(pairs: &[(&str, &str)]) -> Vec<u8> {
    pairs
        .iter()
        .map(|(k, v)| format!("{}={}", encode(k), encode(v)))
        .collect::<Vec<_>>()
        .join("&")
        .into_bytes()
}

/// Percent-encoding for the handful of characters a scope set or a device code can carry.
fn encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(char::from(byte));
        } else {
            out.push('%');
            out.push(hex_digit(byte >> 4));
            out.push(hex_digit(byte & 0x0f));
        }
    }
    out
}

/// One nibble as an upper-case hex digit. A `match` rather than an index, because
/// `clippy::indexing_slicing` is denied crate-wide and a table lookup here would need an
/// `#[allow]` to say something the match says by construction.
fn hex_digit(nibble: u8) -> char {
    match nibble {
        0..=9 => char::from(b'0' + nibble),
        _ => char::from(b'A' + nibble - 10),
    }
}

fn post(
    transport: &dyn HttpTransport,
    url: String,
    body: Vec<u8>,
) -> Result<HttpResponse, ConnectError> {
    let request = HttpRequest {
        method: "POST",
        url,
        headers: vec![
            ("accept".to_owned(), "application/json".to_owned()),
            (
                "content-type".to_owned(),
                "application/x-www-form-urlencoded".to_owned(),
            ),
            ("user-agent".to_owned(), "codotheca".to_owned()),
        ],
        body: Some(body),
        limits: ACCOUNT_LIMITS,
    };
    transport
        .send(&request)
        .map_err(|e| ConnectError::Transport(e.to_string()))
}

fn decode(response: &HttpResponse) -> Result<serde_json::Value, ConnectError> {
    serde_json::from_slice(&response.body).map_err(|e| ConnectError::Decode(e.to_string()))
}

fn field(value: &serde_json::Value, name: &str) -> Option<String> {
    value.get(name)?.as_str().map(str::to_owned)
}

/// §20.2 step 1. **Refuses an empty client id before building a request.**
///
/// # Errors
/// [`ConnectError::NoClientId`] when no application is registered, and the transport's or the
/// provider's own failure otherwise.
pub fn request_device_code(
    transport: &dyn HttpTransport,
    host: &str,
    client_id: &str,
    scopes: &[&str],
    now: i64,
) -> Result<DeviceFlow, ConnectError> {
    if client_id.is_empty() {
        return Err(ConnectError::NoClientId);
    }
    let body = form(&[("client_id", client_id), ("scope", &scopes.join(" "))]);
    let response = post(transport, format!("https://{host}/login/device/code"), body)?;
    let value = decode(&response)?;

    if let Some(slug) = field(&value, "error") {
        return Err(refusal(&slug));
    }
    let device_code = field(&value, "device_code")
        .ok_or_else(|| ConnectError::Decode("no device_code".to_owned()))?;
    let user_code = field(&value, "user_code")
        .ok_or_else(|| ConnectError::Decode("no user_code".to_owned()))?;
    let verification_uri = field(&value, "verification_uri")
        .ok_or_else(|| ConnectError::Decode("no verification_uri".to_owned()))?;
    let expires_in = value
        .get("expires_in")
        .and_then(serde_json::Value::as_i64)
        .ok_or_else(|| ConnectError::Decode("no expires_in".to_owned()))?;
    let interval_secs = value
        .get("interval")
        .and_then(serde_json::Value::as_u64)
        .and_then(|v| u32::try_from(v).ok())
        .unwrap_or(DEFAULT_INTERVAL_SECS);

    Ok(DeviceFlow {
        device_code: SecretToken::new(device_code),
        user_code,
        verification_uri,
        expires_at: now.saturating_add(expires_in),
        interval_secs,
    })
}

/// §20.2 step 3, one poll.
///
/// # Errors
/// The transport's failure, an undecodable answer, or a provider `error` slug this flow does
/// not model.
pub fn poll_once(
    transport: &dyn HttpTransport,
    host: &str,
    client_id: &str,
    flow: &DeviceFlow,
) -> Result<PollOutcome, ConnectError> {
    let body = form(&[
        ("client_id", client_id),
        ("device_code", flow.device_code.expose()),
        ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
    ]);
    let response = post(
        transport,
        format!("https://{host}/login/oauth/access_token"),
        body,
    )?;
    let value = decode(&response)?;

    if let Some(slug) = field(&value, "error") {
        return Ok(match slug.as_str() {
            "authorization_pending" => PollOutcome::Pending,
            "slow_down" => PollOutcome::SlowDown {
                // The server's replacement value, not the old one and not the old one plus a
                // constant. Falling back to `+5` only when the field is absent.
                interval_secs: value
                    .get("interval")
                    .and_then(serde_json::Value::as_u64)
                    .and_then(|v| u32::try_from(v).ok())
                    .unwrap_or_else(|| flow.interval_secs.saturating_add(DEFAULT_INTERVAL_SECS)),
            },
            "expired_token" => PollOutcome::Expired,
            "access_denied" => PollOutcome::Denied,
            other => return Err(refusal(other)),
        });
    }

    let token = field(&value, "access_token")
        .ok_or_else(|| ConnectError::Decode("neither an error nor a token".to_owned()))?;
    Ok(PollOutcome::Granted(
        SecretToken::new(token),
        field(&value, "scope").as_deref().map(split_scopes),
    ))
}

/// A refusal naming the forge's `error` slug, **only when the slug is one we know**.
///
/// The slug is read out of the response body, and the body is the one place a token could be
/// echoed back — so what reaches an error message is enumerated here rather than trusted. A shape
/// test is not enough and was tried: `access-secret-sentinel` is lower-case with hyphens and
/// passes any such rule, and so would a lower-case hex credential. §19.4's own instruction is to
/// **enumerate the variants rather than grep the source**, and this is that.
///
/// An unrecognised slug is reported **without its text**, which keeps the refusal visible and
/// tells a maintainer to extend the list rather than leaving the user with nothing.
fn refusal(slug: &str) -> ConnectError {
    if KNOWN_REFUSALS.contains(&slug) {
        return ConnectError::Refused(slug.to_owned());
    }
    ConnectError::Refused("an unrecognised refusal, whose text is not quoted".to_owned())
}

/// Every `error` slug RFC 6749 §5.2, RFC 8628 §3.5 and the forge's own Device Flow define.
///
/// `authorization_pending`, `slow_down`, `expired_token` and `access_denied` are handled as poll
/// outcomes before they reach `refusal`; they are listed anyway, because the device-code request
/// can return them too and a list that is not the whole vocabulary invites a second one.
const KNOWN_REFUSALS: &[&str] = &[
    "access_denied",
    "authorization_pending",
    "device_flow_disabled",
    "expired_token",
    "incorrect_client_credentials",
    "incorrect_device_code",
    "invalid_client",
    "invalid_grant",
    "invalid_request",
    "invalid_scope",
    "server_error",
    "slow_down",
    "temporarily_unavailable",
    "unauthorized_client",
    "unsupported_grant_type",
];

/// The grant's own scope set, split on space **and** comma: the Device Flow spec says space and
/// the forge has been observed to use comma, and guessing wrong loses the whole grant.
fn split_scopes(value: &str) -> Vec<String> {
    value
        .split([' ', ','])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}
