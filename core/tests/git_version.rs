#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! §3.1's version floor, and the parser that decides whether a `git` on `PATH` meets it.

use codotheca_core::git::{meets_floor, parse_version, GIT_FLOOR};

#[test]
fn parses_the_plain_form() {
    let v = parse_version(b"git version 2.43.0\n").unwrap();
    assert_eq!((v.major, v.minor, v.patch), (2, 43, 0));
    assert_eq!(v.raw, "git version 2.43.0");
}

#[test]
fn parses_the_windows_and_vendor_forms() {
    let w = parse_version(b"git version 2.45.1.windows.1\n").unwrap();
    assert_eq!((w.major, w.minor, w.patch), (2, 45, 1));
    let a = parse_version(b"git version 2.39.3 (Apple Git-146)\n").unwrap();
    assert_eq!((a.major, a.minor, a.patch), (2, 39, 3));
}

#[test]
fn rejects_output_that_carries_no_version() {
    assert!(parse_version(b"").is_none());
    assert!(parse_version(b"command not found\n").is_none());
}

#[test]
fn the_floor_is_two_twentytwo() {
    assert_eq!(GIT_FLOOR, (2, 22));
    assert!(!meets_floor(&parse_version(b"git version 2.21.4").unwrap()));
    assert!(meets_floor(&parse_version(b"git version 2.22.0").unwrap()));
    assert!(meets_floor(&parse_version(b"git version 3.0.0").unwrap()));
    assert!(!meets_floor(&parse_version(b"git version 1.9.5").unwrap()));
}
