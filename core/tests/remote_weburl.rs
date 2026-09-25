#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! §25.2 — the whole of the product's URL construction, asserted on the exact strings.
//!
//! A `contains` assertion would pass a URL carrying a stray suffix, and the string this module
//! produces is handed to `shell.openExternal`.

use codotheca_core::protocol::RemoteLinkKind;
use codotheca_core::remote::weburl::{is_allowlisted_host, web_url, ALLOWLIST_BASE};

const fn no_enterprise() -> Vec<String> {
    Vec::new()
}

#[test]
fn the_five_kinds_render_their_exact_urls() {
    let key = "github.com/acme/widget";
    let cases = [
        (RemoteLinkKind::Repository, "https://github.com/acme/widget"),
        (
            RemoteLinkKind::Issues,
            "https://github.com/acme/widget/issues",
        ),
        (
            RemoteLinkKind::Pulls,
            "https://github.com/acme/widget/pulls",
        ),
        (
            RemoteLinkKind::Actions,
            "https://github.com/acme/widget/actions",
        ),
        (
            RemoteLinkKind::Releases,
            "https://github.com/acme/widget/releases",
        ),
    ];
    for (kind, expected) in cases {
        assert_eq!(
            web_url(key, kind, &no_enterprise()).as_deref(),
            Some(expected),
            "{kind:?} did not render its exact URL"
        );
    }
}

#[test]
fn an_unlisted_host_produces_no_url() {
    assert_eq!(
        web_url(
            "forge.example.invalid/acme/widget",
            RemoteLinkKind::Repository,
            &no_enterprise()
        ),
        None
    );
}

/// The Enterprise host of a connected account joins the allowlist, and only that host.
#[test]
fn a_connected_enterprise_host_is_allowlisted_and_a_stranger_is_not() {
    let hosts = vec!["forge.example.invalid".to_owned()];
    assert_eq!(
        web_url(
            "forge.example.invalid/acme/widget",
            RemoteLinkKind::Issues,
            &hosts
        )
        .as_deref(),
        Some("https://forge.example.invalid/acme/widget/issues")
    );
    assert_eq!(
        web_url(
            "other.example.invalid/acme/widget",
            RemoteLinkKind::Issues,
            &hosts
        ),
        None
    );
    assert!(is_allowlisted_host("github.com", &hosts));
    assert!(is_allowlisted_host("forge.example.invalid", &hosts));
    assert!(!is_allowlisted_host("other.example.invalid", &hosts));
}

/// §1.1's key is `<host>/<owner>/<name>`. Anything else is not a key this build can address, and
/// guessing a URL from two segments or from four is a URL the product invented.
#[test]
fn a_key_that_is_not_three_segments_produces_no_url() {
    for key in [
        "github.com/acme",
        "github.com",
        "github.com/group/sub/widget",
        "",
        "/acme/widget",
    ] {
        assert_eq!(
            web_url(key, RemoteLinkKind::Repository, &no_enterprise()),
            None,
            "{key:?} produced a URL"
        );
    }
}

/// A9: the URL derives from **this project's** key only. A fork parent on an allowlisted host
/// must not make an off-allowlist project linkable — `fork_parent_remote_key` is a rendered
/// string and reaches no URL construction (AC-P2-25-26).
#[test]
fn ac_p2_25_26_url_a_fork_parent_does_not_make_its_child_linkable() {
    let own = "forge.example.invalid/acme/widget";
    let fork_parent = "github.com/upstream/widget";
    assert_eq!(
        web_url(own, RemoteLinkKind::Repository, &no_enterprise()),
        None,
        "the project's own host is not allowlisted, so it has no URL"
    );
    // Stated as an assertion rather than as a comment: the parent key is a valid input to the
    // same function, and what makes this honest is that nothing passes it one.
    assert!(web_url(fork_parent, RemoteLinkKind::Repository, &no_enterprise()).is_some());
}

#[test]
fn the_allowlist_base_is_exactly_the_canonical_forge_host() {
    assert_eq!(ALLOWLIST_BASE, ["github.com"]);
}
