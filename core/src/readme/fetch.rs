//! The remote branch: the URL guard and the bounded fetch.
//!
//! **The core fetches, never the shell.** A shell that gains network code a renderer-supplied URL
//! can aim is a new attack surface in the process that owns the dialogs.
//!
//! **R53: this is not a `SyncTask`, writes no `sync_budget` row, and draws on no account's rate
//! allowance.** The hosts are arbitrary badge and image services, not the forge, and mirroring
//! their headers into a budget keyed `(account_id, resource)` would key a forge pool by a
//! stranger's `x-ratelimit-resource` — a wrong value in the row that decides whether the app
//! burns an account's allowance. It runs over [`crate::http::HttpTransport`] **directly**, with
//! its own [`crate::http::RequestLimits`], and never through §21's `ObservingTransport`, which is
//! R53 expressed in the type system rather than in a comment.
//!
//! **The header constraint is inherited, not re-imposed.** `HttpTransport` adds no
//! `Authorization`, no cookie jar and no `Referer` (`core/src/http/mod.rs:15-20`), and a header
//! set in wave 1 cannot be unset by a consumer in wave 5. This module adds none of its own.

use std::net::IpAddr;

use crate::http::{
    body_cap_detail, HttpRequest, HttpTransport, RequestLimits, TransportError,
    CONNECT_TIMEOUT_SECS,
};
use crate::protocol::ReadmeAssetState;
use crate::readme::assets::{sniff_media_type, ASSET_BYTE_CAP};

/// §25.5's timeout for one asset.
pub const REMOTE_TIMEOUT_SECS: u64 = 5;

/// **Zero, and that is stronger than §25.5 asks for.**
///
/// §25.5 requires that a redirect may not change scheme and that a redirect re-runs the host
/// clause. The transport follows redirects itself so that `redirect_limit` and
/// `allow_scheme_change` stay per-request (`core/src/http/mod.rs:340-342`), and it offers a
/// consumer no hook on each hop — so the host clause below **cannot** be re-run against the host
/// a `Location` names. Refusing every redirect is the shape that makes the escape unreachable
/// rather than asserted: no hop exists to check. The cost is that a badge host answering a `302`
/// renders as a placeholder, which is the same outcome as any other asset this build will not
/// fetch.
pub const REDIRECT_LIMIT: u8 = 0;

/// Resolve a host to its addresses. A function pointer rather than a trait: there is exactly one
/// question to ask, and the production answer is [`system_resolver`].
pub type HostResolver = fn(&str, u16) -> std::io::Result<Vec<IpAddr>>;

/// The production resolver, and the only one the composition root passes.
///
/// # Errors
/// Propagates the platform resolver's failure, which the caller reports as `unreachable`.
pub fn system_resolver(host: &str, port: u16) -> std::io::Result<Vec<IpAddr>> {
    use std::net::ToSocketAddrs as _;
    Ok((host, port)
        .to_socket_addrs()?
        .map(|addr| addr.ip())
        .collect())
}

/// Whether an address is one this fetch may reach.
///
/// Loopback, private (RFC1918 / RFC4193), link-local and unspecified are all refused: a README is
/// content the user may not have written, so an image reference is an attacker-chosen URL and the
/// machine's own network is exactly what it must not be able to aim at.
///
/// The documentation ranges (RFC 5737, RFC 3849) are **not** refused. They are unroutable rather
/// than internal, so refusing them would buy nothing and would make the range reserved for
/// examples unusable as a test fixture — which is the one thing it exists for.
#[must_use]
pub fn is_public_address(addr: IpAddr) -> bool {
    match addr {
        IpAddr::V4(v4) => {
            !(v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                // 100.64.0.0/10, carrier-grade NAT. 169.254.0.0/16 is `is_link_local`, which is
                // the range a cloud metadata endpoint lives in.
                || (v4.octets()[0] == 100 && (64..128).contains(&v4.octets()[1])))
        }
        IpAddr::V6(v6) => {
            !(v6.is_loopback()
                || v6.is_unspecified()
                // RFC4193 unique-local fc00::/7 and RFC4291 link-local fe80::/10. Neither has a
                // stable predicate on stable Rust, so the prefixes are read from the octets.
                || (v6.octets()[0] & 0xfe) == 0xfc
                || (v6.octets()[0] == 0xfe && (v6.octets()[1] & 0xc0) == 0x80))
        }
    }
}

/// The literal half of the host clause: scheme, suffix and an IP literal.
///
/// Pure, and deliberately separate from resolution: this half answers for a host that does not
/// resolve at all, and it is the half a test can assert without a network.
#[must_use]
pub fn is_fetchable_host(url: &reqwest::Url) -> bool {
    if url.scheme() != "https" {
        return false;
    }
    let Some(host) = url.host_str() else {
        return false;
    };
    let lower = host.to_ascii_lowercase();
    // `.local` is mDNS and `.localhost` is the loopback name; both name a machine on this
    // network rather than a host on the internet. The suffix is compared on an already
    // lowercased string, which is what makes clippy's file-extension lint the wrong reading.
    let is_local_name = lower == "localhost"
        || lower.rsplit('.').next() == Some("local")
        || lower.rsplit('.').next() == Some("localhost");
    if is_local_name {
        return false;
    }
    // A bare IP literal is checked here as well as after resolution, because a literal never
    // reaches the resolver at all on some platforms.
    if let Ok(addr) = lower.trim_matches(['[', ']']).parse::<IpAddr>() {
        return is_public_address(addr);
    }
    true
}

/// Fetch one asset, or say which of §25.5's states it landed in.
///
/// Returns the bytes and the media type sniffed **from them** — never from `Content-Type`, which
/// is a claim by the host rather than a fact about the payload.
///
/// # Errors
/// The state to render: `unreachable` for a refused host, a resolver failure, a timeout, a
/// non-200 status or any redirect; `too_large` for a body past [`ASSET_BYTE_CAP`];
/// `not_an_image` for bytes that sniff as nothing this panel renders.
pub fn fetch_remote_asset(
    transport: &dyn HttpTransport,
    url: &reqwest::Url,
    resolve: HostResolver,
) -> Result<(Vec<u8>, &'static str), ReadmeAssetState> {
    if !is_fetchable_host(url) {
        return Err(ReadmeAssetState::Unreachable);
    }
    if let Some(host) = url.host_str() {
        if host.parse::<IpAddr>().is_err() {
            let port = url.port_or_known_default().unwrap_or(443);
            let addrs = resolve(host, port).map_err(|_| ReadmeAssetState::Unreachable)?;
            // An empty answer is not a public one: nothing was established, so nothing is
            // reachable. `all` over an empty set would be true and would wave the host through.
            if addrs.is_empty() || !addrs.iter().copied().all(is_public_address) {
                return Err(ReadmeAssetState::Unreachable);
            }
        }
    }

    let request = HttpRequest {
        method: "GET",
        url: url.to_string(),
        // Empty, and that is the whole header policy: the transport adds none either.
        headers: Vec::new(),
        body: None,
        limits: RequestLimits {
            connect_secs: CONNECT_TIMEOUT_SECS,
            total_secs: REMOTE_TIMEOUT_SECS,
            max_body_bytes: ASSET_BYTE_CAP,
            redirect_limit: REDIRECT_LIMIT,
            allow_scheme_change: false,
        },
    };

    let response = match transport.send(&request) {
        Ok(response) => response,
        Err(TransportError::Io { detail }) if detail == body_cap_detail(ASSET_BYTE_CAP) => {
            return Err(ReadmeAssetState::TooLarge);
        }
        Err(_) => return Err(ReadmeAssetState::Unreachable),
    };
    if response.status != 200 {
        // A redirect included: this build follows none, so a `302` is an answer that carries no
        // image rather than a hop to take.
        return Err(ReadmeAssetState::Unreachable);
    }
    // The transport bounds the read while streaming; this bounds what a transport that did not
    // hands back. Both, because only one of them is exercised by a fake.
    if response.body.len() > ASSET_BYTE_CAP {
        return Err(ReadmeAssetState::TooLarge);
    }
    let media = sniff_media_type(&response.body).ok_or(ReadmeAssetState::NotAnImage)?;
    Ok((response.body, media))
}
