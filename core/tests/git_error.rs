#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! §3.5's classification rule: a read that failed is never a repository that is gone.

use codotheca_core::git::{classify, classify_spawn, BusyMarker, GitError};

#[test]
fn dubious_ownership_is_untrusted() {
    let e = classify(
        128,
        b"fatal: detected dubious ownership in repository at '/x/y'\n",
    );
    assert!(matches!(e, GitError::Untrusted { .. }), "{e:?}");
    assert_eq!(e.protocol_code(), Some("UNTRUSTED_REPO"));
    assert!(!e.implies_absent());
}

#[test]
fn index_lock_is_a_deferral_not_an_error() {
    let e = classify(
        128,
        b"fatal: Unable to create '/x/.git/index.lock': File exists.\n\nAnother git process seems to be running in this repository.\n",
    );
    assert_eq!(
        e,
        GitError::Busy {
            marker: BusyMarker::IndexLock
        }
    );
    assert!(e.is_deferral());
    assert_eq!(
        e.protocol_code(),
        None,
        "a deferral is not a surfaced project error"
    );
    assert!(!e.implies_absent());
}

// §3.5: on Windows, sharing violations, access denial and cloud-placeholder failures produce
// stale/unknown, NEVER `missing`. These are the exact byte strings git for Windows emits.
#[test]
fn windows_sharing_violation_is_stale_never_absent() {
    let e = classify(
        128,
        b"error: unable to read 'x': The process cannot access the file because it is being used by another process.\r\n",
    );
    assert!(matches!(e, GitError::Stale { .. }), "{e:?}");
    assert!(!e.implies_absent());
    assert_eq!(e.protocol_code(), Some("REPO_UNREADABLE"));
}

#[test]
fn windows_access_denied_is_stale_never_absent() {
    let e = classify(128, b"error: open(\"x\"): Access is denied.\r\n");
    assert!(matches!(e, GitError::Stale { .. }), "{e:?}");
    assert!(!e.implies_absent());
}

#[test]
fn cloud_placeholder_failure_is_stale_never_absent() {
    for line in [
        &b"error: unable to read 'x': The cloud file provider is not running.\r\n"[..],
        &b"error: unable to read 'x': The cloud operation was not completed before the time-out period expired.\r\n"[..],
        &b"error: unable to read 'x': The file is not available on this computer.\r\n"[..],
    ] {
        let e = classify(128, line);
        assert!(matches!(e, GitError::Stale { .. }), "{line:?} -> {e:?}");
        assert!(!e.implies_absent(), "{line:?}");
    }
}

#[test]
fn only_a_genuinely_absent_path_implies_absence() {
    let gone = classify(
        128,
        b"fatal: cannot change to '/x/y': No such file or directory\n",
    );
    assert!(matches!(gone, GitError::PathGone { .. }), "{gone:?}");
    assert!(gone.implies_absent());
    assert_eq!(gone.protocol_code(), Some("PATH_GONE"));

    for other in [
        GitError::Stale {
            detail: String::new(),
        },
        GitError::PermissionDenied {
            detail: String::new(),
        },
        GitError::StoreOffline {
            detail: String::new(),
        },
        GitError::Unreadable {
            detail: String::new(),
        },
        GitError::Busy {
            marker: BusyMarker::Rebase,
        },
        GitError::TornRead,
        GitError::Budget { after_ms: 1 },
        GitError::Cancelled,
    ] {
        assert!(!other.implies_absent(), "{other:?}");
    }
}

#[test]
fn a_dead_network_share_is_store_offline() {
    let e = classify(128, b"fatal: unable to access '//srv/share/.git': The specified network name is no longer available.\r\n");
    assert!(matches!(e, GitError::StoreOffline { .. }), "{e:?}");
    assert_eq!(e.protocol_code(), Some("STORE_OFFLINE"));
    assert!(!e.implies_absent());
}

#[test]
fn a_missing_binary_is_git_missing() {
    let io = std::io::Error::from(std::io::ErrorKind::NotFound);
    assert_eq!(classify_spawn(&io), GitError::Missing);
    assert_eq!(GitError::Missing.protocol_code(), Some("GIT_MISSING"));
}

#[test]
fn not_a_repository_is_unreadable() {
    let e = classify(
        128,
        b"fatal: not a git repository (or any of the parent directories): .git\n",
    );
    assert!(matches!(e, GitError::Unreadable { .. }), "{e:?}");
    assert_eq!(e.protocol_code(), Some("REPO_UNREADABLE"));
}

// Nothing here may invent an error code: every mapping must be a member of the closed enum in
// the codegen source of truth (§2.4).
#[test]
fn every_protocol_code_is_in_the_closed_enum() {
    let schema = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("protocol")
            .join("schema")
            .join("protocol.json"),
    )
    .unwrap();
    let doc: serde_json::Value = serde_json::from_str(&schema).unwrap();
    let allowed: Vec<String> = doc["errors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_owned())
        .collect();

    let all = [
        GitError::Missing,
        GitError::TooOld {
            found: String::new(),
        },
        GitError::Untrusted {
            path: String::new(),
        },
        GitError::PermissionDenied {
            detail: String::new(),
        },
        GitError::PathGone {
            detail: String::new(),
        },
        GitError::StoreOffline {
            detail: String::new(),
        },
        GitError::Unreadable {
            detail: String::new(),
        },
        GitError::Stale {
            detail: String::new(),
        },
        GitError::Busy {
            marker: BusyMarker::IndexLock,
        },
        GitError::TornRead,
        GitError::Budget { after_ms: 0 },
        GitError::Cancelled,
        GitError::Internal {
            detail: String::new(),
        },
    ];
    for e in all {
        if let Some(code) = e.protocol_code() {
            assert!(
                allowed.iter().any(|a| a == code),
                "{code} is not in protocol.json"
            );
        }
    }
}
