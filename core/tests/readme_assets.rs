#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! §25.5 — the bytes behind a README's images, and every reason this build refuses to fetch some.
//!
//! **The containment assertions come first, and the symlink cases first among them.** §25.5 cites
//! the containment check "already proven at `app/src/main/art/artProtocol.ts:91-106`" — which is
//! `path.resolve(file).startsWith(path.resolve(root) + path.sep)`, purely lexical, and cannot see
//! a symlink that leaves the root because nothing in it touches the filesystem. A test suite that
//! only exercised `..` would pass against that weaker predicate.

use std::path::Path;

use codotheca_core::http::{HttpResponse, TransportError};
use codotheca_core::protocol::{ReadmeAsset, ReadmeAssetState};
use codotheca_core::readme::assets::{
    classify_ref, read_readme_assets, resolve_local_asset, sniff_media_type, AssetDeps, AssetRef,
    ASSET_BYTE_CAP, ASSET_COUNT_CAP,
};
use codotheca_core::readme::fetch::{fetch_remote_asset, is_fetchable_host, is_public_address};
use codotheca_core::testing::FakeTransport;

const NOW: i64 = 1_781_179_200;

/// A resolver that answers with one documentation address (TEST-NET-3, RFC 5737), so no test
/// depends on this machine having DNS.
///
/// The `Result` is the `HostResolver` signature's, not this stub's choice: a fn pointer must
/// match the type the production resolver has.
#[allow(clippy::unnecessary_wraps)]
fn public_resolver(_host: &str, _port: u16) -> std::io::Result<Vec<std::net::IpAddr>> {
    Ok(vec!["203.0.113.10".parse().expect("literal")])
}

/// A resolver that answers with a private address, which is the DNS-rebinding shape: the literal
/// is innocent and the answer is not.
#[allow(clippy::unnecessary_wraps)]
fn private_resolver(_host: &str, _port: u16) -> std::io::Result<Vec<std::net::IpAddr>> {
    Ok(vec!["192.168.1.7".parse().expect("literal")])
}

/// A resolver that cannot answer at all. A host that does not resolve is not reachable, and the
/// only honest state for it is the one a refused host gets.
fn failing_resolver(_host: &str, _port: u16) -> std::io::Result<Vec<std::net::IpAddr>> {
    Err(std::io::Error::other("no answer"))
}

fn png(bytes: usize) -> Vec<u8> {
    let mut body = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    body.resize(bytes.max(8), 0x42);
    body
}

fn ok_response(body: Vec<u8>) -> HttpResponse {
    HttpResponse {
        status: 200,
        headers: Vec::new(),
        body,
    }
}

fn deps<'a>(
    root: &Path,
    consent: Option<i64>,
    http: &'a FakeTransport,
    resolve: codotheca_core::readme::fetch::HostResolver,
) -> AssetDeps<'a> {
    AssetDeps {
        work_dir: root.to_path_buf(),
        consent,
        http,
        resolve,
        now: NOW,
    }
}

fn state_of(rows: &[ReadmeAsset], reference: &str) -> ReadmeAssetState {
    rows.iter()
        .find(|row| row.r#ref == reference)
        .unwrap_or_else(|| panic!("no row for {reference}: {rows:?}"))
        .state
}

// --- Task 5: containment, and the two cases a lexical check passes -----------------------

#[test]
fn a_symlink_that_leaves_the_root_is_refused_and_one_that_stays_resolves() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path().join("repo");
    std::fs::create_dir(&root).expect("mkdir");
    std::fs::create_dir(root.join("docs")).expect("mkdir");
    std::fs::write(root.join("docs/logo.png"), png(32)).expect("write");
    let outside = tmp.path().join("secret.png");
    std::fs::write(&outside, png(32)).expect("write");

    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&outside, root.join("escape.png")).expect("symlink out");
        std::os::unix::fs::symlink(root.join("docs/logo.png"), root.join("inside.png"))
            .expect("symlink in");
    }
    #[cfg(windows)]
    {
        // A Windows host without the developer-mode privilege cannot create a symlink at all.
        // Skipping *silently* would be a gate that scanned nothing, so it says so.
        if std::os::windows::fs::symlink_file(&outside, root.join("escape.png")).is_err() {
            eprintln!(
                "readme_assets: this host refuses symlink creation, skipping the escape case"
            );
            return;
        }
        std::os::windows::fs::symlink_file(root.join("docs/logo.png"), root.join("inside.png"))
            .expect("symlink in");
    }

    assert_eq!(
        resolve_local_asset(&root, "escape.png"),
        Err(ReadmeAssetState::NotAnImage),
        "a symlink leaving the root is the case the lexical check passes"
    );
    let inside = resolve_local_asset(&root, "inside.png").expect("a symlink inside the root");
    assert!(inside.starts_with(root.canonicalize().expect("canonical root")));
}

#[test]
fn traversal_and_absolute_paths_are_refused_before_the_disk_is_touched() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path().join("repo");
    std::fs::create_dir(&root).expect("mkdir");
    std::fs::write(tmp.path().join("secret.png"), png(32)).expect("write");

    for reference in ["../secret.png", "docs/../../secret.png", ""] {
        assert_eq!(
            resolve_local_asset(&root, reference),
            Err(ReadmeAssetState::NotAnImage),
            "{reference} must be refused"
        );
    }
    let absolute = tmp.path().join("secret.png");
    assert_eq!(
        resolve_local_asset(&root, &absolute.to_string_lossy()),
        Err(ReadmeAssetState::NotAnImage)
    );
}

#[test]
fn a_directory_is_not_a_file_and_is_refused() {
    let tmp = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir(tmp.path().join("docs")).expect("mkdir");
    assert_eq!(
        resolve_local_asset(tmp.path(), "docs"),
        Err(ReadmeAssetState::NotAnImage)
    );
}

#[test]
fn a_file_inside_the_root_resolves_to_its_canonical_path() {
    let tmp = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir(tmp.path().join("docs")).expect("mkdir");
    std::fs::write(tmp.path().join("docs/logo.png"), png(32)).expect("write");
    let resolved = resolve_local_asset(tmp.path(), "docs/logo.png").expect("inside the root");
    assert!(resolved.ends_with("logo.png"));
    // `./` is the same file, and both resolve to one canonical path — which is what keeps a
    // duplicate reference from being counted twice against the caps.
    let dotted = resolve_local_asset(tmp.path(), "./docs/logo.png").expect("inside the root");
    assert_eq!(resolved, dotted);
}

// --- Task 4: classification, sniffing and the caps ---------------------------------------

#[test]
fn classification_is_the_cores_and_the_arrays_are_only_a_hint() {
    assert!(matches!(
        classify_ref("https://cdn.example.test/badge.svg"),
        AssetRef::Remote(_)
    ));
    assert!(matches!(classify_ref("docs/logo.png"), AssetRef::Local(_)));
    assert!(matches!(classify_ref("./logo.png"), AssetRef::Local(_)));
    for hostile in [
        "http://cdn.example.test/badge.svg",
        "data:image/png;base64,AAAA",
        "javascript:alert(1)",
        "file:///etc/passwd",
    ] {
        assert_eq!(
            classify_ref(hostile),
            AssetRef::Rejected(ReadmeAssetState::Unreachable),
            "{hostile} is never fetched"
        );
    }
}

#[test]
fn the_media_type_is_sniffed_from_the_bytes_and_never_from_the_name() {
    assert_eq!(sniff_media_type(&png(16)), Some("image/png"));
    assert_eq!(
        sniff_media_type(&[0xff, 0xd8, 0xff, 0x00]),
        Some("image/jpeg")
    );
    assert_eq!(sniff_media_type(b"GIF89a...."), Some("image/gif"));
    let mut webp = b"RIFF\0\0\0\0WEBPVP8 ".to_vec();
    webp.push(0);
    assert_eq!(sniff_media_type(&webp), Some("image/webp"));
    assert_eq!(
        sniff_media_type(b"<?xml version=\"1.0\"?>\n<svg xmlns=\"http://www.w3.org/2000/svg\"/>"),
        Some("image/svg+xml")
    );
    assert_eq!(sniff_media_type(b"not an image at all"), None);
    assert_eq!(sniff_media_type(b""), None);
}

#[test]
fn a_text_file_named_png_is_not_an_image_and_a_jpeg_named_png_is_a_jpeg() {
    let tmp = tempfile::tempdir().expect("tempdir");
    std::fs::write(tmp.path().join("lying.png"), b"this is prose").expect("write");
    std::fs::write(tmp.path().join("jpeg.png"), [0xff, 0xd8, 0xff, 0x01]).expect("write");
    let http = FakeTransport::new();
    let deps = deps(tmp.path(), None, &http, public_resolver);

    let rows = read_readme_assets(&deps, &["lying.png".to_owned(), "jpeg.png".to_owned()], &[]);
    assert_eq!(state_of(&rows, "lying.png"), ReadmeAssetState::NotAnImage);
    assert_eq!(state_of(&rows, "jpeg.png"), ReadmeAssetState::Ok);
    let uri = rows
        .iter()
        .find(|r| r.r#ref == "jpeg.png")
        .and_then(|r| r.data_uri.clone())
        .expect("a data uri");
    assert!(
        uri.starts_with("data:image/jpeg;base64,"),
        "the extension said png and the bytes decide: {}",
        &uri[..uri.len().min(40)]
    );
}

#[test]
fn a_file_past_the_per_asset_cap_is_too_large_and_carries_no_data_uri() {
    let tmp = tempfile::tempdir().expect("tempdir");
    std::fs::write(tmp.path().join("big.png"), png(ASSET_BYTE_CAP + 1)).expect("write");
    std::fs::write(tmp.path().join("small.png"), png(ASSET_BYTE_CAP)).expect("write");
    let http = FakeTransport::new();
    let deps = deps(tmp.path(), None, &http, public_resolver);

    let rows = read_readme_assets(&deps, &["big.png".to_owned(), "small.png".to_owned()], &[]);
    assert_eq!(state_of(&rows, "big.png"), ReadmeAssetState::TooLarge);
    assert!(rows
        .iter()
        .find(|r| r.r#ref == "big.png")
        .expect("a row")
        .data_uri
        .is_none());
    assert_eq!(
        state_of(&rows, "small.png"),
        ReadmeAssetState::Ok,
        "exactly the cap is admitted, so the boundary is not off by one"
    );
}

#[test]
fn the_reply_cap_is_on_source_bytes_and_keeps_the_answer_inside_the_frame() {
    let tmp = tempfile::tempdir().expect("tempdir");
    // Nine files of 500 KB. Each is inside the 512 KB per-asset cap, so the only thing that can
    // refuse one is the **reply** cap: eight fit inside 4 MB and the ninth does not.
    //
    // The plan's fixture was three 1.5 MB files "returning two ok and one too_large", which
    // cannot happen — 1.5 MB is past ASSET_BYTE_CAP, so all three are `too_large` for the other
    // reason and the reply cap is never reached.
    let names = [
        "a.png", "b.png", "c.png", "d.png", "e.png", "f.png", "g.png", "h.png", "i.png",
    ];
    for name in names {
        std::fs::write(tmp.path().join(name), png(500_000)).expect("write");
    }
    let http = FakeTransport::new();
    let deps = deps(tmp.path(), None, &http, public_resolver);
    let refs: Vec<String> = names.iter().map(|n| (*n).to_owned()).collect();

    let rows = read_readme_assets(&deps, &refs, &[]);
    let ok = rows
        .iter()
        .filter(|r| r.state == ReadmeAssetState::Ok)
        .count();
    let too_large = rows
        .iter()
        .filter(|r| r.state == ReadmeAssetState::TooLarge)
        .count();
    assert_eq!((ok, too_large), (8, 1));

    // MAX_FRAME_BYTES is 8 MiB (`app/src/main/core/frame.ts:8`); the serialised reply must sit
    // inside it, which is the number the 4 MB source cap exists to produce.
    let serialised = serde_json::to_vec(&rows).expect("serialise");
    assert!(
        serialised.len() < 8 * 1024 * 1024,
        "the reply is {} bytes, past MAX_FRAME_BYTES",
        serialised.len()
    );
}

#[test]
fn the_twenty_fifth_reference_is_dropped_from_the_reply_entirely() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut refs = Vec::new();
    for n in 0..=ASSET_COUNT_CAP {
        let name = format!("img{n}.png");
        std::fs::write(tmp.path().join(&name), png(32)).expect("write");
        refs.push(name);
    }
    let http = FakeTransport::new();
    let deps = deps(tmp.path(), None, &http, public_resolver);

    let rows = read_readme_assets(&deps, &refs, &[]);
    assert_eq!(rows.len(), ASSET_COUNT_CAP);
    assert!(
        !rows.iter().any(|r| r.r#ref == refs[ASSET_COUNT_CAP]),
        "the 25th is absent rather than carrying a state that says something false about it"
    );
}

#[test]
fn a_reference_named_twice_is_read_once_and_answered_once() {
    let tmp = tempfile::tempdir().expect("tempdir");
    std::fs::write(tmp.path().join("logo.png"), png(32)).expect("write");
    let http = FakeTransport::new();
    let deps = deps(tmp.path(), None, &http, public_resolver);

    let rows = read_readme_assets(&deps, &["logo.png".to_owned(), "logo.png".to_owned()], &[]);
    assert_eq!(rows.len(), 1, "one row per distinct reference");
}

#[test]
fn the_wire_key_is_ref_although_the_rust_field_is_a_raw_identifier() {
    // `ref` is a Rust keyword, so the emitter writes `pub r#ref`. R31 says the schema is the one
    // declaration; this asserts the serialised key is still what the schema named.
    let row = ReadmeAsset {
        r#ref: "docs/logo.png".to_owned(),
        state: ReadmeAssetState::Ok,
        data_uri: None,
        fetched_at: None,
    };
    let value = serde_json::to_value(&row).expect("serialise");
    assert_eq!(value["ref"], "docs/logo.png");
    assert_eq!(value["state"], "ok");
}

// --- Task 6: consent, the URL guard and the bounded fetch --------------------------------

#[test]
fn with_no_consent_every_remote_reference_is_blocked_and_the_transport_is_never_called() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let http = FakeTransport::new();
    http.push(ok_response(png(64)));
    let deps = deps(tmp.path(), None, &http, public_resolver);

    let rows = read_readme_assets(
        &deps,
        &[],
        &[
            "https://cdn.example.test/one.svg".to_owned(),
            "https://cdn.example.test/two.png".to_owned(),
        ],
    );
    assert_eq!(
        http.request_count(),
        0,
        "the consent gate is before the socket, not after the answer"
    );
    for row in &rows {
        assert_eq!(row.state, ReadmeAssetState::Blocked);
        assert!(
            row.fetched_at.is_none(),
            "a blocked asset observed nothing, so it carries no clock"
        );
        assert!(row.data_uri.is_none());
    }
}

#[test]
fn with_consent_an_https_badge_comes_back_as_a_data_uri() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let http = FakeTransport::new();
    http.push(ok_response(
        b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>".to_vec(),
    ));
    let deps = deps(tmp.path(), Some(NOW - 60), &http, public_resolver);

    let rows = read_readme_assets(
        &deps,
        &[],
        &["https://cdn.example.test/badge.svg".to_owned()],
    );
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].state, ReadmeAssetState::Ok);
    assert_eq!(rows[0].fetched_at, Some(NOW));
    assert!(rows[0]
        .data_uri
        .as_deref()
        .expect("a data uri")
        .starts_with("data:image/svg+xml;base64,"));

    // The request carries no credential of any kind, asserted over what was **sent**.
    let sent = http.requests();
    assert_eq!(sent.len(), 1);
    for (name, _) in &sent[0].headers {
        let lower = name.to_ascii_lowercase();
        assert!(
            !["authorization", "cookie", "referer"].contains(&lower.as_str()),
            "{name} reached a stranger's host"
        );
    }
    assert!(sent[0].headers.is_empty(), "no header at all is the policy");
    // And the bounds this fetch asked for, which is where the redirect and scheme rules live.
    assert_eq!(sent[0].limits.max_body_bytes, ASSET_BYTE_CAP);
    assert_eq!(sent[0].limits.total_secs, 5);
    assert_eq!(sent[0].limits.redirect_limit, 0);
    assert!(!sent[0].limits.allow_scheme_change);
}

#[test]
fn a_private_or_loopback_host_is_refused_before_a_socket_is_opened() {
    let http = FakeTransport::new();
    for refused in [
        "https://127.0.0.1/badge.svg",
        "https://localhost/badge.svg",
        "https://[::1]/badge.svg",
        "https://10.0.0.5/badge.svg",
        "https://192.168.1.1/badge.svg",
        "https://169.254.169.254/latest/meta-data",
        "https://printer.local/badge.svg",
        "http://cdn.example.test/badge.svg",
    ] {
        let url = reqwest::Url::parse(refused).expect("parse");
        assert!(!is_fetchable_host(&url), "{refused} must be refused");
        assert_eq!(
            fetch_remote_asset(&http, &url, public_resolver),
            Err(ReadmeAssetState::Unreachable),
            "{refused}"
        );
    }
    assert_eq!(http.request_count(), 0);

    let allowed = reqwest::Url::parse("https://cdn.example.test/badge.svg").expect("parse");
    assert!(is_fetchable_host(&allowed));
}

#[test]
fn a_public_name_resolving_to_a_private_address_is_refused() {
    let http = FakeTransport::new();
    http.push(ok_response(png(64)));
    let url = reqwest::Url::parse("https://rebind.example.test/badge.png").expect("parse");

    assert_eq!(
        fetch_remote_asset(&http, &url, private_resolver),
        Err(ReadmeAssetState::Unreachable),
        "the literal is innocent and the answer is not"
    );
    assert_eq!(http.request_count(), 0);
    assert!(!is_public_address("192.168.1.7".parse().expect("literal")));
    assert!(is_public_address("203.0.113.10".parse().expect("literal")));
}

#[test]
fn a_host_that_does_not_resolve_is_unreachable_and_opens_no_socket() {
    let http = FakeTransport::new();
    let url = reqwest::Url::parse("https://cdn.example.test/badge.png").expect("parse");
    assert_eq!(
        fetch_remote_asset(&http, &url, failing_resolver),
        Err(ReadmeAssetState::Unreachable)
    );
    assert_eq!(http.request_count(), 0);
}

#[test]
fn a_redirect_is_an_answer_that_carries_no_image() {
    let http = FakeTransport::new();
    // Neither hop is taken: with `redirect_limit: 0` the transport refuses to follow, and a
    // status that is not 200 carries no image whatever it points at.
    for location in [
        "http://cdn.example.test/badge.svg",
        "https://127.0.0.1/x.svg",
    ] {
        http.push(HttpResponse {
            status: 302,
            headers: vec![("location".to_owned(), location.to_owned())],
            body: Vec::new(),
        });
        let url = reqwest::Url::parse("https://cdn.example.test/badge.svg").expect("parse");
        assert_eq!(
            fetch_remote_asset(&http, &url, public_resolver),
            Err(ReadmeAssetState::Unreachable),
            "a 302 to {location}"
        );
    }
}

#[test]
fn a_body_past_the_cap_is_too_large_and_a_stall_is_unreachable() {
    let http = FakeTransport::new();
    let url = reqwest::Url::parse("https://cdn.example.test/badge.png").expect("parse");

    // The shape a real transport produces when it stops reading at the cap. The detail comes
    // from `body_cap_detail`, which is the one owner of those words.
    http.push_err(TransportError::Io {
        detail: codotheca_core::http::body_cap_detail(ASSET_BYTE_CAP),
    });
    assert_eq!(
        fetch_remote_asset(&http, &url, public_resolver),
        Err(ReadmeAssetState::TooLarge)
    );

    // And the shape a fake produces, which returns the whole body: the same answer.
    http.push(ok_response(png(600 * 1024)));
    assert_eq!(
        fetch_remote_asset(&http, &url, public_resolver),
        Err(ReadmeAssetState::TooLarge)
    );

    http.push_err(TransportError::Timeout);
    assert_eq!(
        fetch_remote_asset(&http, &url, public_resolver),
        Err(ReadmeAssetState::Unreachable)
    );
}

#[test]
fn bytes_that_are_not_an_image_are_not_an_image_however_the_host_labels_them() {
    let http = FakeTransport::new();
    http.push(HttpResponse {
        status: 200,
        headers: vec![("content-type".to_owned(), "image/png".to_owned())],
        body: b"<!doctype html><title>login</title>".to_vec(),
    });
    let url = reqwest::Url::parse("https://cdn.example.test/badge.png").expect("parse");
    assert_eq!(
        fetch_remote_asset(&http, &url, public_resolver),
        Err(ReadmeAssetState::NotAnImage),
        "Content-Type is a claim by the host; the bytes are the fact"
    );
}
