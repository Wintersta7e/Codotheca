//! §24.3a and §24.3d: where a clone lands, and the refusals that are replies rather than failures.
//!
//! **The renderer names a destination by `RootId` and nothing else.** The subpath is the project's
//! stored `seed_basename` — write-once, the directory basename recorded at first index, and the
//! only art-seed input (§7.4) — so the basename the scanner reads back after the clone is
//! identical to the one the blueprint was seeded on.
//!
//! **The core refuses; it never transforms.** A `-2` suffix is never generated. A clone landing in
//! `myapp-2` for remote `myapp` is the exact failure §7.4 records: a transformed basename is not
//! `seed_basename`, so the art re-rolls mid-scan for a project the user did not touch.

use crate::protocol::InstallRefusal;

/// Windows reserved device names, which are refused **whatever their case and whatever extension
/// follows** — `aux`, `AUX`, `Aux.txt` and `aux.tar.gz` all name the same device.
///
/// These are refused on **every** target, not only on Windows. A library is a portable artefact:
/// a repository cloned into `com1/` on Linux is one that cannot be checked out, scanned or removed
/// on Windows, and the failure would arrive on a different machine from the one that caused it.
const RESERVED_DEVICE_NAMES: &[&str] = &[
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
    "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

/// §24.3d's whole list: is this a legal single path segment on both targets?
///
/// Refuses `.`, `..`, anything containing a path separator or one of `: * ? " < > |`, any control
/// character, a Windows reserved device name, and — beyond the spec's list, because they are the
/// same class of hazard — the empty string, a leading `-`, and a trailing space or dot.
///
/// **A trailing space or dot is silently stripped by the Windows filesystem**, so `widget.` and
/// `widget ` both become `widget`. Admitting one would mean the directory created is not the
/// directory named, which breaks the `seed_basename` identity this module exists to preserve just
/// as surely as a `-2` suffix does.
#[must_use]
pub fn is_safe_path_segment(segment: &str) -> bool {
    if segment.is_empty() || segment == "." || segment == ".." {
        return false;
    }
    // A segment beginning with `-` is a path that argv reads as an option.
    if segment.starts_with('-') {
        return false;
    }
    if segment.ends_with(' ') || segment.ends_with('.') {
        return false;
    }
    if segment.chars().any(|c| {
        c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
    }) {
        return false;
    }
    // `aux`, `AUX`, `aux.txt` and `aux.tar.gz` all name the device: the stem is what matters.
    let stem = segment.split('.').next().unwrap_or(segment);
    !RESERVED_DEVICE_NAMES
        .iter()
        .any(|name| stem.eq_ignore_ascii_case(name))
}

/// The refusal for a name that cannot be a directory.
#[must_use]
pub fn refuse_unsafe_name(segment: &str) -> Option<InstallRefusal> {
    (!is_safe_path_segment(segment)).then_some(InstallRefusal::UnsafeName)
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use super::is_safe_path_segment;

    #[test]
    fn an_ordinary_basename_is_safe() {
        for name in [
            "widget",
            "my-tool",
            "a.b.c",
            "Widget2",
            "_private",
            "ünïcode",
        ] {
            assert!(is_safe_path_segment(name), "{name} must be a legal segment");
        }
    }

    #[test]
    fn a_relative_component_is_refused() {
        for name in ["", ".", ".."] {
            assert!(!is_safe_path_segment(name), "{name:?} must be refused");
        }
    }

    #[test]
    fn a_separator_or_a_windows_illegal_character_is_refused() {
        for name in [
            "a/b", "a\\b", "a:b", "a*b", "a?b", "a\"b", "a<b", "a>b", "a|b", "a\tb", "a\0b",
        ] {
            assert!(!is_safe_path_segment(name), "{name:?} must be refused");
        }
    }

    /// Case-varied, and with an extension, because all of these name the same device.
    #[test]
    fn a_reserved_device_name_is_refused_in_every_spelling() {
        for name in [
            "aux",
            "AUX",
            "Aux",
            "con",
            "CON",
            "prn",
            "nul",
            "NUL",
            "com1",
            "COM9",
            "lpt1",
            "LPT9",
            "aux.txt",
            "COM1.tar.gz",
            "nul.md",
        ] {
            assert!(
                !is_safe_path_segment(name),
                "{name:?} names a reserved device and must be refused on every target"
            );
        }
    }

    /// `com0` and `lpt0` are **not** reserved, and refusing them would be a transformation of a
    /// legal name — the thing this module exists not to do.
    #[test]
    fn a_name_that_merely_resembles_a_device_is_allowed() {
        for name in ["com0", "lpt0", "com10", "console", "auxiliary", "nullable"] {
            assert!(
                is_safe_path_segment(name),
                "{name:?} is not a reserved device and must be allowed"
            );
        }
    }

    /// The Windows filesystem strips these, so the directory created would not be the directory
    /// named — the same identity break as a `-2` suffix, arriving by a different route.
    #[test]
    fn a_trailing_space_or_dot_is_refused_because_the_filesystem_would_strip_it() {
        for name in ["widget ", "widget.", "widget..", "widget . "] {
            assert!(!is_safe_path_segment(name), "{name:?} must be refused");
        }
    }

    #[test]
    fn a_leading_dash_is_refused_because_argv_reads_it_as_an_option() {
        assert!(!is_safe_path_segment("-rf"));
        assert!(is_safe_path_segment("a-rf"));
    }
}
