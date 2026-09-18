#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! §29.3's marker pass, and §28.1's normalisation as J7 applies it.
//!
//! **J7 does not parse.** It does not know which language it is reading, so a marker inside a
//! string literal is a finding; a lexer per language is a cost with no measured benefit.

use codotheca_core::debt::identity::{normalise_salient, SALIENT_CAP_BYTES};
use codotheca_core::jobs::markers::{scan_blob, Marker, MARKERS};

/// Both directions in one test, because asserting only the first passes against a rule that also
/// requires a **trailing** boundary — and that rule would reject `TODOs`, which is a real marker.
/// The leading boundary is what rejects `NOTODO`.
#[test]
fn a_leading_boundary_rejects_notodo_and_a_trailing_one_would_reject_todos() {
    let found = scan_blob(b"// NOTODO nothing here\n// TODOs: two of them\n");
    let salients: Vec<&str> = found
        .iter()
        .map(|o| o.salient_text_capped.as_str())
        .collect();
    eprintln!("found {} occurrences: {salients:?}", found.len());
    assert_eq!(found.len(), 1, "the boundary rule admitted the wrong set");
    assert_eq!(found[0].salient_text_capped, "TODOs: two of them");
    assert_eq!(found[0].marker, Marker::Todo);

    // The blob may also begin at the marker, with no byte before it at all.
    let at_start = scan_blob(b"TODO: first byte\n");
    assert_eq!(at_start.len(), 1);
    // An underscore is a word byte, so it is not a boundary either.
    assert!(scan_blob(b"x_TODO here\n").is_empty());
}

/// The cap is applied **at the largest UTF-8 character boundary at or below 200 bytes**: a byte
/// index that splits a code point panics in Rust, and the cap is part of the identity, so it is
/// applied before the hash and not after.
#[test]
fn the_cap_never_splits_a_code_point() {
    // A three-byte character straddling byte 200.
    let mut raw = b"TODO ".to_vec();
    while raw.len() < SALIENT_CAP_BYTES - 1 {
        raw.push(b'x');
    }
    raw.extend_from_slice("€".as_bytes());
    raw.extend_from_slice(b" and more");
    assert!(raw.len() > SALIENT_CAP_BYTES);

    let capped = normalise_salient(&raw);
    eprintln!(
        "capped {} raw bytes to {} bytes at a cap of {SALIENT_CAP_BYTES}",
        raw.len(),
        capped.len()
    );
    assert!(capped.len() <= SALIENT_CAP_BYTES);
    assert!(
        capped.len() >= SALIENT_CAP_BYTES - 3,
        "the cap fell further back than one character"
    );
    assert!(!capped.ends_with('\u{fffd}'), "a code point was split");
}

/// A formatter reflowing a wrapped comment must not change an item's identity, so the
/// whitespace collapse happens **before** the hash. Two differently-wrapped copies of one
/// comment hash identically.
#[test]
fn whitespace_runs_collapse_before_the_hash() {
    let one = scan_blob(b"// TODO:   tidy   this   up\n");
    let two = scan_blob(b"//\tTODO:\ttidy \t this\tup\t\n");
    eprintln!(
        "{:?} against {:?}",
        one[0].salient_text_capped, two[0].salient_text_capped
    );
    assert_eq!(one[0].salient_sha256, two[0].salient_sha256);
    assert_eq!(one[0].salient_text_capped, "TODO: tidy this up");
}

/// `ordinal_in_blob` is 0-based and ascends on `(line, column)` **within this blob**. §28.1's
/// per-project ordinal is a different number over a different set.
#[test]
fn ordinals_ascend_on_line_then_column() {
    let found = scan_blob(b"HACK a; FIXME b\n\n// TODO c\n");
    let seen: Vec<(u32, u32, u32)> = found
        .iter()
        .map(|o| (o.ordinal_in_blob, o.line, o.column))
        .collect();
    eprintln!("ordinal, line, column: {seen:?}");
    assert_eq!(seen, vec![(0, 1, 1), (1, 1, 9), (2, 3, 4)]);
    assert_eq!(found[0].marker, Marker::Hack);
    assert_eq!(found[1].marker, Marker::Fixme);
    assert_eq!(found[2].marker, Marker::Todo);
}

/// The marker set is `concept.md`'s three and only those, matched case-sensitively and upper
/// case. J7 does not parse, so a marker inside a string literal is a finding.
#[test]
fn three_markers_case_sensitive_and_no_language_is_parsed() {
    eprintln!("MARKERS holds {}: {MARKERS:?}", MARKERS.len());
    assert!(!MARKERS.is_empty());
    assert!(scan_blob(b"// todo lower case\n").is_empty());
    assert!(scan_blob(b"// XXX not a marker\n").is_empty());
    let in_a_literal = scan_blob(b"let s = \"TODO inside a string\";\n");
    assert_eq!(in_a_literal.len(), 1);
    assert_eq!(
        in_a_literal[0].salient_text_capped,
        "TODO inside a string\";"
    );
}

/// The salient is the bytes from the marker's first byte to the **end of its line** — that is
/// the whole of the producer's rule, and it is parse-free.
#[test]
fn the_salient_runs_to_the_end_of_the_line_and_no_further() {
    let found = scan_blob(b"before TODO: the tail\nthe next line\n");
    assert_eq!(found[0].salient_text_capped, "TODO: the tail");
    // A carriage return is trailing whitespace and is trimmed, not carried into the hash.
    let crlf = scan_blob(b"before TODO: the tail\r\nnext\r\n");
    assert_eq!(crlf[0].salient_text_capped, "TODO: the tail");
    assert_eq!(found[0].salient_sha256, crlf[0].salient_sha256);
}
